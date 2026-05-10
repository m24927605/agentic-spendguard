-- =====================================================================
-- 0033: Move TTL check inside resolve_approval_request transaction
--       (Codex round-9 P2)
-- =====================================================================
--
-- Round 3 added a handler-level TTL preflight in
-- services/control_plane/src/main.rs:resolve_approval. Codex round 9
-- pointed out the obvious race window:
--
--   T0  handler reads ttl_expires_at, sees it's in the future
--   T1  pending → past TTL (clock advances)
--   T2  handler calls resolve_approval_request SP
--   T3  SP only checks state (= 'pending'), happily transitions
--
-- For approver action requests this means an approval can be
-- approved / denied / cancelled after `ttl_expires_at` whenever the
-- preflight-to-SP gap exceeds the remaining TTL. Round-3's
-- migration-0030 (TTL sweeper SP fix) already establishes 'expired'
-- as a terminal state; this migration makes the user-driven SP path
-- symmetric.
--
-- Atomic check: inside resolve_approval_request, after the FOR
-- UPDATE row lock, if v_current_state='pending' and the caller
-- requested a user-driven terminal state ('approved' / 'denied' /
-- 'cancelled') and ttl_expires_at <= clock_timestamp(), reject with
-- the same errcode the handler already maps to CONFLICT. 'expired'
-- itself is admitted (sweeper path).
--
-- Body copied from 0030; the new check is between the existing
-- "current_state <> pending" check and the round-5 system-actor
-- injection block.

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
BEGIN
    -- Codex round-9 P2: read ttl_expires_at alongside state under the
    -- same FOR UPDATE lock so the TTL gate and the state transition
    -- are atomic.
    SELECT state, ttl_expires_at INTO v_current_state, v_ttl_expires
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

    RETURN QUERY SELECT p_target_state, TRUE, v_event_id;
END;
$$;

COMMENT ON FUNCTION resolve_approval_request IS
    'S14 + Codex rounds 5+9: atomic SP for approve/deny/cancel/expire. Idempotent. TTL check moved inside the FOR UPDATE lock so user-driven terminal states cannot bypass ttl_expires_at via a preflight-to-SP race. Sweeper-driven ''expired'' admitted regardless of wallclock. ''expired'' with NULL actor injects ''system:ttl-sweeper'' / ''system:spendguard'' so approval_resolved_when_terminal CHECK admits the transition.';
