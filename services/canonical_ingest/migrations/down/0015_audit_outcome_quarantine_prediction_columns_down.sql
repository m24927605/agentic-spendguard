-- Down-migration: reverse 0015_audit_outcome_quarantine_prediction_columns.sql
-- (round-3 fix B2; round-4 fixes M4 + M5).
--
-- Apply BEFORE 0013_down — the 18 columns mirror canonical_events but live
-- on a different table, so the only ordering constraint is the
-- internal one within this file (round-4 M5): DROP COLUMN must precede
-- CREATE OR REPLACE FUNCTION (see Step 2 rationale).
--
-- Round-3 fix M4 + round-4 fix M4: per-file destructive-down guard
-- (spendguard.allow_destructive_down_0015) — see slice §11 for the
-- exact SET LOCAL form.
-- Round-3 fix M10: no explicit BEGIN/COMMIT.
--
-- Round-4 fix M5: internal step order corrected. Round-3 ran
-- "CREATE OR REPLACE FUNCTION (revert to 24-col)" BEFORE "ALTER TABLE
-- DROP COLUMN (×18)". That created a tamper window: the reverted
-- function references only 24 columns of the OLD/NEW row, but the 18
-- prediction columns still exist on the table. An UPDATE in this window
-- could mutate any of the 18 columns and the trigger would silently
-- pass because the prediction-column fields are not in its comparison
-- list. Reversing the order — DROP COLUMN first, then revert the
-- function — ensures the function only ever references columns that
-- exist at trigger-fire time.

DO $$
BEGIN
    IF current_setting('spendguard.allow_destructive_down_0015', true) IS DISTINCT FROM 'on' THEN
        RAISE EXCEPTION 'destructive down-migration 0015 requires `SET LOCAL spendguard.allow_destructive_down_0015 = on` first';
    END IF;
    RAISE NOTICE 'DESTRUCTIVE down-migration 0015 proceeding (caller: %)', current_user;
END $$;

-- ============================================================================
-- Step 1: drop the sentinel/domain CHECK constraints that 0015's
-- up-migration installed. ALTER TABLE DROP COLUMN below cascades to
-- constraints that reference the dropped columns, but being explicit is
-- more self-documenting and resilient to constraint additions on legacy
-- rows.
-- ============================================================================
ALTER TABLE audit_outcome_quarantine
    DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_reserved_strategy_chk,
    DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_prediction_strategy_used_chk,
    DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_prediction_policy_used_chk,
    DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_tokenizer_tier_chk,
    DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_prediction_confidence_chk,
    DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_cold_start_layer_used_chk,
    DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_predicted_tokens_chk,
    DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_actual_tokens_chk,
    DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_run_steps_chk,
    DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_run_projection_chk,
    DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_run_projection_int64_chk,
    DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_prediction_sample_size_chk,
    DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_delta_b_ratio_chk,
    DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_delta_c_ratio_chk,
    DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_predicted_b_tokens_nonzero_chk,
    DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_predicted_c_tokens_nonzero_chk,
    DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_cold_start_layer_outcome_chk;

-- ============================================================================
-- Step 2 (round-4 fix M5): drop the 18 prediction columns FIRST. This
-- closes the tamper window where the trigger function below would only
-- check 24 columns while 18 mutable columns still exist on the table.
--
-- DROP COLUMN takes ACCESS EXCLUSIVE on the table, so no concurrent
-- UPDATE can sneak through during the DDL itself.
-- ============================================================================
ALTER TABLE audit_outcome_quarantine
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

-- ============================================================================
-- Step 3 (round-4 fix M5): NOW revert the trigger function. Round-3 ran
-- this BEFORE the DROP COLUMN, leaving an interval where the reverted
-- function failed to cover the 18 still-present columns. With the
-- columns already gone, the 24-col tuple comparison matches the table
-- shape exactly.
--
-- Restored body is the pre-SLICE_01 version from
-- services/canonical_ingest/migrations/0005_immutability_triggers.sql.
-- ============================================================================
CREATE OR REPLACE FUNCTION reject_quarantine_immutable_columns()
RETURNS TRIGGER AS $$
BEGIN
    IF (OLD.quarantine_id, OLD.event_id, OLD.tenant_id, OLD.decision_id,
        OLD.storage_class, OLD.producer_id, OLD.producer_sequence,
        OLD.producer_signature, OLD.signing_key_id, OLD.schema_bundle_id,
        OLD.schema_bundle_hash, OLD.event_type, OLD.specversion, OLD.source,
        OLD.event_time, OLD.datacontenttype, OLD.payload_json,
        OLD.payload_blob_ref, OLD.region_id, OLD.ingest_shard_id,
        OLD.ingest_log_offset, OLD.run_id, OLD.quarantined_at,
        OLD.orphan_after)
       IS DISTINCT FROM
       (NEW.quarantine_id, NEW.event_id, NEW.tenant_id, NEW.decision_id,
        NEW.storage_class, NEW.producer_id, NEW.producer_sequence,
        NEW.producer_signature, NEW.signing_key_id, NEW.schema_bundle_id,
        NEW.schema_bundle_hash, NEW.event_type, NEW.specversion, NEW.source,
        NEW.event_time, NEW.datacontenttype, NEW.payload_json,
        NEW.payload_blob_ref, NEW.region_id, NEW.ingest_shard_id,
        NEW.ingest_log_offset, NEW.run_id, NEW.quarantined_at,
        NEW.orphan_after) THEN
        RAISE EXCEPTION 'audit_outcome_quarantine immutable columns cannot be changed'
            USING ERRCODE = '42P10';
    END IF;
    IF OLD.state = 'released' OR OLD.state = 'orphaned' THEN
        RAISE EXCEPTION 'audit_outcome_quarantine state is terminal'
            USING ERRCODE = '42P10';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
