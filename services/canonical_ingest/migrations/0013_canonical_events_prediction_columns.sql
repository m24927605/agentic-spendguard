-- Canonical events mirror of the audit_outbox prediction columns added
-- in services/ledger/migrations/0046_audit_outbox_prediction_columns.sql.
--
-- Spec ancestor: docs/audit-chain-prediction-extension-v1alpha1.md §6 +
-- SLICE_01 §2 ("Add columns to canonical_events table — mirror schema;
-- replicated via outbox_forwarder unchanged").
--
-- Why mirror to canonical_events:
--   * calibration-report CLI runs SQL aggregations against canonical_events
--     (the storage-class-bound canonical store; audit_outbox is the
--     ledger-side outbox the forwarder replicates from).
--   * cross-storage consistency check (spec §11.2 + verify-chain
--     --check-prediction-mirror) compares column values between
--     audit_outbox and canonical_events; symmetric schema is required.
--   * payload_json continues to carry the CloudEvent envelope including
--     proto fields 300-317, so the canonical signature still covers them
--     end-to-end; the first-class columns are SQL accelerators.
--
-- All ADD COLUMN nullable. No backfill — legacy rows stay NULL. The
-- existing canonical_events_no_update_delete trigger (BEFORE UPDATE OR
-- DELETE; round-2 fix m1 — name corrected from prior round-1 comment
-- "canonical_events_immutability") already rejects ALL mutations on
-- canonical_events, so no trigger update is needed here — the table is
-- strictly append-only at the application level. Adding NULLable columns
-- is a metadata-only operation on the partitioned parent; child
-- partitions inherit instantaneously.
--
-- The outbox_forwarder path is unchanged: it reads audit_outbox columns
-- and the CloudEvent payload, pushes via gRPC AppendEvents, and
-- canonical_ingest's persistence layer (services/canonical_ingest/src/
-- persistence/append.rs) populates the first-class mirror columns
-- starting in SLICE_06 along with the proto field mirror in producers.
--
-- Cross-DB deployment ordering (round-2 fix M16): this migration MUST
-- run AFTER services/ledger/migrations/0046+0048 because the spec §6
-- mirror invariant requires the ledger side to be schema-ready first.
-- The Helm migrations.yaml apply loop enforces this by processing the
-- ledger glob before the canonical glob.
--
-- Migration runner wrapping convention: no explicit BEGIN/COMMIT — the
-- runner wraps each .sql in its own transaction (round-2 fix m3,
-- matches the 12 pre-existing canonical_ingest migrations 0000-0012).

-- ============================================================================
-- Step 1: Add the 18 new columns. INT → BIGINT for token-count columns
-- per round-2 finding M4. prediction_confidence NUMERIC(4,3) per M12.
-- ============================================================================

ALTER TABLE canonical_events
    -- === Decision-side prediction columns (11 total per spec §2.1) ===
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

    -- === Run-level projection columns (3 total per spec §2.2) ===
    ADD COLUMN run_projection_at_decision_atomic NUMERIC(38,0),
    ADD COLUMN run_predicted_remaining_steps     INT,
    ADD COLUMN run_steps_completed_so_far        BIGINT,

    -- === Commit-side actual columns (4 total per spec §2.3) ===
    ADD COLUMN actual_input_tokens   BIGINT,
    ADD COLUMN actual_output_tokens  BIGINT,
    ADD COLUMN delta_b_ratio         REAL,
    ADD COLUMN delta_c_ratio         REAL;

-- ============================================================================
-- Step 2: Domain CHECK constraints — mirror of ledger side
-- (round-2 fixes M2 / M3 / M5 / M6 / M12).
-- ============================================================================

