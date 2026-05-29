-- Down-migration: reverse 0013_canonical_events_prediction_columns.sql
-- (round-2 fix m2).
--
-- Apply AFTER ledger-side down migrations per SLICE_01 §11. canonical_events
-- has no FK from outside this DB so order between 0013 and 0014 down-migs
-- is independent.

BEGIN;

ALTER TABLE canonical_events
    DROP CONSTRAINT IF EXISTS canonical_events_reserved_strategy_chk,
    DROP CONSTRAINT IF EXISTS canonical_events_prediction_strategy_used_chk,
    DROP CONSTRAINT IF EXISTS canonical_events_prediction_policy_used_chk,
    DROP CONSTRAINT IF EXISTS canonical_events_tokenizer_tier_chk,
    DROP CONSTRAINT IF EXISTS canonical_events_prediction_confidence_chk,
    DROP CONSTRAINT IF EXISTS canonical_events_cold_start_layer_used_chk,
    DROP CONSTRAINT IF EXISTS canonical_events_predicted_tokens_chk,
    DROP CONSTRAINT IF EXISTS canonical_events_actual_tokens_chk,
    DROP CONSTRAINT IF EXISTS canonical_events_run_steps_chk,
    DROP CONSTRAINT IF EXISTS canonical_events_run_projection_chk,
    DROP CONSTRAINT IF EXISTS canonical_events_run_projection_int64_chk,
    DROP CONSTRAINT IF EXISTS canonical_events_prediction_sample_size_chk,
    DROP CONSTRAINT IF EXISTS canonical_events_delta_b_ratio_chk,
    DROP CONSTRAINT IF EXISTS canonical_events_delta_c_ratio_chk,
    DROP CONSTRAINT IF EXISTS canonical_events_decision_required_cols_chk,
    DROP CONSTRAINT IF EXISTS canonical_events_outcome_required_cols_chk;

DROP INDEX IF EXISTS canonical_events_calibration_idx;
DROP INDEX IF EXISTS canonical_events_tier_idx;
DROP INDEX IF EXISTS canonical_events_outcome_calibration_idx;

DROP TABLE IF EXISTS canonical_events_2026_10;
DROP TABLE IF EXISTS canonical_events_2026_09;
DROP TABLE IF EXISTS canonical_events_2026_08;

ALTER TABLE canonical_events
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
