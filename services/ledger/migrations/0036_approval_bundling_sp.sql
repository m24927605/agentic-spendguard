-- =====================================================================
-- 0036: Approval bundling — schema + atomic mark SP (followup #9, part 1)
-- =====================================================================
--
-- Phase 5 S14 + S15 + S16 ship the approval state model + REST API +
-- adapter resume proto, but no SP closes the loop:
-- `pending → approved` works, but no idempotent ledger transaction
-- lands when an approver clicks approve. Adapters waiting on
-- ResumeAfterApproval never see real progress.
--
-- This migration is part 1 of 2:
--   * Part 1 (this file): schema columns + immutability trigger
--     update + atomic `mark_approval_bundled` SP
--   * Part 2 (separate PR): Rust handler that reads approval, builds
--     post_ledger_transaction JSONB inputs, calls
--     post_ledger_transaction + mark_approval_bundled in one tx; new
--     gRPC RPC `Ledger.PostBundledApprovalTransaction`; sidecar
--     ResumeAfterApproval handler wired to the new RPC.
--
-- The split mirrors the existing pattern (post_invoice_reconciled,
-- post_release, etc.): Rust builds the rich JSONB inputs, a thin SP
-- does the atomic state assertion + UPDATE.
--
-- Smoke-tested: schema columns, trigger, and SP ready for part 2.

-- ---------------------------------------------------------------------
-- 1) Schema columns: bundled_at + bundled_ledger_transaction_id
-- ---------------------------------------------------------------------
--
-- Both NULL by default; mark_approval_bundled is the SOLE legal write
-- path. Once set, both stay frozen for the lifetime of the row.

ALTER TABLE approval_requests
    ADD COLUMN IF NOT EXISTS bundled_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS bundled_ledger_transaction_id UUID;

CREATE INDEX IF NOT EXISTS approval_requests_bundled_idx
    ON approval_requests (bundled_at)
    WHERE bundled_at IS NOT NULL;

COMMENT ON COLUMN approval_requests.bundled_at IS
    'Followup #9: clock_timestamp() at the moment mark_approval_bundled() ran. NULL while pending OR while terminal-but-not-yet-bundled.';
COMMENT ON COLUMN approval_requests.bundled_ledger_transaction_id IS
    'Followup #9: ledger_transactions.ledger_transaction_id of the deferred operation that landed when this approval was bundled. NULL until bundled.';

-- ---------------------------------------------------------------------
-- 2) Immutability trigger update (extends round-4 / migration 0029).
-- ---------------------------------------------------------------------
--
-- Round-4's trigger froze every column on terminal rows. The bundling
-- columns need a single legal write: NULL → non-NULL exactly once,
-- after the row is already in a terminal state ('approved' is the
-- expected case; defensively allow other terminal states too in case
-- future scenarios bundle on denied / expired / cancelled).
--
-- After the columns are set (non-NULL), they're frozen — same shape
-- as the round-4 "once-frozen-on-terminal" rule.

CREATE OR REPLACE FUNCTION approval_requests_block_immutable_updates()
    RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    -- (a) Always-frozen columns. Set at creation; never change for the
    -- lifetime of the row, regardless of state.
    IF NEW.tenant_id              IS DISTINCT FROM OLD.tenant_id
        OR NEW.decision_id        IS DISTINCT FROM OLD.decision_id
        OR NEW.audit_decision_event_id IS DISTINCT FROM OLD.audit_decision_event_id
        OR NEW.requested_effect   IS DISTINCT FROM OLD.requested_effect
        OR NEW.decision_context   IS DISTINCT FROM OLD.decision_context
        OR NEW.created_at         IS DISTINCT FROM OLD.created_at
        OR NEW.ttl_expires_at     IS DISTINCT FROM OLD.ttl_expires_at
        OR NEW.approver_policy    IS DISTINCT FROM OLD.approver_policy
    THEN
        RAISE EXCEPTION
            'approval_requests row %: immutable column changed (S14 invariant)',
            OLD.approval_id
            USING ERRCODE = '23514';   -- check_violation
    END IF;

    -- (b) State-machine guard. Once a row leaves 'pending' it is
    -- terminal; no state regression allowed.
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

        -- Followup #9: bundling columns. Once-frozen-once-set.
        --   * NULL → non-NULL: admit (the legal mark_approval_bundled write)
        --   * non-NULL → anything different: reject (frozen)
        --   * NULL → NULL: admit (no-op same-value UPDATE)
        IF OLD.bundled_at IS NOT NULL
            AND NEW.bundled_at IS DISTINCT FROM OLD.bundled_at
        THEN
            RAISE EXCEPTION
                'approval_requests row %: bundled_at is frozen once set',
                OLD.approval_id
                USING ERRCODE = '23514';
        END IF;
        IF OLD.bundled_ledger_transaction_id IS NOT NULL
            AND NEW.bundled_ledger_transaction_id IS DISTINCT FROM OLD.bundled_ledger_transaction_id
        THEN
            RAISE EXCEPTION
                'approval_requests row %: bundled_ledger_transaction_id is frozen once set',
                OLD.approval_id
                USING ERRCODE = '23514';
        END IF;
        -- Defensively: bundled columns must be set together, never
        -- one-without-the-other.
        IF (NEW.bundled_at IS NULL) <> (NEW.bundled_ledger_transaction_id IS NULL) THEN
            RAISE EXCEPTION
                'approval_requests row %: bundled_at and bundled_ledger_transaction_id must be set together',
                OLD.approval_id
                USING ERRCODE = '23514';
        END IF;
    ELSE
        -- pending row: bundling columns must stay NULL until terminal.
        IF NEW.bundled_at IS NOT NULL OR NEW.bundled_ledger_transaction_id IS NOT NULL THEN
            RAISE EXCEPTION
                'approval_requests row %: bundling columns can only be set once the approval is terminal',
                OLD.approval_id
                USING ERRCODE = '23514';
        END IF;
    END IF;

    RETURN NEW;