ALTER TABLE canonical_events
    ADD CONSTRAINT canonical_events_reserved_strategy_chk
        CHECK (reserved_strategy IS NULL OR reserved_strategy IN ('A','B','C'))
        NOT VALID,
    ADD CONSTRAINT canonical_events_prediction_strategy_used_chk
        CHECK (prediction_strategy_used IS NULL
               OR prediction_strategy_used IN ('A','B','C'))
        NOT VALID,
    ADD CONSTRAINT canonical_events_prediction_policy_used_chk
        CHECK (prediction_policy_used IS NULL OR prediction_policy_used IN (
            'STRICT_CEILING','EMPIRICAL_RUN_CEILING',
            'ADAPTIVE_CEILING','SHADOW_ONLY'))
        NOT VALID,
    ADD CONSTRAINT canonical_events_tokenizer_tier_chk
        CHECK (tokenizer_tier IS NULL OR tokenizer_tier IN ('T1','T2','T3'))
        NOT VALID,
    ADD CONSTRAINT canonical_events_prediction_confidence_chk
        CHECK (prediction_confidence IS NULL
               OR (prediction_confidence >= 0.000
                   AND prediction_confidence <= 1.000))
        NOT VALID,
    ADD CONSTRAINT canonical_events_cold_start_layer_used_chk
        CHECK (cold_start_layer_used IS NULL
               OR cold_start_layer_used IN ('L1','L2','L3','L4'))
        NOT VALID,

    -- === Sentinel discipline mirror (round-2 fix M3) ===
    ADD CONSTRAINT canonical_events_predicted_tokens_chk
        CHECK ((predicted_a_tokens IS NULL OR predicted_a_tokens >= 0)
           AND (predicted_b_tokens IS NULL OR predicted_b_tokens >= 0)
           AND (predicted_c_tokens IS NULL OR predicted_c_tokens >= 0))
        NOT VALID,
    ADD CONSTRAINT canonical_events_actual_tokens_chk
        CHECK ((actual_input_tokens IS NULL OR actual_input_tokens >= 0)
           AND (actual_output_tokens IS NULL OR actual_output_tokens >= 0))
        NOT VALID,
    ADD CONSTRAINT canonical_events_run_steps_chk
        CHECK ((run_predicted_remaining_steps IS NULL
                  OR run_predicted_remaining_steps >= -1)
           AND (run_steps_completed_so_far IS NULL
                  OR run_steps_completed_so_far >= 0))
        NOT VALID,
    ADD CONSTRAINT canonical_events_run_projection_chk
        CHECK (run_projection_at_decision_atomic IS NULL
               OR run_projection_at_decision_atomic >= 0)
        NOT VALID,
    -- Round-2 fix M5 mirror: int64 overflow guard for proto field 311.
    ADD CONSTRAINT canonical_events_run_projection_int64_chk
        CHECK (run_projection_at_decision_atomic IS NULL
               OR run_projection_at_decision_atomic <= 9223372036854775807)
        NOT VALID,
    ADD CONSTRAINT canonical_events_prediction_sample_size_chk
        CHECK (prediction_sample_size IS NULL OR prediction_sample_size >= 0)
        NOT VALID,
    ADD CONSTRAINT canonical_events_delta_b_ratio_chk
        CHECK (delta_b_ratio IS NULL
               OR (delta_b_ratio >= 0.0 AND delta_b_ratio = delta_b_ratio))
        NOT VALID,
    ADD CONSTRAINT canonical_events_delta_c_ratio_chk
        CHECK (delta_c_ratio IS NULL
               OR (delta_c_ratio >= 0.0 AND delta_c_ratio = delta_c_ratio))
        NOT VALID;

ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_reserved_strategy_chk;
ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_prediction_strategy_used_chk;
ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_prediction_policy_used_chk;
ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_tokenizer_tier_chk;
ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_prediction_confidence_chk;
ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_cold_start_layer_used_chk;
ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_predicted_tokens_chk;
ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_actual_tokens_chk;
ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_run_steps_chk;
ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_run_projection_chk;
ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_run_projection_int64_chk;
ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_prediction_sample_size_chk;
ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_delta_b_ratio_chk;
ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_delta_c_ratio_chk;

-- ============================================================================
-- Step 3: Partial NOT-NULL via CHECK on event_type (round-2 fix M2
-- mirror of ledger side, per spec §2.1-§2.3 "Nullable: NO" columns).
--
-- The same event_time < '2027-01-01' cutoff applies (round-3 fix B5:
-- extended from 2026-07-01 calendar bomb) — SLICE_06 producers begin
-- populating these columns; SLICE_06 deployment plan MUST land before
-- 2027-01-01.
--
-- Round-4 fix B4: DROP CONSTRAINT IF EXISTS prepended so re-application
-- against a database that previously ran the 2026-07-01 form replaces
-- the CHECK body cleanly. Same-name + different body would error 42710
-- or silently keep the old body. Drop-then-add inside the same
-- migration-runner transaction keeps observers in a consistent state.
-- ============================================================================

ALTER TABLE canonical_events
    DROP CONSTRAINT IF EXISTS canonical_events_decision_required_cols_chk,
    DROP CONSTRAINT IF EXISTS canonical_events_outcome_required_cols_chk;

