-- =====================================================================
-- 0035: Wire approval notification outbox writes (followup #4)
-- =====================================================================
--
-- Phase 5 S15 shipped the `approval_notifications` table (migration
-- 0027) but no SP ever INSERTs into it, so the dispatcher worker
-- (separate followup) has nothing to forward. External notification
-- (Slack / email / webhook) is silently no-op end-to-end.
--
-- This migration:
--   1. Adds `tenant_notification_config` so the SP can resolve the
--      NOT-NULL `target_url` + `signing_key_id` columns the
--      approval_notifications schema requires. The 0027 schema
--      explicitly punted operator config to "not in this slice"; we
--      give it the smallest possible shape now so the outbox path
--      becomes usable.
--   2. CREATE OR REPLACE FUNCTION resolve_approval_request to also
--      INSERT into approval_notifications atomic with the existing
--      approval_events insert. Tenants without a config row simply
--      don't receive notifications — current behavior preserved, no
--      data leak.
--
-- Body of resolve_approval_request copied from 0033 (Codex round 9
-- atomic TTL guard). The new INSERT is between the existing
-- `INSERT INTO approval_events ... RETURNING ... INTO v_event_id;`
-- block and `RETURN QUERY ...`. All round-5 + round-9 invariants
-- preserved.

-- ---------------------------------------------------------------------
-- 1) tenant_notification_config — operator-managed dispatch target.
-- ---------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS tenant_notification_config (
    tenant_id        UUID NOT NULL PRIMARY KEY,
    webhook_url      TEXT NOT NULL CHECK (length(webhook_url) > 0),
    signing_key_id   TEXT NOT NULL CHECK (length(signing_key_id) > 0),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

COMMENT ON TABLE tenant_notification_config IS
    'Followup #4 / S15 wiring: per-tenant webhook + signing-key-id used by resolve_approval_request to populate approval_notifications.target_url + signing_key_id. Tenants without a row here do not receive approval notifications.';

-- ---------------------------------------------------------------------
-- 2) resolve_approval_request — add notification outbox write.
-- ---------------------------------------------------------------------

CREATE OR REPLACE FUNCTION resolve_approval_request(
    p_approval_id UUID,
    p_target_state TEXT,
    p_actor_subject TEXT,
    p_actor_issuer TEXT,
    p_reason TEXT
) RETURNS TABLE (
    final_state TEXT,
    transitioned BOOLEAN,
    event_id UUID
) LANGUAGE plpgsql AS $$
DECLARE
    v_current_state TEXT;
    v_ttl_expires   TIMESTAMPTZ;
    v_event_id UUID;
    v_actor_subject TEXT;
    v_actor_issuer TEXT;
    -- Followup #4: notification outbox.
    v_tenant_id            UUID;
    v_notif_target_url     TEXT;
    v_notif_signing_key_id TEXT;
