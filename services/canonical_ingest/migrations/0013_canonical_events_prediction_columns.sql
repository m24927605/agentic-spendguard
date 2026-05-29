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
-- All ADD COLUMN nullable. No backfill — legacy rows stay NULL. Existing
-- canonical_events_immutability trigger (BEFORE UPDATE OR DELETE) already
-- rejects ALL mutations on canonical_events, so no trigger update is
-- needed here — the table is strictly append-only at the application
-- level. Adding NULLable columns is a metadata-only operation on the
-- partitioned parent; child partitions inherit instantaneously.
--
-- The outbox_forwarder path is unchanged: it reads audit_outbox columns
-- and the CloudEvent payload, pushes via gRPC AppendEvents, and
-- canonical_ingest's persistence layer (services/canonical_ingest/src/
-- persistence/append.rs) populates the first-class mirror columns
-- starting in SLICE_06 along with the proto field mirror in producers.

BEGIN;

ALTER TABLE canonical_events
    -- === Decision-side prediction columns (11 total per spec §2.1) ===
    ADD COLUMN predicted_a_tokens         INT,
    ADD COLUMN predicted_b_tokens         INT,
    ADD COLUMN predicted_c_tokens         INT,
    ADD COLUMN reserved_strategy          TEXT,
    ADD COLUMN prediction_strategy_used   TEXT,
    ADD COLUMN prediction_policy_used     TEXT,
    ADD COLUMN tokenizer_tier             TEXT,
    ADD COLUMN tokenizer_version_id       UUID,
    ADD COLUMN prediction_confidence      REAL,
    ADD COLUMN prediction_sample_size     INT,
    ADD COLUMN cold_start_layer_used      TEXT,

    -- === Run-level projection columns (3 total per spec §2.2) ===
    ADD COLUMN run_projection_at_decision_atomic NUMERIC(38,0),
    ADD COLUMN run_predicted_remaining_steps     INT,
    ADD COLUMN run_steps_completed_so_far        INT,

    -- === Commit-side actual columns (4 total per spec §2.3) ===
    ADD COLUMN actual_input_tokens   INT,
    ADD COLUMN actual_output_tokens  INT,
    ADD COLUMN delta_b_ratio         REAL,
    ADD COLUMN delta_c_ratio         REAL;

-- ============================================================================
-- CHECK constraints — same domain rules as the ledger side.
-- ============================================================================

ALTER TABLE canonical_events
    ADD CONSTRAINT canonical_events_reserved_strategy_chk
        CHECK (reserved_strategy IS NULL OR reserved_strategy IN ('A','B','C')),
    ADD CONSTRAINT canonical_events_prediction_strategy_used_chk
        CHECK (prediction_strategy_used IS NULL
               OR prediction_strategy_used IN ('A','B','C')),
    ADD CONSTRAINT canonical_events_prediction_policy_used_chk
        CHECK (prediction_policy_used IS NULL OR prediction_policy_used IN (
            'STRICT_CEILING','EMPIRICAL_RUN_CEILING',
            'ADAPTIVE_CEILING','SHADOW_ONLY')),
    ADD CONSTRAINT canonical_events_tokenizer_tier_chk
        CHECK (tokenizer_tier IS NULL OR tokenizer_tier IN ('T1','T2','T3')),
    ADD CONSTRAINT canonical_events_prediction_confidence_chk
        CHECK (prediction_confidence IS NULL
               OR (prediction_confidence >= 0.0
                   AND prediction_confidence <= 1.0)),
    ADD CONSTRAINT canonical_events_cold_start_layer_used_chk
        CHECK (cold_start_layer_used IS NULL
               OR cold_start_layer_used IN ('L1','L2','L3','L4'));

-- ============================================================================
-- Calibration-report indexes (mirror of ledger side, same predicates).
--
-- canonical_events.event_type is TEXT (no enum), and these are partial
-- indexes scoped to audit.decision rows for size economy.
-- ============================================================================

CREATE INDEX canonical_events_calibration_idx
    ON canonical_events (recorded_month, tenant_id,
                         prediction_strategy_used, prediction_policy_used)
    WHERE event_type = 'spendguard.audit.decision';

CREATE INDEX canonical_events_tier_idx
    ON canonical_events (recorded_month, tenant_id, tokenizer_tier)
    WHERE event_type = 'spendguard.audit.decision';

COMMENT ON COLUMN canonical_events.predicted_a_tokens IS
    'Mirror of audit_outbox.predicted_a_tokens; SQL accelerator for calibration-report. Per audit-chain-prediction-extension-v1alpha1.md §2.1 + §6.';
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
    'Mirror of audit_outbox.prediction_confidence.';
COMMENT ON COLUMN canonical_events.prediction_sample_size IS
    'Mirror of audit_outbox.prediction_sample_size.';
COMMENT ON COLUMN canonical_events.cold_start_layer_used IS
    'Mirror of audit_outbox.cold_start_layer_used.';
COMMENT ON COLUMN canonical_events.run_projection_at_decision_atomic IS
    'Mirror of audit_outbox.run_projection_at_decision_atomic.';
COMMENT ON COLUMN canonical_events.run_predicted_remaining_steps IS
    'Mirror of audit_outbox.run_predicted_remaining_steps.';
COMMENT ON COLUMN canonical_events.run_steps_completed_so_far IS
    'Mirror of audit_outbox.run_steps_completed_so_far.';
COMMENT ON COLUMN canonical_events.actual_input_tokens IS
    'Mirror of audit_outbox.actual_input_tokens.';
COMMENT ON COLUMN canonical_events.actual_output_tokens IS
    'Mirror of audit_outbox.actual_output_tokens.';
COMMENT ON COLUMN canonical_events.delta_b_ratio IS
    'Mirror of audit_outbox.delta_b_ratio.';
COMMENT ON COLUMN canonical_events.delta_c_ratio IS
    'Mirror of audit_outbox.delta_c_ratio.';

COMMIT;
