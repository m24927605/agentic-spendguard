-- Audit chain prediction extension (SLICE 01 — schema additions).
--
-- Spec: docs/audit-chain-prediction-extension-v1alpha1.md §2 + §4.1
-- Slice: docs/slices/SLICE_01_canonical_events_migration.md
--
-- 18 new nullable columns on audit_outbox:
--   * 11 decision-side prediction columns (§2.1)
--   * 3 run-level projection columns (§2.2)
--   * 4 commit-side actual columns (§2.3)
--
-- All ADD COLUMN with implicit NULL default. No backfill — existing rows
-- stay NULL forever (proto3 default-encoding semantics keep their
-- producer_signature valid; see §7 of the spec).
--
-- ADD COLUMN nullable is a metadata-only operation on Postgres 11+ — no
-- row rewrite even on partitioned audit_outbox. The migration is
-- effectively instantaneous regardless of partition count.
--
-- Producer code that writes these columns lands in SLICE_06+ (sidecar /
-- webhook_receiver / ttl_sweeper / ledger invoice_reconcile mirror); this
-- migration only adds the schema substrate, the immutability-trigger
-- update follows in 0047, and the tokenizer_versions FK target table is
-- created in 0048.

BEGIN;

ALTER TABLE audit_outbox
    -- === Decision-side prediction columns (11 total per §2.1) ===
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

    -- === Run-level projection columns (3 total per §2.2) ===
    ADD COLUMN run_projection_at_decision_atomic NUMERIC(38,0),
    ADD COLUMN run_predicted_remaining_steps     INT,
    ADD COLUMN run_steps_completed_so_far        INT,

    -- === Commit-side actual columns (4 total per §2.3) ===
    ADD COLUMN actual_input_tokens   INT,
    ADD COLUMN actual_output_tokens  INT,
    ADD COLUMN delta_b_ratio         REAL,
    ADD COLUMN delta_c_ratio         REAL;

-- ============================================================================
-- CHECK constraints (per spec §4.1 verbatim).
--
-- Each constraint is NOT VALID-able but we declare them eagerly: the table
-- has no rows that violate them (all NULL on legacy rows; new rows must
-- comply going forward). On the partitioned parent table the constraint
-- applies to every child partition automatically.
-- ============================================================================

ALTER TABLE audit_outbox
    ADD CONSTRAINT audit_outbox_reserved_strategy_chk
        CHECK (reserved_strategy IS NULL OR reserved_strategy IN ('A','B','C')),
    ADD CONSTRAINT audit_outbox_prediction_strategy_used_chk
        CHECK (prediction_strategy_used IS NULL
               OR prediction_strategy_used IN ('A','B','C')),
    ADD CONSTRAINT audit_outbox_prediction_policy_used_chk
        CHECK (prediction_policy_used IS NULL OR prediction_policy_used IN (
            'STRICT_CEILING','EMPIRICAL_RUN_CEILING',
            'ADAPTIVE_CEILING','SHADOW_ONLY')),
    ADD CONSTRAINT audit_outbox_tokenizer_tier_chk
        CHECK (tokenizer_tier IS NULL OR tokenizer_tier IN ('T1','T2','T3')),
    ADD CONSTRAINT audit_outbox_prediction_confidence_chk
        CHECK (prediction_confidence IS NULL
               OR (prediction_confidence >= 0.0
                   AND prediction_confidence <= 1.0)),
    ADD CONSTRAINT audit_outbox_cold_start_layer_used_chk
        CHECK (cold_start_layer_used IS NULL
               OR cold_start_layer_used IN ('L1','L2','L3','L4'));

-- ============================================================================
-- Calibration-report indexes (per spec §4.1).
--
-- Both partial-indexes on (event_type = 'spendguard.audit.decision') to
-- keep them small — outcome rows do not populate these columns. Both are
-- defined on the partitioned parent; Postgres applies them per-partition.
-- ============================================================================

CREATE INDEX audit_outbox_calibration_idx
    ON audit_outbox (recorded_month, tenant_id,
                     prediction_strategy_used, prediction_policy_used)
    WHERE event_type = 'spendguard.audit.decision';

CREATE INDEX audit_outbox_tier_idx
    ON audit_outbox (recorded_month, tenant_id, tokenizer_tier)
    WHERE event_type = 'spendguard.audit.decision';

COMMENT ON COLUMN audit_outbox.predicted_a_tokens IS
    'Strategy A token ceiling at decision time (always populated on .decision events). Per audit-chain-prediction-extension-v1alpha1.md §2.1.';
COMMENT ON COLUMN audit_outbox.predicted_b_tokens IS
    'Strategy B (empirical) prediction; NULL when sample bucket < 30. §2.1.';
COMMENT ON COLUMN audit_outbox.predicted_c_tokens IS
    'Strategy C (customer plugin) prediction; NULL when plugin unconfigured / failed / fallback. §2.1.';
COMMENT ON COLUMN audit_outbox.reserved_strategy IS
    'Strategy actually used to size the reservation (A/B/C). §2.1.';
COMMENT ON COLUMN audit_outbox.prediction_strategy_used IS
    'Strategy the predictor recommended (may differ from reserved_strategy under STRICT_CEILING). §2.1.';
COMMENT ON COLUMN audit_outbox.prediction_policy_used IS
    'Contract policy class governing this decision (STRICT_CEILING / EMPIRICAL_RUN_CEILING / ADAPTIVE_CEILING / SHADOW_ONLY). §2.1.';
COMMENT ON COLUMN audit_outbox.tokenizer_tier IS
    'Tokenizer tier that produced the input token count (T1/T2/T3). §2.1.';
COMMENT ON COLUMN audit_outbox.tokenizer_version_id IS
    'FK to tokenizer_versions(tokenizer_version_id). NULL on Tier 3 fallback. §2.1.';
COMMENT ON COLUMN audit_outbox.prediction_confidence IS
    'Predictor confidence for Strategy B/C (0.0-1.0); NULL for Strategy A. §2.1.';
COMMENT ON COLUMN audit_outbox.prediction_sample_size IS
    'Sample count behind Strategy B/C; NULL for cold-start / A. §2.1.';
COMMENT ON COLUMN audit_outbox.cold_start_layer_used IS
    'Cold-start fallback layer (L1-L4) when B/C fell through; NULL when warm. Promoted from metadata to first-class per §2.4 reviewer note. §2.1.';
COMMENT ON COLUMN audit_outbox.run_projection_at_decision_atomic IS
    'Per-run projected cumulative cost (NUMERIC(38,0)) at decision time. §2.2.';
COMMENT ON COLUMN audit_outbox.run_predicted_remaining_steps IS
    'Predicted remaining run steps; NULL when run_cost_projector unreachable. §2.2.';
COMMENT ON COLUMN audit_outbox.run_steps_completed_so_far IS
    'Step counter from sidecar in-process state cache. §2.2.';
COMMENT ON COLUMN audit_outbox.actual_input_tokens IS
    'Provider-reported input tokens at commit_estimated / provider_report time. §2.3.';
COMMENT ON COLUMN audit_outbox.actual_output_tokens IS
    'Provider-reported output tokens at commit_estimated / provider_report time. §2.3.';
COMMENT ON COLUMN audit_outbox.delta_b_ratio IS
    'actual_output_tokens / predicted_b_tokens; NULL when prediction B was null at decision time. §2.3.';
COMMENT ON COLUMN audit_outbox.delta_c_ratio IS
    'actual_output_tokens / predicted_c_tokens; NULL when prediction C was null at decision time. §2.3.';

COMMIT;