BEGIN
    -- Codex round-9 P2: read ttl_expires_at alongside state under the
    -- same FOR UPDATE lock so the TTL gate and the state transition
    -- are atomic.
    --
    -- Followup #4: also read tenant_id so the notification INSERT
    -- below sees the same locked snapshot.
    SELECT state, ttl_expires_at, tenant_id
      INTO v_current_state, v_ttl_expires, v_tenant_id
      FROM approval_requests
     WHERE approval_id = p_approval_id
     FOR UPDATE;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'approval_id % not found', p_approval_id
            USING ERRCODE = 'P0002';   -- no_data
    END IF;

    -- Idempotency: caller asks for state X, we're already in state X
    -- → return (X, false, null) so caller knows it didn't transition.
    IF v_current_state = p_target_state THEN
        RETURN QUERY SELECT v_current_state, FALSE, NULL::UUID;
        RETURN;
    END IF;

    -- Only pending → terminal transitions allowed (the trigger also
    -- enforces this; we double-check here so the SP returns a
    -- friendlier error than the trigger's RAISE).
    IF v_current_state <> 'pending' THEN
        RAISE EXCEPTION
            'approval % already in terminal state %, cannot move to %',
            p_approval_id, v_current_state, p_target_state
            USING ERRCODE = '22023';   -- invalid_parameter_value
    END IF;

    IF p_target_state NOT IN ('approved', 'denied', 'cancelled', 'expired') THEN
        RAISE EXCEPTION
            'invalid target state: %', p_target_state
            USING ERRCODE = '22023';
    END IF;

    -- Codex round-9 P2: TTL atomic check for user-driven terminal
    -- states. 'expired' is the sweeper path (system-driven), so it
    -- is admitted regardless of the wallclock — that's the whole
    -- point of the sweeper.
    IF p_target_state IN ('approved', 'denied', 'cancelled')
       AND v_ttl_expires <= clock_timestamp()
    THEN
        RAISE EXCEPTION
            'approval % expired at %, cannot move to % (S14 invariant)',
            p_approval_id, v_ttl_expires, p_target_state
            USING ERRCODE = '22023';
    END IF;

    -- Codex round-5 P1: when the TTL sweeper expires a row it cannot
    -- pass a real actor. Inject a system marker for 'expired' so the
    -- row-level CHECK approval_resolved_when_terminal is satisfied
    -- and forensics still see a recognizable actor.
    IF p_target_state = 'expired' AND p_actor_subject IS NULL THEN
        v_actor_subject := 'system:ttl-sweeper';
    ELSE
        v_actor_subject := p_actor_subject;
    END IF;
    IF p_target_state = 'expired' AND p_actor_issuer IS NULL THEN
        v_actor_issuer := 'system:spendguard';
    ELSE
        v_actor_issuer := p_actor_issuer;
    END IF;

    -- Update approval_requests.
    UPDATE approval_requests
       SET state = p_target_state,
           resolved_at = clock_timestamp(),
           resolved_by_subject = v_actor_subject,
           resolved_by_issuer = v_actor_issuer,
           resolution_reason = p_reason
     WHERE approval_id = p_approval_id;

    -- Insert event. The approval_events CHECK already special-cases
    -- to_state='expired' to allow NULL actor, but for symmetry we
    -- write the same system marker here so audit reads don't have
    -- to special-case the expired path.
    INSERT INTO approval_events
        (approval_id, from_state, to_state,
         actor_subject, actor_issuer, resolution_reason)
        VALUES
        (p_approval_id, v_current_state, p_target_state,
         v_actor_subject, v_actor_issuer, p_reason)
        RETURNING approval_events.event_id INTO v_event_id;

    -- Followup #4 / S15: notification outbox write. Atomic with the
    -- approval_events insert above (same SP transaction). Tenants
    -- without a tenant_notification_config row simply don't receive
    -- notifications — that path stays no-op, matching the schema
    -- author's stated intent that operator config is "not in this
    -- slice".
    --
    -- Includes the 'expired' (sweeper-driven) path so operators get
    -- notified that an approval lapsed without action — symmetric
    -- with approve / deny / cancel.
    SELECT webhook_url, signing_key_id
      INTO v_notif_target_url, v_notif_signing_key_id
      FROM tenant_notification_config
     WHERE tenant_id = v_tenant_id;

    IF v_notif_target_url IS NOT NULL THEN
        INSERT INTO approval_notifications
            (approval_id, transition_event_id, tenant_id, transition_kind,
             target_url, signing_key_id, payload)
        VALUES
            (p_approval_id, v_event_id, v_tenant_id, p_target_state,
             v_notif_target_url, v_notif_signing_key_id,
             jsonb_build_object(
                 'approval_id',         p_approval_id,
                 'tenant_id',           v_tenant_id,
                 'transition',          p_target_state,
                 'from_state',          v_current_state,
                 'reason',              p_reason,
                 'resolved_by_subject', v_actor_subject,
                 'resolved_by_issuer',  v_actor_issuer,
                 'resolved_at',         clock_timestamp()
             ));
    END IF;

    RETURN QUERY SELECT p_target_state, TRUE, v_event_id;
END;
$$;

COMMENT ON FUNCTION resolve_approval_request IS
    'S14 + Codex rounds 5+9 + followup #4: atomic SP for approve/deny/cancel/expire. Idempotent. TTL check inside FOR UPDATE; sweeper-driven ''expired'' admitted regardless of wallclock; ''expired'' with NULL actor injects ''system:ttl-sweeper'' / ''system:spendguard''. Atomic with the state transition: writes one approval_notifications outbox row per transition (looked up via tenant_notification_config) for the dispatcher to forward.';
