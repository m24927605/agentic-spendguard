-- Round-3 fix B2: extend audit_outcome_quarantine with the 18 prediction
-- columns so the release-from-quarantine path can re-hydrate them when
-- promoting a quarantined outcome to canonical_events.
--
-- Spec ancestor: docs/audit-chain-prediction-extension-v1alpha1.md §11.2
-- (cross-storage consistency) + SLICE_01 §6 (mirror invariant).
--
-- ## Why this matters
--
-- Without these columns the quarantine table is the only path where an
-- audit.outcome can land on canonical_events. The 0013 migration added
-- the 18 columns on canonical_events directly but did NOT mirror them
-- onto audit_outcome_quarantine. As a result:
--
--   1. Producer writes an audit.outcome with tag-300+ fields populated.
--   2. canonical_ingest decodes it, sees no preceding audit.decision,
--      routes to quarantine via insert into audit_outcome_quarantine.
--   3. Quarantine row stores cloudevent_payload (which carries the
--      prediction tags as proto bytes) but has nowhere to store the
--      decoded first-class column values.
--   4. The matching .decision arrives later; release_quarantined_outcomes()
--      reads the quarantine row + INSERTs into canonical_events.
--   5. Because the quarantine row has no first-class prediction columns,
--      the INSERT into canonical_events lands them all as NULL — even
--      though the proto bytes inside cloudevent_payload contain the
--      values.
--   6. The CHECK constraint
--      canonical_events_outcome_required_cols_chk (event_time >= 2026-07-01)
--      fires because actual_input_tokens / actual_output_tokens are NULL.
--      The release transaction aborts. The quarantine row gets stuck
--      forever in 'awaiting_decision' state.
--   7. Bonus failure: even if the CHECK passed, the verify-chain mirror
--      cross-check would flag a divergence (column NULL vs proto-encoded
--      value).
--
-- Fix: mirror the same 18 nullable columns onto the quarantine table so
-- the release path can carry them forward.
--
-- ## Round-4 fix M2 design decision: mirror sentinel CHECKs WITH release-path coherence
--
-- ### What changed in round-4
--
-- Round-3 explicitly skipped mirroring the 17 sentinel CHECKs to keep
-- the quarantine path minimal. Round-4 codex M2 escalated this: if the
-- producer writes a value that violates a sentinel CHECK
-- (e.g., predicted_b_tokens >= 0, delta_b_ratio non-NaN), the
-- canonical_events INSERT at release time fires the CHECK and aborts
-- the release transaction. The quarantine row gets stuck forever in
-- 'awaiting_decision' state because the canonical_events_global_keys
-- INSERT also rolls back. Worse: the bad bytes sit in the quarantine
-- table's payload_json, indistinguishable from a benign late-arrival.
--
-- Decision: mirror the 17 type/range/sentinel CHECKs onto the quarantine
-- table so producer-side malformation is rejected at the quarantine
-- INSERT (within 30s of arrival per spec §4.8 SLO) rather than 30s+
-- later at the release-path INSERT into canonical_events.
--
-- ### Sentinel-collision guards: chose (b) "release path preserves
-- original payload"
--
-- Codex flagged three options for the
-- predicted_b_tokens > 0 WHEN strategy='B' sentinel-collision CHECK:
--   (a) Drop the > 0 guard and accept the NULL↔0 collision.
--   (b) Mark the release path "always populates from carried value,
--       never coerces NULL→0".
--   (c) Add release-path coercion: if predicted_b_tokens=0 AND strategy='B',
--       INSERT with NULL.
--
-- We pick (b). Rationale: the quarantine release path's invariant is
-- byte-identical preservation of the original audit.outcome payload.
-- Option (a) loses sentinel discipline at the SQL boundary and pushes
-- the burden onto every consumer. Option (c) introduces a silent
-- producer-malformation rewrite that masks bugs in the upstream
-- spendguard-prediction-mirror crate.
--
-- Concrete implication: a producer that writes
-- prediction_strategy_used='B' AND predicted_b_tokens=0 fails the
-- quarantine INSERT with errcode 23514. The producer gets a clear
-- error at the gRPC boundary; the operator sees a single-event
-- failure rather than a stuck quarantine row + a late release
-- abort.
--
-- ### What is NOT mirrored: the partial-NOT-NULL CHECKs from 0013 Step 3
--
-- Those CHECKs gate event_type×event_time on "all required cols are
-- populated". The quarantine table is a strict subset of audit.outcome
-- events; the corresponding NOT-NULL set differs (only actual_*_tokens
-- are required, not the decision-side columns). Mirroring would force
-- producer code to populate decision-side columns it doesn't have at
-- audit.outcome write time. The canonical_events INSERT at release time
-- re-validates the outcome-side NOT-NULLs anyway.
--
-- ## No backfill
--
-- Existing quarantine rows (pre-SLICE_01) have NULL on the new columns.
-- Those rows are either already released (state='released'; new columns
-- stay NULL, signature still verifies because the CloudEvent payload is
-- already on canonical_events) or orphaned (state='orphaned'; never
-- reaches canonical_events). No retroactive fixup needed.

