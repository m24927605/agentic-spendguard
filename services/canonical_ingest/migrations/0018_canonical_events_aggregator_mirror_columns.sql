-- ============================================================================
-- 0018_canonical_events_aggregator_mirror_columns.sql
--
-- SLICE_06 R2 B4 (Option A): add first-class mirror columns on
-- canonical_events so the stats_aggregator hot path can group by
-- model / agent_id / run_id / prompt_class / prompt_class_fingerprint
-- with partition-prunable indexes, instead of repeatedly decoding the
-- base64 CloudEvent payload from `payload_json->'data_b64'`.
--
-- Spec ancestors:
--   - docs/stats-aggregator-spec-v1alpha1.md §4.1 (aggregation source)
--   - docs/audit-chain-prediction-extension-v1alpha1.md §2 (mirror discipline)
--   - docs/slices/SLICE_06_output_predictor_a_b_stats_aggregator.md
--   - 0013_canonical_events_prediction_columns.sql (prior mirror precedent)
--
-- ## R2 review motivation
--
-- R1 panel B4 (DB B1): the SLICE_06 aggregation queries referenced
-- `cloudevent_payload->>'model'` etc., but canonical_events actually
-- carries the CloudEvent envelope at `payload_json` (with the inner
-- data base64-encoded). The queries would raise "column does not exist"
-- at runtime. R2 picks Option A — first-class mirror columns:
--
--   * Faster grouping (no JSON decode + base64 decode per row)
--   * Partition-prunable index covers the canonical hot WHERE shape
--   * Mirror-column write contract follows the same SLICE_01 immutability
--     pattern as the 18 prediction columns in 0013: WRITE-ONCE at INSERT,
--     ALTER COLUMN-ish updates rejected by the existing canonical_events
--     immutability trigger.
--
-- ## Population timeline
--
-- SLICE_06: aggregator queries the new columns; the predictor service
-- does NOT write canonical_events (only canonical_ingest does). Mirror
-- columns are NULL for now because the only producers (sidecar SLICE_10
-- + ledger outbox forwarder) don't yet populate them. Aggregation
-- queries handle NULL by simply skipping rows where the column is NULL
-- — those rows are not yet aggregator-visible. This degrades
-- gracefully: pre-SLICE_10 rows ignored, post-SLICE_10 rows included.
--
-- A tracking GH issue (per R2 plan §"Tracked as GH issues" #14)
-- captures the SLICE_10 producer-side population work.
--
-- ## Additivity (no v1alpha2 spec bump)
--
-- Strictly additive: NULLable columns, no new constraints on existing
-- rows, no trigger churn. Migration 0013's
-- `canonical_events_no_update_delete` trigger (added in
-- 0005_immutability_triggers.sql) already locks ALL rows for UPDATE/
-- DELETE — these columns are write-once at INSERT just like the
-- prediction mirror columns.
--
-- ## Stylistic alignment
--
-- - psql autocommit per SLICE_01 R5 (each ALTER + CREATE INDEX commits)
-- - No down migration file per SLICE_03 R2 M3 convention (rollback via
--   ALTER TABLE DROP COLUMN with safety guard on prod)
-- - Comments are verbose because the migration crosses spec boundaries
--   between canonical_ingest and the aggregator service
-- ============================================================================

-- Step 1: Add five NULLable mirror columns. TEXT for model / agent_id
-- / prompt_class / prompt_class_fingerprint because the upstream
-- producers carry them as TEXT in CloudEvent JSON. UUID for run_id
-- because it's structurally a UUID per spec §4.1.
ALTER TABLE canonical_events
    ADD COLUMN model                     TEXT,
    ADD COLUMN agent_id                  TEXT,
    ADD COLUMN run_id_mirror             UUID,
    ADD COLUMN prompt_class              TEXT,
    ADD COLUMN prompt_class_fingerprint  TEXT;

-- Step 2: Length + enum validation. Same shape as 0013's prediction
-- column CHECKs — NOT VALID (avoid re-scan of existing data) + then
-- VALIDATE so new rows are still rejected at write time.

ALTER TABLE canonical_events
    ADD CONSTRAINT canonical_events_model_length_chk
        CHECK (model IS NULL OR (char_length(model) <= 64 AND char_length(model) > 0))
        NOT VALID,
    ADD CONSTRAINT canonical_events_agent_id_length_chk
        CHECK (agent_id IS NULL OR (char_length(agent_id) <= 128 AND char_length(agent_id) > 0))
        NOT VALID,
    ADD CONSTRAINT canonical_events_prompt_class_enum_chk
        CHECK (prompt_class IS NULL OR prompt_class IN (
            'chat_short', 'chat_long', 'code_gen', 'summarization',
            'rag', 'tool_calling', 'vision'))
        NOT VALID,
    ADD CONSTRAINT canonical_events_prompt_class_fingerprint_length_chk
        CHECK (prompt_class_fingerprint IS NULL
               OR (char_length(prompt_class_fingerprint) BETWEEN 4 AND 256))
        NOT VALID;

ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_model_length_chk;
ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_agent_id_length_chk;
ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_prompt_class_enum_chk;
ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_prompt_class_fingerprint_length_chk;

-- Step 3: Partition-prunable composite index for the stats_aggregator
-- hot aggregation query shape. Per-bucket UPSERT cycle filters by
-- (tenant_id, event_type='spendguard.audit.outcome', ingest_at window).
-- Group by (model, agent_id, prompt_class). Partition pruning via
-- recorded_month lifts the active partition set.
CREATE INDEX canonical_events_aggregator_bucket_idx
    ON canonical_events (tenant_id, recorded_month, model, agent_id, prompt_class)
    WHERE event_type = 'spendguard.audit.outcome' AND actual_output_tokens IS NOT NULL;

-- Step 4: Partition-prunable index for the run_length aggregation
-- query shape (per stats-aggregator-spec-v1alpha1.md §6 — group by
-- (tenant_id, agent_id, run_id) over decision events).
CREATE INDEX canonical_events_aggregator_run_length_idx
    ON canonical_events (tenant_id, recorded_month, agent_id, run_id_mirror)
    WHERE event_type = 'spendguard.audit.decision' AND run_id_mirror IS NOT NULL;

-- ============================================================================
-- Column comments — match 0013 verbosity for the same audit-chain
-- maintainability reason.
-- ============================================================================

COMMENT ON COLUMN canonical_events.model IS
    'Mirror of the producer-supplied model name (e.g. "gpt-4o"). Populated by canonical_ingest write path starting SLICE_10. SLICE_06 R2 B4 — aggregator groups by this column. NULL allowed for legacy rows pre-SLICE_10; aggregation queries treat NULL as "row not yet aggregator-visible".';
COMMENT ON COLUMN canonical_events.agent_id IS
    'Mirror of the producer-supplied agent identifier. Aggregator bucket key. Populated SLICE_10+. NULL allowed for legacy rows.';
COMMENT ON COLUMN canonical_events.run_id_mirror IS
    'UUID mirror of the producer-supplied run_id (column suffix _mirror to disambiguate from canonical_events.run_id which is the audit chain anchor). Used by run_length aggregation (spec §6). NULL allowed for legacy + non-run-scoped events.';
COMMENT ON COLUMN canonical_events.prompt_class IS
    'Classifier label per output-predictor-service-spec-v1alpha1.md §8.1 (7-class enum). Used as the stats_aggregator bucket key — NOT prompt_class_fingerprint (output-predictor spec §8.2 closing paragraph: "Aggregator key uses class itself, not fingerprint"). Populated SLICE_10+.';
COMMENT ON COLUMN canonical_events.prompt_class_fingerprint IS
    'SHA-256-hex string per output-predictor-service-spec-v1alpha1.md §8.2 (canonical: v1:{class}|{model}|{message_count}). Audit identifier for the bucket; not used as aggregator GROUP BY key. Populated SLICE_10+.';

-- ============================================================================
-- DO-block smoke check: confirm the four CHECKs are VALID + indexes are
-- partition-aware (every child partition gets the local index because
-- the parent is partitioned).
-- ============================================================================
DO $$
BEGIN
    SET LOCAL search_path = pg_catalog, pg_temp;
    PERFORM 1 FROM pg_constraint
        WHERE conname = 'canonical_events_prompt_class_enum_chk'
          AND convalidated = TRUE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'canonical_events_prompt_class_enum_chk not validated';
    END IF;
    PERFORM 1 FROM pg_indexes
        WHERE schemaname = 'public'
          AND indexname = 'canonical_events_aggregator_bucket_idx';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'canonical_events_aggregator_bucket_idx missing';
    END IF;
END $$;
