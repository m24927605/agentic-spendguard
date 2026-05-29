-- Update reject_audit_outbox_immutable_columns to cover the 18 new
-- prediction columns added in 0046.
--
-- Spec: docs/audit-chain-prediction-extension-v1alpha1.md §5 (critical
-- surface) and §5.2 verbatim trigger body.
--
-- Critical risk this closes (HANDOFF Step 4 discrepancy #4): the original
-- trigger function (migration 0011) only compares the 14 base columns.
-- Without this update, the 18 new columns are silently mutable by any
-- UPDATE statement — DBA / forwarder ORM / attacker could rewrite
-- calibration evidence after INSERT and `verify-chain` would still see
-- the signature match if mirror was tampered alongside (only mirror
-- cross-check from §11.2 would catch it). With this update, any UPDATE
-- to a prediction column raises Postgres errcode 42P10.
--
-- The function is CREATE OR REPLACE — no trigger DROP, no audit
-- downtime. The existing audit_outbox_immutability trigger continues to
-- fire BEFORE UPDATE; it just dispatches to the new function body.
--
-- Forwarder-state columns (pending_forward, forwarded_at,
-- forward_attempts, last_forward_error) remain UPDATE-able — they are
-- intentionally excluded from both OLD and NEW tuples, so a forwarder
-- UPDATE that only touches those four columns produces tuples that are
-- still equal under IS DISTINCT FROM, and the trigger passes silently.
-- See audit-chain-prediction-extension-v1alpha1.md §5.3.

BEGIN;

CREATE OR REPLACE FUNCTION reject_audit_outbox_immutable_columns()
RETURNS TRIGGER AS $$
BEGIN
    IF (OLD.audit_outbox_id, OLD.audit_decision_event_id, OLD.decision_id,
        OLD.tenant_id, OLD.ledger_transaction_id, OLD.event_type,
        OLD.cloudevent_payload, OLD.cloudevent_payload_signature,
        OLD.ledger_fencing_epoch, OLD.workload_instance_id,
        OLD.recorded_at, OLD.recorded_month,
        OLD.producer_sequence, OLD.idempotency_key,
        -- === NEW prediction columns (per audit-chain-prediction-extension §5.2) ===
        OLD.predicted_a_tokens, OLD.predicted_b_tokens, OLD.predicted_c_tokens,
        OLD.reserved_strategy, OLD.prediction_strategy_used,
        OLD.prediction_policy_used, OLD.tokenizer_tier, OLD.tokenizer_version_id,
        OLD.prediction_confidence, OLD.prediction_sample_size,
        OLD.cold_start_layer_used,
        OLD.run_projection_at_decision_atomic,
        OLD.run_predicted_remaining_steps,
        OLD.run_steps_completed_so_far,
        OLD.actual_input_tokens, OLD.actual_output_tokens,
        OLD.delta_b_ratio, OLD.delta_c_ratio)
       IS DISTINCT FROM
       (NEW.audit_outbox_id, NEW.audit_decision_event_id, NEW.decision_id,
        NEW.tenant_id, NEW.ledger_transaction_id, NEW.event_type,
        NEW.cloudevent_payload, NEW.cloudevent_payload_signature,
        NEW.ledger_fencing_epoch, NEW.workload_instance_id,
        NEW.recorded_at, NEW.recorded_month,
        NEW.producer_sequence, NEW.idempotency_key,
        NEW.predicted_a_tokens, NEW.predicted_b_tokens, NEW.predicted_c_tokens,
        NEW.reserved_strategy, NEW.prediction_strategy_used,
        NEW.prediction_policy_used, NEW.tokenizer_tier, NEW.tokenizer_version_id,
        NEW.prediction_confidence, NEW.prediction_sample_size,
        NEW.cold_start_layer_used,
        NEW.run_projection_at_decision_atomic,
        NEW.run_predicted_remaining_steps,
        NEW.run_steps_completed_so_far,
        NEW.actual_input_tokens, NEW.actual_output_tokens,
        NEW.delta_b_ratio, NEW.delta_c_ratio) THEN
        RAISE EXCEPTION 'audit_outbox immutable columns cannot be changed (incl. prediction extension cols)'
            USING ERRCODE = '42P10';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

COMMIT;
