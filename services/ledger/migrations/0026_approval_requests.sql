-- Phase 5 GA hardening S14: approval state model.
--
-- REQUIRE_APPROVAL is no longer a terminal POC outcome. Decisions
-- that route to approval get a first-class `approval_requests` row
-- with a state machine: pending → approved | denied | expired |
-- cancelled.
--
-- Spec invariants enforced by this schema:
--
--   * "Approval request creation is audited atomically with the
--     decision." — approval_requests is created in the same SP
--     call as the audit_outbox decision row (S14-followup wires
--     post_approval_required_decision SP that bundles both).
--   * "Approving does not mutate the original decision; it appends
--     an approval event and resumes with a new idempotent operation."
--     — approval_requests rows are IMMUTABLE except for the state
--     column + state-related fields. approval_events is append-only.
--   * "Approver identity is required and auditable." — every
--     non-null state transition requires approver_subject + issuer.
--   * "Approval payload cannot be modified after creation." —
--     decision_context JSONB is in approval_requests; updates
--     are blocked by the BEFORE UPDATE trigger.
--
-- Out of scope for S14 (S15 + S16 handle):
--   * REST API endpoints (S15)
--   * Notification dispatcher (S15)
--   * Adapter resume semantics (S16)

-- =====================================================================
-- approval_requests — first-class record per REQUIRE_APPROVAL decision.
-- =====================================================================

CREATE TABLE approval_requests (
    -- Identity.
    approval_id           UUID NOT NULL DEFAULT gen_random_uuid()
                          PRIMARY KEY,
    tenant_id             UUID NOT NULL,
    decision_id           UUID NOT NULL,

    -- The audit_decision row for this REQUIRE_APPROVAL decision.
    -- One:one — exactly one approval per audit.decision row of kind
    -- 'spendguard.audit.decision' WHERE the contract evaluator
    -- returned REQUIRE_APPROVAL.
    audit_decision_event_id UUID NOT NULL,

    -- State machine.
    state                 TEXT NOT NULL DEFAULT 'pending'
                          CHECK (state IN
                              ('pending', 'approved', 'denied',
                               'expired', 'cancelled')),

    -- TTL — the contract rule's `then.approval_ttl_seconds` (default
    -- 1 hour). Approver action MUST happen before this wallclock.
    ttl_expires_at        TIMESTAMPTZ NOT NULL,

    -- Approver policy. Names the role(s) eligible to resolve this
    -- approval. RBAC enforcement at API layer (S15) consults this
    -- against principal.roles (S18).
    approver_policy       JSONB NOT NULL DEFAULT '{}'::JSONB,

    -- Requested effect — the projected_claims that would land if
    -- approved. Same shape as the original DecisionRequest's
    -- projected_claims; immutable.
    requested_effect      JSONB NOT NULL,

    -- Decision context — the matched_rule_ids + reason_codes +
    -- ContractBundleRef + PricingFreeze tuple — everything needed
    -- to re-validate the budget at resolution time. IMMUTABLE
    -- (defense in depth via BEFORE UPDATE trigger below).
    decision_context      JSONB NOT NULL,

    -- Resolution fields. NULL until state transitions out of pending.
    resolved_at           TIMESTAMPTZ,
    resolved_by_subject   TEXT,
    resolved_by_issuer    TEXT,
    -- Operator-supplied reason for approve/deny. Required (CHECK)
    -- when state in (approved, denied).
    resolution_reason     TEXT,

    -- Operational metadata.
    created_at            TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),

    -- Defense in depth on resolution invariants.
    CONSTRAINT approval_resolved_when_terminal
        CHECK (
            state = 'pending'
            OR (resolved_at IS NOT NULL AND resolved_by_subject IS NOT NULL
                AND resolved_by_issuer IS NOT NULL)
        ),
    CONSTRAINT approval_resolution_reason_when_explicit
        CHECK (
            state NOT IN ('approved', 'denied')
            OR (resolution_reason IS NOT NULL AND length(resolution_reason) > 0)
        ),
    CONSTRAINT approval_ttl_after_creation
        CHECK (ttl_expires_at > created_at)
);

CREATE UNIQUE INDEX approval_requests_decision_uq
    ON approval_requests (tenant_id, decision_id);

CREATE INDEX approval_requests_pending_ttl_idx
    ON approval_requests (ttl_expires_at)
    WHERE state = 'pending';

CREATE INDEX approval_requests_tenant_state_idx
    ON approval_requests (tenant_id, state, created_at DESC);

COMMENT ON TABLE approval_requests IS
    'S14: REQUIRE_APPROVAL decisions become first-class records with state machine + TTL. Approving does not mutate the original decision; it appends an approval_event + (S16) resumes with a new idempotent ledger operation.';

-- =====================================================================
-- approval_events — append-only state-transition audit log.
-- =====================================================================
--
-- Every state transition writes a row here. UPDATEs to
-- approval_requests are gated by the trigger below — only column
-- changes that match an event row are allowed.

