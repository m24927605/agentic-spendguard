-- Down-migration: reverse 0046_audit_outbox_prediction_columns.sql
-- (round-2 fix m2).
--
-- Spec ref: docs/slices/SLICE_01_canonical_events_migration.md §11
--   rollback plan.
--
-- ## Rollback order (per slice §11)
--
--   1. Stop all 4 producer services so no new tag-300+ writes happen.
--   2. Apply this down-migration (drops 18 columns + restores the
--      pre-SLICE_01 trigger function + removes the TRUNCATE guard +
--      drops the 2026-08 through 2026-10 partitions IF they are empty).
--   3. Apply 0048's down-migration to drop tokenizer_versions and the FK.
--   4. Apply 0013's down-migration on the canonical DB.
--   5. Roll back canonical_ingest pods to the pre-SLICE_01 image.
--
-- ## Idempotency
--
-- All ALTER TABLE ... DROP COLUMN are guarded with IF EXISTS so re-running
-- after partial rollback is safe. DROP TRIGGER IF EXISTS likewise.
--
-- ## Data loss warning
--
-- Dropping the 18 columns is destructive. Any row written by SLICE_06+
-- producers will lose its prediction columns; the CloudEvent proto
-- payload in cloudevent_payload still carries the tag-300+ data, but
-- the SQL-side accelerators are gone. calibration-report stops working
-- until the columns are re-added.
--
-- For non-destructive rollback (e.g., temporary disable while debugging
-- a producer bug), prefer reverting only the producer image without
-- touching the schema — the columns sit unused but harmless.

BEGIN;

-- Restore the pre-SLICE_01 trigger function (from
-- services/ledger/migrations/0011_immutability_triggers.sql).
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

-- Drop the TRUNCATE guard added in 0046 step 7.
DROP TRIGGER IF EXISTS audit_outbox_no_truncate ON audit_outbox;

-- Drop CHECK constraints. Order doesn't matter; they are independent.
ALTER TABLE audit_outbox
    DROP CONSTRAINT IF EXISTS audit_outbox_reserved_strategy_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_prediction_strategy_used_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_prediction_policy_used_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_tokenizer_tier_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_prediction_confidence_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_cold_start_layer_used_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_predicted_tokens_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_actual_tokens_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_run_steps_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_run_projection_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_run_projection_int64_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_prediction_sample_size_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_delta_b_ratio_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_delta_c_ratio_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_decision_required_cols_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_outcome_required_cols_chk;

-- Drop indexes (PG drops them transparently with the columns but
-- being explicit makes the rollback intent obvious).
DROP INDEX IF EXISTS audit_outbox_calibration_idx;
DROP INDEX IF EXISTS audit_outbox_tier_idx;
DROP INDEX IF EXISTS audit_outbox_outcome_calibration_idx;

-- Drop pre-created partitions (only safe if they are empty; if rows
-- have landed in 2026-08+ the operator must manually move them first).
DROP TABLE IF EXISTS audit_outbox_2026_10;
DROP TABLE IF EXISTS audit_outbox_2026_09;
DROP TABLE IF EXISTS audit_outbox_2026_08;

-- Drop the 18 prediction columns.
ALTER TABLE audit_outbox
    DROP COLUMN IF EXISTS predicted_a_tokens,
    DROP COLUMN IF EXISTS predicted_b_tokens,
    DROP COLUMN IF EXISTS predicted_c_tokens,
    DROP COLUMN IF EXISTS reserved_strategy,
    DROP COLUMN IF EXISTS prediction_strategy_used,
    DROP COLUMN IF EXISTS prediction_policy_used,
    DROP COLUMN IF EXISTS tokenizer_tier,
    DROP COLUMN IF EXISTS tokenizer_version_id,
    DROP COLUMN IF EXISTS prediction_confidence,
    DROP COLUMN IF EXISTS prediction_sample_size,
    DROP COLUMN IF EXISTS cold_start_layer_used,
    DROP COLUMN IF EXISTS run_projection_at_decision_atomic,
    DROP COLUMN IF EXISTS run_predicted_remaining_steps,
    DROP COLUMN IF EXISTS run_steps_completed_so_far,
    DROP COLUMN IF EXISTS actual_input_tokens,
    DROP COLUMN IF EXISTS actual_output_tokens,
    DROP COLUMN IF EXISTS delta_b_ratio,
    DROP COLUMN IF EXISTS delta_c_ratio;

COMMIT;