ALTER TABLE audit_outcome_quarantine
    -- === Decision-side prediction columns (11 per spec §2.1) ===
    ADD COLUMN predicted_a_tokens         BIGINT,
    ADD COLUMN predicted_b_tokens         BIGINT,
    ADD COLUMN predicted_c_tokens         BIGINT,
    ADD COLUMN reserved_strategy          TEXT,
    ADD COLUMN prediction_strategy_used   TEXT,
    ADD COLUMN prediction_policy_used     TEXT,
    ADD COLUMN tokenizer_tier             TEXT,
    ADD COLUMN tokenizer_version_id       UUID,
    ADD COLUMN prediction_confidence      NUMERIC(4,3),
    ADD COLUMN prediction_sample_size     BIGINT,
    ADD COLUMN cold_start_layer_used      TEXT,
    -- === Run-level projection columns (3 per spec §2.2) ===
    ADD COLUMN run_projection_at_decision_atomic NUMERIC(38,0),
    ADD COLUMN run_predicted_remaining_steps     INT,
    ADD COLUMN run_steps_completed_so_far        BIGINT,
    -- === Commit-side actual columns (4 per spec §2.3) ===
    ADD COLUMN actual_input_tokens   BIGINT,
    ADD COLUMN actual_output_tokens  BIGINT,
    ADD COLUMN delta_b_ratio         REAL,
    ADD COLUMN delta_c_ratio         REAL;

