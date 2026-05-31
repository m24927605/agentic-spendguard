-- Down-migration: reverse 0019_audit_outcome_quarantine_aggregator_mirror_columns.sql

DROP INDEX IF EXISTS audit_outcome_quarantine_aggregator_bucket_idx;

ALTER TABLE audit_outcome_quarantine
    DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_prompt_class_fingerprint_length_chk,
    DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_prompt_class_enum_chk,
    DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_agent_id_length_chk,
    DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_model_length_chk;

ALTER TABLE audit_outcome_quarantine
    DROP COLUMN IF EXISTS prompt_class_fingerprint,
    DROP COLUMN IF EXISTS prompt_class,
    DROP COLUMN IF EXISTS run_id_mirror,
    DROP COLUMN IF EXISTS agent_id,
    DROP COLUMN IF EXISTS model;

-- Restore the 0015 trigger body that protects prediction-extension
-- columns but does not reference the 0019 aggregator mirror columns.
CREATE OR REPLACE FUNCTION reject_quarantine_immutable_columns()
RETURNS TRIGGER
SECURITY INVOKER
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF (OLD.quarantine_id, OLD.event_id, OLD.tenant_id, OLD.decision_id,
        OLD.storage_class, OLD.producer_id, OLD.producer_sequence,
        OLD.producer_signature, OLD.signing_key_id, OLD.schema_bundle_id,
        OLD.schema_bundle_hash, OLD.event_type, OLD.specversion, OLD.source,
        OLD.event_time, OLD.datacontenttype, OLD.payload_json,
        OLD.payload_blob_ref, OLD.region_id, OLD.ingest_shard_id,
        OLD.ingest_log_offset, OLD.run_id, OLD.quarantined_at,
        OLD.orphan_after,
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
       (NEW.quarantine_id, NEW.event_id, NEW.tenant_id, NEW.decision_id,
        NEW.storage_class, NEW.producer_id, NEW.producer_sequence,
        NEW.producer_signature, NEW.signing_key_id, NEW.schema_bundle_id,
        NEW.schema_bundle_hash, NEW.event_type, NEW.specversion, NEW.source,
        NEW.event_time, NEW.datacontenttype, NEW.payload_json,
        NEW.payload_blob_ref, NEW.region_id, NEW.ingest_shard_id,
        NEW.ingest_log_offset, NEW.run_id, NEW.quarantined_at,
        NEW.orphan_after,
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
        RAISE EXCEPTION 'audit_outcome_quarantine immutable columns cannot be changed (incl. prediction extension cols)'
            USING ERRCODE = '42P10';
    END IF;
    IF OLD.state = 'released' OR OLD.state = 'orphaned' THEN
        RAISE EXCEPTION 'audit_outcome_quarantine state is terminal'
            USING ERRCODE = '42P10';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
