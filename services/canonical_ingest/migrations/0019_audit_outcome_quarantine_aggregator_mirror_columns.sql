-- HARDEN_03 R2: keep SLICE_06 aggregator mirror columns intact when an
-- audit.outcome arrives before its audit.decision and is released from
-- audit_outcome_quarantine later.
--
-- 0018 added model / agent_id / run_id_mirror / prompt_class /
-- prompt_class_fingerprint to canonical_events. The out-of-order
-- outcome path persisted only prediction columns in quarantine, so
-- release_quarantined_outcomes() inserted NULL mirrors even though the
-- signed payload carried the values. The stats_aggregator requires
-- non-NULL model + agent_id + prompt_class and silently skipped those
-- released rows.

ALTER TABLE audit_outcome_quarantine
    ADD COLUMN model                     TEXT,
    ADD COLUMN agent_id                  TEXT,
    ADD COLUMN run_id_mirror             UUID,
    ADD COLUMN prompt_class              TEXT,
    ADD COLUMN prompt_class_fingerprint  TEXT;

ALTER TABLE audit_outcome_quarantine
    ADD CONSTRAINT audit_outcome_quarantine_model_length_chk
        CHECK (model IS NULL OR (char_length(model) <= 64 AND char_length(model) > 0))
        NOT VALID,
    ADD CONSTRAINT audit_outcome_quarantine_agent_id_length_chk
        CHECK (agent_id IS NULL OR (char_length(agent_id) <= 128 AND char_length(agent_id) > 0))
        NOT VALID,
    ADD CONSTRAINT audit_outcome_quarantine_prompt_class_enum_chk
        CHECK (prompt_class IS NULL OR prompt_class IN (
            'chat_short', 'chat_long', 'code_gen', 'summarization',
            'rag', 'tool_calling', 'vision'))
        NOT VALID,
    ADD CONSTRAINT audit_outcome_quarantine_prompt_class_fingerprint_length_chk
        CHECK (prompt_class_fingerprint IS NULL
               OR (char_length(prompt_class_fingerprint) BETWEEN 4 AND 256))
        NOT VALID;

ALTER TABLE audit_outcome_quarantine VALIDATE CONSTRAINT audit_outcome_quarantine_model_length_chk;
ALTER TABLE audit_outcome_quarantine VALIDATE CONSTRAINT audit_outcome_quarantine_agent_id_length_chk;
ALTER TABLE audit_outcome_quarantine VALIDATE CONSTRAINT audit_outcome_quarantine_prompt_class_enum_chk;
ALTER TABLE audit_outcome_quarantine VALIDATE CONSTRAINT audit_outcome_quarantine_prompt_class_fingerprint_length_chk;

CREATE INDEX audit_outcome_quarantine_aggregator_bucket_idx
    ON audit_outcome_quarantine (tenant_id, model, agent_id, prompt_class)
    WHERE state = 'awaiting_decision'
      AND event_type = 'spendguard.audit.outcome'
      AND actual_output_tokens IS NOT NULL;

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
        OLD.model, OLD.agent_id, OLD.run_id_mirror, OLD.prompt_class,
        OLD.prompt_class_fingerprint,
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
        NEW.model, NEW.agent_id, NEW.run_id_mirror, NEW.prompt_class,
        NEW.prompt_class_fingerprint,
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
        RAISE EXCEPTION 'audit_outcome_quarantine immutable columns cannot be changed (incl. prediction + aggregator mirror cols)'
            USING ERRCODE = '42P10';
    END IF;
    IF OLD.state = 'released' OR OLD.state = 'orphaned' THEN
        RAISE EXCEPTION 'audit_outcome_quarantine state is terminal'
            USING ERRCODE = '42P10';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

COMMENT ON COLUMN audit_outcome_quarantine.model IS
    'HARDEN_03 R2: quarantine copy of canonical_events.model so released out-of-order outcomes stay visible to stats_aggregator.';
COMMENT ON COLUMN audit_outcome_quarantine.agent_id IS
    'HARDEN_03 R2: quarantine copy of canonical_events.agent_id for released out-of-order outcomes.';
COMMENT ON COLUMN audit_outcome_quarantine.run_id_mirror IS
    'HARDEN_03 R2: quarantine copy of canonical_events.run_id_mirror for released out-of-order outcomes.';
COMMENT ON COLUMN audit_outcome_quarantine.prompt_class IS
    'HARDEN_03 R2: quarantine copy of canonical_events.prompt_class for released out-of-order outcomes.';
COMMENT ON COLUMN audit_outcome_quarantine.prompt_class_fingerprint IS
    'HARDEN_03 R2: quarantine copy of canonical_events.prompt_class_fingerprint for released out-of-order outcomes.';

DO $$
BEGIN
    SET LOCAL search_path = pg_catalog, pg_temp;
    PERFORM 1 FROM pg_constraint
        WHERE conname = 'audit_outcome_quarantine_prompt_class_enum_chk'
          AND convalidated = TRUE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'audit_outcome_quarantine_prompt_class_enum_chk not validated';
    END IF;
    PERFORM 1 FROM pg_indexes
        WHERE schemaname = 'public'
          AND indexname = 'audit_outcome_quarantine_aggregator_bucket_idx';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'audit_outcome_quarantine_aggregator_bucket_idx missing';
    END IF;
END $$;