-- ============================================================================
-- Round-4 fix M2: sentinel/domain CHECKs (mirror of 0013 Step 2 + Step 3b
-- minus the partial-NOT-NULL CHECKs that don't apply to quarantine).
--
-- Round-4 fix B4: DROP CONSTRAINT IF EXISTS prepended for idempotent
-- re-application (matches the 0046+0013 convention).
--
-- Why: producer-side malformation (e.g., delta_b_ratio = NaN) would pass
-- the quarantine INSERT but fail the canonical_events INSERT at release
-- time, leaving the row stuck in 'awaiting_decision'. Mirroring the
-- CHECKs rejects bad bytes at the quarantine boundary so the producer
-- gets a synchronous error and the operator never sees a stuck row.
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

ALTER TABLE audit_outcome_quarantine
    -- Enum-string domain checks.
    ADD CONSTRAINT audit_outcome_quarantine_reserved_strategy_chk
        CHECK (reserved_strategy IS NULL OR reserved_strategy IN ('A','B','C'))
        NOT VALID,
    ADD CONSTRAINT audit_outcome_quarantine_prediction_strategy_used_chk
        CHECK (prediction_strategy_used IS NULL
               OR prediction_strategy_used IN ('A','B','C'))
        NOT VALID,
    ADD CONSTRAINT audit_outcome_quarantine_prediction_policy_used_chk
        CHECK (prediction_policy_used IS NULL OR prediction_policy_used IN (
            'STRICT_CEILING','EMPIRICAL_RUN_CEILING',
            'ADAPTIVE_CEILING','SHADOW_ONLY'))
        NOT VALID,
    ADD CONSTRAINT audit_outcome_quarantine_tokenizer_tier_chk
        CHECK (tokenizer_tier IS NULL OR tokenizer_tier IN ('T1','T2','T3'))
        NOT VALID,
    ADD CONSTRAINT audit_outcome_quarantine_prediction_confidence_chk
        CHECK (prediction_confidence IS NULL
               OR (prediction_confidence >= 0.000
                   AND prediction_confidence <= 1.000))
        NOT VALID,
    ADD CONSTRAINT audit_outcome_quarantine_cold_start_layer_used_chk
        CHECK (cold_start_layer_used IS NULL
               OR cold_start_layer_used IN ('L1','L2','L3','L4'))
        NOT VALID,
    -- Sentinel discipline.
    ADD CONSTRAINT audit_outcome_quarantine_predicted_tokens_chk
        CHECK ((predicted_a_tokens IS NULL OR predicted_a_tokens >= 0)
           AND (predicted_b_tokens IS NULL OR predicted_b_tokens >= 0)
           AND (predicted_c_tokens IS NULL OR predicted_c_tokens >= 0))
        NOT VALID,
    ADD CONSTRAINT audit_outcome_quarantine_actual_tokens_chk
        CHECK ((actual_input_tokens IS NULL OR actual_input_tokens >= 0)
           AND (actual_output_tokens IS NULL OR actual_output_tokens >= 0))
        NOT VALID,
    ADD CONSTRAINT audit_outcome_quarantine_run_steps_chk
        CHECK ((run_predicted_remaining_steps IS NULL
                  OR run_predicted_remaining_steps >= -1)
           AND (run_steps_completed_so_far IS NULL
                  OR run_steps_completed_so_far >= 0))
        NOT VALID,
    ADD CONSTRAINT audit_outcome_quarantine_run_projection_chk
        CHECK (run_projection_at_decision_atomic IS NULL
               OR run_projection_at_decision_atomic >= 0)
        NOT VALID,
    ADD CONSTRAINT audit_outcome_quarantine_run_projection_int64_chk
        CHECK (run_projection_at_decision_atomic IS NULL
               OR run_projection_at_decision_atomic <= 9223372036854775807)
        NOT VALID,
    ADD CONSTRAINT audit_outcome_quarantine_prediction_sample_size_chk
        CHECK (prediction_sample_size IS NULL OR prediction_sample_size >= 0)
        NOT VALID,
    ADD CONSTRAINT audit_outcome_quarantine_delta_b_ratio_chk
        CHECK (delta_b_ratio IS NULL
               OR (delta_b_ratio >= 0.0 AND delta_b_ratio = delta_b_ratio))
        NOT VALID,
    ADD CONSTRAINT audit_outcome_quarantine_delta_c_ratio_chk
        CHECK (delta_c_ratio IS NULL
               OR (delta_c_ratio >= 0.0 AND delta_c_ratio = delta_c_ratio))
        NOT VALID,
    -- Sentinel-collision guards. Note: predicted_a_tokens has no
    -- corresponding CHECK here because the quarantine table holds
    -- audit.outcome events only — Strategy A reservation context lives
    -- on the decision row, not the outcome. The strategy-conditional
    -- B/C guards still apply because the outcome carries strategy used
    -- at decision time.
    ADD CONSTRAINT audit_outcome_quarantine_predicted_b_tokens_nonzero_chk
        CHECK (prediction_strategy_used IS DISTINCT FROM 'B'
               OR predicted_b_tokens IS NULL
               OR predicted_b_tokens > 0)
        NOT VALID,
    ADD CONSTRAINT audit_outcome_quarantine_predicted_c_tokens_nonzero_chk
        CHECK (prediction_strategy_used IS DISTINCT FROM 'C'
               OR predicted_c_tokens IS NULL
               OR predicted_c_tokens > 0)
        NOT VALID,
    -- Round-4 M3 mirror: cold_start_layer_used is decision-side; the
    -- quarantine table holds audit.outcome events so the column must be
    -- NULL on every row. event_type is already constrained to
    -- 'spendguard.audit.outcome' by the 0003 schema design but we keep
    -- the check explicit for grep-ability.
    ADD CONSTRAINT audit_outcome_quarantine_cold_start_layer_outcome_chk
        CHECK (cold_start_layer_used IS NULL)
        NOT VALID;

ALTER TABLE audit_outcome_quarantine VALIDATE CONSTRAINT audit_outcome_quarantine_reserved_strategy_chk;
ALTER TABLE audit_outcome_quarantine VALIDATE CONSTRAINT audit_outcome_quarantine_prediction_strategy_used_chk;
ALTER TABLE audit_outcome_quarantine VALIDATE CONSTRAINT audit_outcome_quarantine_prediction_policy_used_chk;
ALTER TABLE audit_outcome_quarantine VALIDATE CONSTRAINT audit_outcome_quarantine_tokenizer_tier_chk;
ALTER TABLE audit_outcome_quarantine VALIDATE CONSTRAINT audit_outcome_quarantine_prediction_confidence_chk;
ALTER TABLE audit_outcome_quarantine VALIDATE CONSTRAINT audit_outcome_quarantine_cold_start_layer_used_chk;
ALTER TABLE audit_outcome_quarantine VALIDATE CONSTRAINT audit_outcome_quarantine_predicted_tokens_chk;
ALTER TABLE audit_outcome_quarantine VALIDATE CONSTRAINT audit_outcome_quarantine_actual_tokens_chk;
ALTER TABLE audit_outcome_quarantine VALIDATE CONSTRAINT audit_outcome_quarantine_run_steps_chk;
ALTER TABLE audit_outcome_quarantine VALIDATE CONSTRAINT audit_outcome_quarantine_run_projection_chk;
ALTER TABLE audit_outcome_quarantine VALIDATE CONSTRAINT audit_outcome_quarantine_run_projection_int64_chk;
ALTER TABLE audit_outcome_quarantine VALIDATE CONSTRAINT audit_outcome_quarantine_prediction_sample_size_chk;
ALTER TABLE audit_outcome_quarantine VALIDATE CONSTRAINT audit_outcome_quarantine_delta_b_ratio_chk;
ALTER TABLE audit_outcome_quarantine VALIDATE CONSTRAINT audit_outcome_quarantine_delta_c_ratio_chk;
ALTER TABLE audit_outcome_quarantine VALIDATE CONSTRAINT audit_outcome_quarantine_predicted_b_tokens_nonzero_chk;
ALTER TABLE audit_outcome_quarantine VALIDATE CONSTRAINT audit_outcome_quarantine_predicted_c_tokens_nonzero_chk;
ALTER TABLE audit_outcome_quarantine VALIDATE CONSTRAINT audit_outcome_quarantine_cold_start_layer_outcome_chk;

-- ============================================================================
-- Round-3 fix B2: extend the quarantine immutability trigger to lock
-- the 18 new columns. Otherwise a tampering UPDATE could rewrite a
-- prediction column inside the quarantine while state still
-- 'awaiting_decision', then the release path would carry the tampered
-- value into canonical_events — bypassing both the canonical_events
-- immutability trigger AND the verify-chain mirror cross-check (since
-- the producer's CloudEvent signature covers the proto payload, not the
-- mirrored column).
-- ============================================================================

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
        -- === NEW prediction columns (round-3 B2) ===
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
    -- Allowed transitions: awaiting_decision -> released | orphaned.
    IF OLD.state = 'released' OR OLD.state = 'orphaned' THEN
        RAISE EXCEPTION 'audit_outcome_quarantine state is terminal'
            USING ERRCODE = '42P10';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

COMMENT ON COLUMN audit_outcome_quarantine.predicted_a_tokens IS
    'Round-3 B2: carries the audit.outcome''s tag-300+ prediction context through quarantine staging so release_quarantined_outcomes() can populate canonical_events with first-class columns matching the embedded CloudEvent payload.';