ALTER TABLE canonical_events
    ADD CONSTRAINT canonical_events_decision_required_cols_chk
        CHECK (event_type <> 'spendguard.audit.decision'
               OR event_time < '2027-01-01 00:00:00+00'::timestamptz
               OR (predicted_a_tokens IS NOT NULL
                   AND reserved_strategy IS NOT NULL
                   AND prediction_strategy_used IS NOT NULL
                   AND prediction_policy_used IS NOT NULL
                   AND tokenizer_tier IS NOT NULL
                   AND run_projection_at_decision_atomic IS NOT NULL
                   AND run_steps_completed_so_far IS NOT NULL))
        NOT VALID,
    ADD CONSTRAINT canonical_events_outcome_required_cols_chk
        CHECK (event_type <> 'spendguard.audit.outcome'
               OR event_time < '2027-01-01 00:00:00+00'::timestamptz
               OR (actual_input_tokens IS NOT NULL
                   AND actual_output_tokens IS NOT NULL))
        NOT VALID;

ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_decision_required_cols_chk;
ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_outcome_required_cols_chk;

-- ============================================================================
-- Step 3a: Outcome-side cold_start_layer_used must be NULL (round-4 fix M3
-- mirror of ledger 0046). Calibration-report invariant: outcome rows do
-- not carry the cold-start-fallback-layer column populated.
-- ============================================================================

ALTER TABLE canonical_events
    DROP CONSTRAINT IF EXISTS canonical_events_cold_start_layer_outcome_chk;

ALTER TABLE canonical_events
    ADD CONSTRAINT canonical_events_cold_start_layer_outcome_chk
        CHECK (event_type <> 'spendguard.audit.outcome'
               OR cold_start_layer_used IS NULL)
        NOT VALID;

ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_cold_start_layer_outcome_chk;

-- ============================================================================
-- Step 3b: Sentinel-collision guards (round-3 fix M13 mirror; round-4
-- fix B4 idempotent re-application).
--
-- Mirror of 0046's Step 3b. Without these CHECKs a row with
-- prediction_strategy_used = 'B' AND predicted_b_tokens = 0 would be
-- indistinguishable from "Strategy B was null at decision time" once the
-- proto3 sentinel mapping rewrites column NULL ↔ wire 0. See
-- crates/spendguard-prediction-mirror/src/lib.rs preamble for the
-- producer-side precondition.
-- ============================================================================

ALTER TABLE canonical_events
    DROP CONSTRAINT IF EXISTS canonical_events_predicted_a_tokens_nonzero_chk,
    DROP CONSTRAINT IF EXISTS canonical_events_predicted_b_tokens_nonzero_chk,
    DROP CONSTRAINT IF EXISTS canonical_events_predicted_c_tokens_nonzero_chk;

ALTER TABLE canonical_events
    ADD CONSTRAINT canonical_events_predicted_a_tokens_nonzero_chk
        CHECK (event_type <> 'spendguard.audit.decision'
               OR event_time < '2027-01-01 00:00:00+00'::timestamptz
               OR predicted_a_tokens IS NULL
               OR predicted_a_tokens > 0)
        NOT VALID,
    ADD CONSTRAINT canonical_events_predicted_b_tokens_nonzero_chk
        CHECK (prediction_strategy_used IS DISTINCT FROM 'B'
               OR predicted_b_tokens IS NULL
               OR predicted_b_tokens > 0)
        NOT VALID,
    ADD CONSTRAINT canonical_events_predicted_c_tokens_nonzero_chk
        CHECK (prediction_strategy_used IS DISTINCT FROM 'C'
               OR predicted_c_tokens IS NULL
               OR predicted_c_tokens > 0)
        NOT VALID;

ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_predicted_a_tokens_nonzero_chk;
ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_predicted_b_tokens_nonzero_chk;
ALTER TABLE canonical_events VALIDATE CONSTRAINT canonical_events_predicted_c_tokens_nonzero_chk;

-- ============================================================================
-- Step 4: Calibration-report indexes (round-2 fix M9 — tenant_id-first
-- composite key + outcome-side covering index).
--
-- canonical_events.event_type is TEXT (no enum), and these are partial
-- indexes scoped to audit.decision rows for size economy.
-- ============================================================================

CREATE INDEX canonical_events_calibration_idx
    ON canonical_events (tenant_id, recorded_month,
                         prediction_strategy_used, prediction_policy_used)
    WHERE event_type = 'spendguard.audit.decision';

CREATE INDEX canonical_events_tier_idx
    ON canonical_events (tenant_id, recorded_month, tokenizer_tier)
    WHERE event_type = 'spendguard.audit.decision';

-- Round-4 fix M14 mirror: WHERE clause relaxed to just
-- `event_type = '...outcome'`. See ledger-side 0046 for rationale —
-- the partial-index planner only fires when the query WHERE implies
-- the index predicate. Forcing calibration-report queries to include
-- the IS NOT NULL clause was a planner foot-gun. DROP-then-CREATE
-- handles re-application against the round-3 form.
DROP INDEX IF EXISTS canonical_events_outcome_calibration_idx;
CREATE INDEX canonical_events_outcome_calibration_idx
    ON canonical_events (tenant_id, recorded_month, prediction_strategy_used)
    INCLUDE (delta_b_ratio, delta_c_ratio, actual_output_tokens)
    WHERE event_type = 'spendguard.audit.outcome';

