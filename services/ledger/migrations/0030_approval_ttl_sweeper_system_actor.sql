-- =====================================================================
-- 0030: Fix TTL sweeper dead-end (Codex round-5 P1)
-- =====================================================================
--
-- The original `expire_pending_approvals_due()` from migration 0026
-- called:
--
--   resolve_approval_request(approval_id, 'expired', NULL, NULL, NULL)
--
-- That UPDATEs approval_requests setting resolved_by_subject and
-- resolved_by_issuer to NULL. But the row-level CHECK constraint
-- `approval_resolved_when_terminal` (defined in 0026) requires
--
--   state = 'pending'
--   OR (resolved_at IS NOT NULL AND resolved_by_subject IS NOT NULL
--       AND resolved_by_issuer IS NOT NULL)
--
-- so every TTL expiry attempt raises check_violation → no row ever
-- transitions to 'expired'. Combined with the round-3 handler-level
-- TTL guard (CONFLICT for state=pending past TTL), this leaves
-- expired approvals permanently stuck in pending with no path out.
--
-- Fix: SP injects an explicit system actor ('system:ttl-sweeper')
-- when called with NULL actor params for state='expired'. Other
-- target states still require a real actor (the SP body returns
-- normally; the row CHECK is what enforces non-NULL for those).
--
-- Forensics improvement: 'system:ttl-sweeper' is recognizable in
-- audit reads, distinct from human approver subjects. The
-- approval_events row gets the same system marker so the chain is
-- self-describing.
--
-- This migration is idempotent: CREATE OR REPLACE replaces the
-- function in place. Existing rows are not touched (operator may
-- need to manually expire stuck pending rows by calling the SP
-- once after deploy).

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
    v_event_id UUID;
    v_actor_subject TEXT;
    v_actor_issuer TEXT;
BEGIN
    SELECT state INTO v_current_state
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

    RETURN QUERY SELECT p_target_state, TRUE, v_event_id;
END;
$$;

COMMENT ON FUNCTION resolve_approval_request IS
    'S14 + Codex round-5: atomic SP for approve/deny/cancel/expire. Idempotent. For target_state=''expired'' with NULL actor params, injects ''system:ttl-sweeper'' / ''system:spendguard'' so the row-level CHECK approval_resolved_when_terminal admits the transition. Other target states still require an explicit actor.';
