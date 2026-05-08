-- Immutability triggers (per Ledger §6.1 + Stage 2 §4.3 audit_outbox tightening).
-- Defense-in-depth alongside role + procedure (§6.2 + §6.3).

-- ============================================================================
-- ledger_entries: complete immutability (no UPDATE / DELETE).
-- ============================================================================
CREATE OR REPLACE FUNCTION reject_immutable_ledger_entry_mutation()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'ledger_entries are immutable; use compensating entry'
        USING ERRCODE = '42P10';
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER ledger_entries_no_update_delete
    BEFORE UPDATE OR DELETE ON ledger_entries
    FOR EACH ROW EXECUTE FUNCTION reject_immutable_ledger_entry_mutation();

-- ============================================================================
-- ledger_units identity columns: immutable.
-- ============================================================================
CREATE OR REPLACE FUNCTION reject_replay_identity_mutation()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'replay-critical identity columns are immutable'
        USING ERRCODE = '42P10';
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER ledger_units_no_identity_update
    BEFORE UPDATE OF unit_kind, currency, unit_name, scale, rounding_mode,
                     token_kind, model_family, credit_program
    ON ledger_units
    FOR EACH ROW EXECUTE FUNCTION reject_replay_identity_mutation();

-- ============================================================================
-- budget_window_instances: completely immutable (no UPDATE / DELETE).
-- ============================================================================
CREATE OR REPLACE FUNCTION reject_immutable_reference_mutation()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'replay-critical reference is immutable'
        USING ERRCODE = '42P10';
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER budget_window_instances_no_update_delete
    BEFORE UPDATE OR DELETE ON budget_window_instances
    FOR EACH ROW EXECUTE FUNCTION reject_immutable_reference_mutation();

-- ============================================================================
-- pricing_snapshots: completely immutable.
-- ============================================================================
CREATE TRIGGER pricing_snapshots_no_update_delete
    BEFORE UPDATE OR DELETE ON pricing_snapshots
    FOR EACH ROW EXECUTE FUNCTION reject_immutable_reference_mutation();

-- ============================================================================
-- audit_outbox: tightened immutability (Stage 2 v2 patch).
-- Only forwarder state fields are UPDATE-able.
-- ============================================================================
CREATE OR REPLACE FUNCTION reject_audit_outbox_immutable_columns()
RETURNS TRIGGER AS $$
BEGIN
    IF (OLD.audit_outbox_id, OLD.audit_decision_event_id, OLD.decision_id,
        OLD.tenant_id, OLD.ledger_transaction_id, OLD.event_type,
        OLD.cloudevent_payload, OLD.cloudevent_payload_signature,
        OLD.ledger_fencing_epoch, OLD.workload_instance_id,
        OLD.recorded_at, OLD.recorded_month,
        OLD.producer_sequence, OLD.idempotency_key)
       IS DISTINCT FROM
       (NEW.audit_outbox_id, NEW.audit_decision_event_id, NEW.decision_id,
        NEW.tenant_id, NEW.ledger_transaction_id, NEW.event_type,
        NEW.cloudevent_payload, NEW.cloudevent_payload_signature,
        NEW.ledger_fencing_epoch, NEW.workload_instance_id,
        NEW.recorded_at, NEW.recorded_month,
        NEW.producer_sequence, NEW.idempotency_key) THEN
        RAISE EXCEPTION 'audit_outbox immutable columns cannot be changed'
            USING ERRCODE = '42P10';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER audit_outbox_immutability
    BEFORE UPDATE ON audit_outbox
    FOR EACH ROW EXECUTE FUNCTION reject_audit_outbox_immutable_columns();

-- DELETE protection.
CREATE TRIGGER audit_outbox_no_delete
    BEFORE DELETE ON audit_outbox
    FOR EACH ROW EXECUTE FUNCTION reject_immutable_ledger_entry_mutation();

