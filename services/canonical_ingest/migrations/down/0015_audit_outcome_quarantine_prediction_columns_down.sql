-- Down-migration: reverse 0015_audit_outcome_quarantine_prediction_columns.sql
-- (round-3 fix B2).
--
-- Apply BEFORE 0013_down — the 18 columns mirror canonical_events but live
-- on a different table, so order between 0013_down and 0015_down is
-- internal-only.
--
-- Round-3 fix M4: destructive-down guard.
-- Round-3 fix M10: no explicit BEGIN/COMMIT.

DO $$
BEGIN
    IF current_setting('spendguard.allow_destructive_down', true) IS DISTINCT FROM 'on' THEN
        RAISE EXCEPTION 'destructive down-migration 0015 requires `SET spendguard.allow_destructive_down = on` first';
    END IF;
END $$;

-- Restore the pre-SLICE_01 quarantine immutability trigger function from
-- services/canonical_ingest/migrations/0005_immutability_triggers.sql.
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