END;
$$;

COMMENT ON FUNCTION approval_requests_block_immutable_updates IS
    'S14 + Codex round-4 + followup #9: rejects UPDATEs that would mutate any frozen column. Always-frozen: identity + payload + ttl + approver_policy. Once-frozen-on-terminal: state, resolution metadata, bundled_at + bundled_ledger_transaction_id. Bundling columns are set exactly once via mark_approval_bundled() after the row is terminal; both columns must move from NULL together.';

-- ---------------------------------------------------------------------
-- 3) mark_approval_bundled — atomic state assertion + UPDATE.
-- ---------------------------------------------------------------------
--
-- Called by the Rust orchestration layer (followup #9 part 2) AFTER
-- post_ledger_transaction has landed the deferred operation. Both
-- calls go inside one Rust-managed transaction so either both rows
-- (the new ledger_transactions row + the marked approval_requests row)
-- land or neither does.
--
-- Idempotent contract:
--   * mark_approval_bundled(A, T) on a not-yet-bundled approval →
--     UPDATE columns; return (TRUE, T).
--   * mark_approval_bundled(A, T) when row already shows
--     bundled_ledger_transaction_id = T → no-op; return (FALSE, T).
--   * mark_approval_bundled(A, T') when row already shows a different
--     bundled_ledger_transaction_id T → raise IDEMPOTENCY_CONFLICT.
--   * mark_approval_bundled on an approval whose state IS NOT terminal
--     → raise (the SP refuses to bundle pending rows).
--
-- Note on bundling-eligible terminal states:
--   * 'approved'  — the expected case (deferred op should land)
--   * 'denied'    — refuse: nothing to bundle (no op should land)
--   * 'expired'   — refuse: TTL elapsed, op should not land
--   * 'cancelled' — refuse: caller withdrew, op should not land
-- Defensively code rejects all but 'approved' so a buggy caller can't
-- bundle a denied / expired / cancelled approval into a real ledger op.

CREATE OR REPLACE FUNCTION mark_approval_bundled(
    p_approval_id              UUID,
    p_ledger_transaction_id    UUID
) RETURNS TABLE (
    was_first_bundling     BOOLEAN,
    ledger_transaction_id  UUID
) LANGUAGE plpgsql AS $$
DECLARE
    v_state                       TEXT;
    v_existing_bundled_tx         UUID;
BEGIN
    SELECT state, approval_requests.bundled_ledger_transaction_id
      INTO v_state, v_existing_bundled_tx
      FROM approval_requests
     WHERE approval_id = p_approval_id
     FOR UPDATE;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'approval_id % not found', p_approval_id
            USING ERRCODE = 'P0002';   -- no_data
    END IF;

    -- Only 'approved' rows can be bundled.
    IF v_state <> 'approved' THEN
        RAISE EXCEPTION
            'approval % cannot be bundled in state %',
            p_approval_id, v_state
            USING ERRCODE = '22023';   -- invalid_parameter_value
    END IF;

    -- Idempotency: same caller, same tx → no-op success.
    IF v_existing_bundled_tx IS NOT NULL THEN
        IF v_existing_bundled_tx = p_ledger_transaction_id THEN
            RETURN QUERY SELECT FALSE, v_existing_bundled_tx;
            RETURN;
        ELSE
            RAISE EXCEPTION
                'approval % already bundled with ledger_transaction_id % (caller asked for %)',
                p_approval_id, v_existing_bundled_tx, p_ledger_transaction_id
                USING ERRCODE = '40P03';   -- IDEMPOTENCY_CONFLICT (matches post_ledger_transaction convention)
        END IF;
    END IF;

    -- First bundling: set the columns.
    UPDATE approval_requests
       SET bundled_at = clock_timestamp(),
           bundled_ledger_transaction_id = p_ledger_transaction_id
     WHERE approval_id = p_approval_id;

    RETURN QUERY SELECT TRUE, p_ledger_transaction_id;
END;
$$;

COMMENT ON FUNCTION mark_approval_bundled IS
    'Followup #9: atomic mark for an approval whose deferred operation has just landed in ledger_transactions. Asserts state=approved. Idempotent on (approval_id, ledger_transaction_id); raises IDEMPOTENCY_CONFLICT (40P03) if a different tx is presented. Caller (Rust orchestrator) wraps post_ledger_transaction + this SP in one transaction so both rows commit atomically.';