-- ============================================================================
-- Step 5: Pre-create future partitions through 2026-10 (round-2 fix
-- M14 mirror). The canonical_events table is partitioned by
-- recorded_month (same convention as ledger audit_outbox); pre-create
-- partitions to match the ledger side so the outbox_forwarder doesn't
-- fall through to canonical_events_default mid-month.
-- ============================================================================

CREATE TABLE canonical_events_2026_08 PARTITION OF canonical_events
    FOR VALUES FROM ('2026-08-01') TO ('2026-09-01');
CREATE TABLE canonical_events_2026_09 PARTITION OF canonical_events
    FOR VALUES FROM ('2026-09-01') TO ('2026-10-01');
CREATE TABLE canonical_events_2026_10 PARTITION OF canonical_events
    FOR VALUES FROM ('2026-10-01') TO ('2026-11-01');

-- ============================================================================
-- Column comments — verbose for the same SLICE_06+ reason as the ledger
-- side.
-- ============================================================================

COMMENT ON COLUMN canonical_events.predicted_a_tokens IS
    'Mirror of audit_outbox.predicted_a_tokens; SQL accelerator for calibration-report. BIGINT per round-2 fix M4. Per audit-chain-prediction-extension-v1alpha1.md §2.1 + §6.';
COMMENT ON COLUMN canonical_events.predicted_b_tokens IS
    'Mirror of audit_outbox.predicted_b_tokens.';
COMMENT ON COLUMN canonical_events.predicted_c_tokens IS
    'Mirror of audit_outbox.predicted_c_tokens.';
COMMENT ON COLUMN canonical_events.reserved_strategy IS
    'Mirror of audit_outbox.reserved_strategy.';
COMMENT ON COLUMN canonical_events.prediction_strategy_used IS
    'Mirror of audit_outbox.prediction_strategy_used.';
COMMENT ON COLUMN canonical_events.prediction_policy_used IS
    'Mirror of audit_outbox.prediction_policy_used.';
COMMENT ON COLUMN canonical_events.tokenizer_tier IS
    'Mirror of audit_outbox.tokenizer_tier.';
COMMENT ON COLUMN canonical_events.tokenizer_version_id IS
    'Mirror of audit_outbox.tokenizer_version_id. NOTE: no FK declared here because the tokenizer_versions registry lives in the ledger database; cross-database referential integrity is enforced by the producer-side write path (canonical_ingest persistence layer in SLICE 06+).';
COMMENT ON COLUMN canonical_events.prediction_confidence IS
    'Mirror of audit_outbox.prediction_confidence. NUMERIC(4,3) for deterministic AVG (round-2 fix M12).';
COMMENT ON COLUMN canonical_events.prediction_sample_size IS
    'Mirror of audit_outbox.prediction_sample_size. BIGINT per round-2 fix M4.';
COMMENT ON COLUMN canonical_events.cold_start_layer_used IS
    'Mirror of audit_outbox.cold_start_layer_used.';
COMMENT ON COLUMN canonical_events.run_projection_at_decision_atomic IS
    'Mirror of audit_outbox.run_projection_at_decision_atomic. Constrained to <= int64 max (round-2 fix M5) so the CloudEvent proto int64 mirror at tag 311 round-trips losslessly.';
COMMENT ON COLUMN canonical_events.run_predicted_remaining_steps IS
    'Mirror of audit_outbox.run_predicted_remaining_steps.';
COMMENT ON COLUMN canonical_events.run_steps_completed_so_far IS
    'Mirror of audit_outbox.run_steps_completed_so_far. BIGINT per round-2 fix M4.';
COMMENT ON COLUMN canonical_events.actual_input_tokens IS
    'Mirror of audit_outbox.actual_input_tokens.';
COMMENT ON COLUMN canonical_events.actual_output_tokens IS
    'Mirror of audit_outbox.actual_output_tokens.';
COMMENT ON COLUMN canonical_events.delta_b_ratio IS
    'Mirror of audit_outbox.delta_b_ratio. CHECK guards NaN per IEEE 754 (round-2 fix M3).';
COMMENT ON COLUMN canonical_events.delta_c_ratio IS
    'Mirror of audit_outbox.delta_c_ratio. CHECK guards NaN per IEEE 754 (round-2 fix M3).';