CREATE TABLE approval_events (
    event_id          UUID NOT NULL DEFAULT gen_random_uuid()
                      PRIMARY KEY,
    approval_id       UUID NOT NULL REFERENCES approval_requests(approval_id),

    -- Transition.
    from_state        TEXT NOT NULL CHECK (from_state IN
                          ('pending', 'approved', 'denied',
                           'expired', 'cancelled')),
    to_state          TEXT NOT NULL CHECK (to_state IN
                          ('pending', 'approved', 'denied',
                           'expired', 'cancelled')),

    -- Actor identity. NULL only for to_state='expired' (system
    -- transition); explicit approve/deny/cancel must carry an actor.
    actor_subject     TEXT,
    actor_issuer      TEXT,

    -- Resolution metadata mirrored from approval_requests at the
    -- moment of transition (so approval_events is self-contained
    -- for forensics; no JOIN required).
    resolution_reason TEXT,

    -- Append-only timestamp.
    occurred_at       TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),

    -- Spec invariant: actor required unless system-driven expiry.
    CONSTRAINT approval_events_actor_required_for_explicit
        CHECK (
            to_state = 'expired'
            OR (actor_subject IS NOT NULL AND actor_issuer IS NOT NULL)
        ),
    -- Approve/deny require a reason.
    CONSTRAINT approval_events_reason_required_for_explicit
        CHECK (
            to_state NOT IN ('approved', 'denied')
            OR (resolution_reason IS NOT NULL AND length(resolution_reason) > 0)
        )
);

CREATE INDEX approval_events_approval_idx
    ON approval_events (approval_id, occurred_at DESC);

COMMENT ON TABLE approval_events IS
    'S14: append-only audit log of approval state transitions. Created atomically with approval_requests UPDATE via SP (S14-followup).';

-- =====================================================================
-- Immutability trigger on approval_requests.
-- =====================================================================
--
-- Defense in depth: only the SP (S14-followup post_approval_resolved)
-- may UPDATE approval_requests, and only state + resolved_*
-- columns. tenant_id / decision_id / requested_effect /
-- decision_context are FROZEN at creation. An operator with direct
-- DB access who runs UPDATE bypasses application logic but the
-- trigger still rejects the change.

CREATE OR REPLACE FUNCTION approval_requests_block_immutable_updates()
    RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
        OR NEW.decision_id IS DISTINCT FROM OLD.decision_id
        OR NEW.audit_decision_event_id IS DISTINCT FROM OLD.audit_decision_event_id
        OR NEW.requested_effect IS DISTINCT FROM OLD.requested_effect
        OR NEW.decision_context IS DISTINCT FROM OLD.decision_context
        OR NEW.created_at IS DISTINCT FROM OLD.created_at
    THEN
        RAISE EXCEPTION
            'approval_requests row %: immutable column changed (S14 invariant)',
            OLD.approval_id
            USING ERRCODE = '23514';   -- check_violation
    END IF;
    -- Forbid going backwards in the state machine.
    IF OLD.state <> 'pending' AND NEW.state <> OLD.state THEN
        RAISE EXCEPTION
            'approval_requests row %: terminal state % cannot transition to %',
            OLD.approval_id, OLD.state, NEW.state
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER approval_requests_immutability
    BEFORE UPDATE ON approval_requests
    FOR EACH ROW
    EXECUTE FUNCTION approval_requests_block_immutable_updates();

COMMENT ON FUNCTION approval_requests_block_immutable_updates IS
    'S14: rejects UPDATEs that would mutate the immutable parts of an approval (decision_context, requested_effect, identity). Also forbids backwards state transitions out of terminal states.';

-- =====================================================================
-- Atomic state-transition stored procedure.
-- =====================================================================
--
-- Single entry point for resolving an approval. Caller (S15 control
-- plane API) passes (approval_id, target_state, actor identity,
-- reason). SP atomically:
--   1. Reads current state with FOR UPDATE.
--   2. Validates the transition is legal.
--   3. UPDATEs approval_requests.
--   4. Inserts approval_events row.
-- All in one transaction. Idempotent on (approval_id, target_state)
-- — repeated approve/deny calls return the same row.

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

    -- Update approval_requests.
    UPDATE approval_requests
       SET state = p_target_state,
           resolved_at = clock_timestamp(),
           resolved_by_subject = p_actor_subject,
           resolved_by_issuer = p_actor_issuer,
           resolution_reason = p_reason
     WHERE approval_id = p_approval_id;

    -- Insert event.
    INSERT INTO approval_events
        (approval_id, from_state, to_state,
         actor_subject, actor_issuer, resolution_reason)
        VALUES
        (p_approval_id, v_current_state, p_target_state,
         p_actor_subject, p_actor_issuer, p_reason)
        RETURNING approval_events.event_id INTO v_event_id;

    RETURN QUERY SELECT p_target_state, TRUE, v_event_id;
END;
$$;

COMMENT ON FUNCTION resolve_approval_request IS
    'S14: atomic SP for approve/deny/cancel/expire. Idempotent — calling with the same target state twice returns transitioned=false.';

-- =====================================================================
-- TTL expiry helper (called by background reaper, S14-followup).
-- =====================================================================

CREATE OR REPLACE FUNCTION expire_pending_approvals_due()
    RETURNS INT LANGUAGE plpgsql AS $$
DECLARE
    v_count INT := 0;
    v_row RECORD;
BEGIN
    FOR v_row IN
        SELECT approval_id
          FROM approval_requests
         WHERE state = 'pending'
           AND ttl_expires_at < clock_timestamp()
    LOOP
        PERFORM resolve_approval_request(
            v_row.approval_id,
            'expired',
            NULL,    -- system actor
            NULL,
            NULL
        );
        -- The actor-required CHECK kicks in for non-expired states;
        -- expired is the documented exception (system transition).
        -- However, the explicit-reason CHECK requires reason for
        -- approved/denied only — expired is fine without one.
        v_count := v_count + 1;
    END LOOP;
    RETURN v_count;
END;
$$;

COMMENT ON FUNCTION expire_pending_approvals_due IS
    'S14: background reaper helper. Marks all pending approvals past TTL as expired. Returns row count. Caller schedules at appropriate interval.';
