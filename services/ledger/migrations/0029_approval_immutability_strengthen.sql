-- =====================================================================
-- 0029: Strengthen approval_requests immutability trigger
--       (Codex round-4 P2 — defense-in-depth gaps)
-- =====================================================================
--
-- The original trigger from 0026 only froze identity + payload columns
-- (tenant_id, decision_id, audit_decision_event_id, requested_effect,
-- decision_context, created_at) and forbade backwards transitions out
-- of terminal states. Codex round-4 review caught two leaks:
--
--   (1) ttl_expires_at and approver_policy were NOT frozen. A direct
--       UPDATE could shorten/lengthen TTL or rewrite approver_policy
--       *after* creation, bypassing the contract author's stated
--       intent and (for ttl_expires_at) the S14 invariant that
--       "approver action MUST happen before this wallclock".
--
--   (2) For terminal rows the trigger only rejected changes to
--       `state` itself. A direct UPDATE that kept state='approved'
--       could still rewrite resolved_at, resolved_by_subject,
--       resolved_by_issuer, or resolution_reason — i.e. silently
--       relabel who approved what and why.
--
-- The S14 SP `resolve_approval_request` writes the resolution
-- metadata as part of the pending → terminal transition, so the
-- trigger admits those changes when OLD.state='pending'. Once the
-- row is terminal, every column is frozen.
--
-- This migration is idempotent: it CREATE OR REPLACEs the function
-- and leaves the trigger binding untouched.

CREATE OR REPLACE FUNCTION approval_requests_block_immutable_updates()
    RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    -- (a) Always-frozen columns. Set at creation; never change for
    -- the lifetime of the row, regardless of state.
    IF NEW.tenant_id              IS DISTINCT FROM OLD.tenant_id
        OR NEW.decision_id        IS DISTINCT FROM OLD.decision_id
        OR NEW.audit_decision_event_id IS DISTINCT FROM OLD.audit_decision_event_id
        OR NEW.requested_effect   IS DISTINCT FROM OLD.requested_effect
        OR NEW.decision_context   IS DISTINCT FROM OLD.decision_context
        OR NEW.created_at         IS DISTINCT FROM OLD.created_at
        -- Codex round-4 P2 (1): freeze TTL + approver_policy.
        OR NEW.ttl_expires_at     IS DISTINCT FROM OLD.ttl_expires_at
        OR NEW.approver_policy    IS DISTINCT FROM OLD.approver_policy
    THEN
        RAISE EXCEPTION
            'approval_requests row %: immutable column changed (S14 invariant)',
            OLD.approval_id
            USING ERRCODE = '23514';   -- check_violation
    END IF;

    -- (b) State-machine guard. Once a row leaves 'pending' it is
    -- terminal; the only legal future is "no further updates".
    --
    -- Codex round-4 P2 (2): when OLD.state is terminal, freeze the
    -- resolution metadata as well. The original trigger only checked
    -- NEW.state <> OLD.state, which permitted same-state UPDATEs that
    -- rewrote resolved_by_subject / resolved_by_issuer / resolved_at
    -- / resolution_reason — silently relabelling who acted.
    IF OLD.state <> 'pending' THEN
        IF NEW.state IS DISTINCT FROM OLD.state THEN
            RAISE EXCEPTION
                'approval_requests row %: terminal state % cannot transition to %',
                OLD.approval_id, OLD.state, NEW.state
                USING ERRCODE = '23514';
        END IF;
        IF NEW.resolved_at         IS DISTINCT FROM OLD.resolved_at
            OR NEW.resolved_by_subject IS DISTINCT FROM OLD.resolved_by_subject
            OR NEW.resolved_by_issuer  IS DISTINCT FROM OLD.resolved_by_issuer
            OR NEW.resolution_reason   IS DISTINCT FROM OLD.resolution_reason
        THEN
            RAISE EXCEPTION
                'approval_requests row %: terminal-row resolution metadata is frozen',
                OLD.approval_id
                USING ERRCODE = '23514';
        END IF;
    END IF;

    RETURN NEW;
END;
$$;

COMMENT ON FUNCTION approval_requests_block_immutable_updates IS
    'S14 + Codex round-4: rejects UPDATEs that would mutate any frozen column. Always-frozen: identity + payload (tenant_id, decision_id, audit_decision_event_id, requested_effect, decision_context, created_at, ttl_expires_at, approver_policy). Once-frozen-on-terminal: state, resolved_at, resolved_by_subject, resolved_by_issuer, resolution_reason. The single legal write to resolution metadata is the pending → terminal transition driven by resolve_approval_request.';