-- ============================================================================
-- ledger_transactions: identity columns immutable + DELETE forbidden.
-- Posting state machine is allowed: posting_state / posted_at can transition
-- pending -> posted -> voided once. All other columns immutable.
-- ============================================================================
CREATE OR REPLACE FUNCTION reject_ledger_transactions_identity_mutation()
RETURNS TRIGGER AS $$
BEGIN
    IF (OLD.ledger_transaction_id, OLD.tenant_id, OLD.operation_kind,
        OLD.idempotency_key, OLD.request_hash,
        OLD.audit_decision_event_id, OLD.decision_id,
        OLD.effective_at, OLD.recorded_at,
        OLD.lock_order_token, OLD.fencing_scope_id, OLD.fencing_epoch_at_post,
        OLD.trace_event_id)
       IS DISTINCT FROM
       (NEW.ledger_transaction_id, NEW.tenant_id, NEW.operation_kind,
        NEW.idempotency_key, NEW.request_hash,
        NEW.audit_decision_event_id, NEW.decision_id,
        NEW.effective_at, NEW.recorded_at,
        NEW.lock_order_token, NEW.fencing_scope_id, NEW.fencing_epoch_at_post,
        NEW.trace_event_id) THEN
        RAISE EXCEPTION 'ledger_transactions immutable identity columns cannot be changed'
            USING ERRCODE = '42P10';
    END IF;
    -- Posting state transitions: pending -> posted | voided; posted -> voided is allowed
    -- only via compensating entries in code path (not direct UPDATE).
    IF OLD.posting_state = 'posted' AND NEW.posting_state <> 'posted' THEN
        RAISE EXCEPTION 'ledger_transactions: posted state is terminal; use compensating entry'
            USING ERRCODE = '42P10';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER ledger_transactions_identity_immutability
    BEFORE UPDATE ON ledger_transactions
    FOR EACH ROW EXECUTE FUNCTION reject_ledger_transactions_identity_mutation();

CREATE TRIGGER ledger_transactions_no_delete
    BEFORE DELETE ON ledger_transactions
    FOR EACH ROW EXECUTE FUNCTION reject_immutable_ledger_entry_mutation();

-- ============================================================================
-- ledger_accounts: identity columns immutable + DELETE forbidden.
-- ============================================================================
CREATE TRIGGER ledger_accounts_no_update_identity
    BEFORE UPDATE OF ledger_account_id, tenant_id, budget_id,
                     window_instance_id, account_kind, unit_id
    ON ledger_accounts
    FOR EACH ROW EXECUTE FUNCTION reject_replay_identity_mutation();

CREATE TRIGGER ledger_accounts_no_delete
    BEFORE DELETE ON ledger_accounts
    FOR EACH ROW EXECUTE FUNCTION reject_immutable_ledger_entry_mutation();

-- ============================================================================
-- audit_outbox_global_keys: completely immutable.
-- ============================================================================
CREATE TRIGGER audit_outbox_global_keys_no_update_delete
    BEFORE UPDATE OR DELETE ON audit_outbox_global_keys
    FOR EACH ROW EXECUTE FUNCTION reject_immutable_reference_mutation();

-- ============================================================================
-- "no audit, no effect" deferred constraint trigger.
--
-- Every ledger_transactions row MUST have at least one matching audit_outbox
-- row at commit time. Stored proc enforces this in its happy path, but the
-- trigger is defense-in-depth against direct SQL writes that bypass the proc.
-- DEFERRABLE INITIALLY DEFERRED — checks run at COMMIT, after audit_outbox
-- INSERT in the same tx.
-- ============================================================================
CREATE OR REPLACE FUNCTION assert_ledger_transaction_has_audit()
RETURNS TRIGGER AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM audit_outbox
         WHERE ledger_transaction_id = NEW.ledger_transaction_id
    ) THEN
        RAISE EXCEPTION
            'AUDIT_INVARIANT_VIOLATED: ledger_transaction % posted with no audit_outbox row',
            NEW.ledger_transaction_id
            USING ERRCODE = '23514';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER ledger_transactions_must_have_audit
    AFTER INSERT ON ledger_transactions
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION assert_ledger_transaction_has_audit();

-- ============================================================================
-- Per-(transaction, unit_id) balance constraint trigger (Ledger §3 backstop).
-- Stored procedure also enforces inline; this is defense-in-depth.
-- ============================================================================
CREATE OR REPLACE FUNCTION assert_ledger_transaction_balanced_per_unit()
RETURNS TRIGGER AS $$
DECLARE
    v_imbalanced TEXT;
BEGIN
    SELECT string_agg(unit_id::TEXT || ':' || diff::TEXT, ', ')
    INTO v_imbalanced
    FROM (
        SELECT unit_id,
               SUM(CASE WHEN direction = 'debit'  THEN amount_atomic
                        WHEN direction = 'credit' THEN -amount_atomic
                   END) AS diff
        FROM ledger_entries
        WHERE ledger_transaction_id = NEW.ledger_transaction_id
        GROUP BY unit_id
    ) per_unit
    WHERE diff <> 0;

    IF v_imbalanced IS NOT NULL THEN
        RAISE EXCEPTION 'per-unit balance violation: %', v_imbalanced
            USING ERRCODE = '23514';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER ledger_transaction_balanced_per_unit
    AFTER INSERT OR UPDATE ON ledger_entries
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION assert_ledger_transaction_balanced_per_unit();
