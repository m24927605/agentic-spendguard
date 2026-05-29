-- Down-migration: reverse 0013_canonical_events_prediction_columns.sql
-- (round-2 fix m2; round-3 fixes M4 + M10; round-4 fixes M4 + B6).
--
-- Apply AFTER 0015_down (quarantine drops are independent but per slice
-- §11 0015_down lands first); canonical_events has no FK from outside
-- this DB so the only dependency is on the partition/column order.
-- Apply BEFORE ledger-side down migrations per slice §11.
--
-- Round-3 fix M4 + round-4 fix M4: per-file destructive-down guard
-- (spendguard.allow_destructive_down_0013) — see slice §11 for the exact
-- SET form (round-5 N12-A: SET, not SET LOCAL, because the migration
-- runner autocommits each statement and SET LOCAL would die at the
-- DO block's commit boundary before the next statement runs).
-- Round-3 fix M10: no explicit BEGIN/COMMIT (matches up-migration).
-- Round-4 fix B6 + round-5 fix N12-B: ACCESS EXCLUSIVE LOCK + COUNT +
-- DROP collapsed into ONE DO $$ block per partition so the three
-- operations share a single implicit transaction under the autocommit
-- runner.

DO $$
BEGIN
    IF current_setting('spendguard.allow_destructive_down_0013', true) IS DISTINCT FROM 'on' THEN
        RAISE EXCEPTION 'destructive down-migration 0013 requires `SET spendguard.allow_destructive_down_0013 = ''on''` first (session-scoped; runner autocommits so SET LOCAL would die at the commit boundary)';
    END IF;
    RAISE NOTICE 'DESTRUCTIVE down-migration 0013 proceeding (caller: %)', current_user;
END $$;

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
    DROP CONSTRAINT IF EXISTS canonical_events_outcome_required_cols_chk,
    -- Round-3 M13 mirror.
    DROP CONSTRAINT IF EXISTS canonical_events_predicted_a_tokens_nonzero_chk,
    DROP CONSTRAINT IF EXISTS canonical_events_predicted_b_tokens_nonzero_chk,
    DROP CONSTRAINT IF EXISTS canonical_events_predicted_c_tokens_nonzero_chk,
    -- Round-4 M3 mirror.
    DROP CONSTRAINT IF EXISTS canonical_events_cold_start_layer_outcome_chk;

DROP INDEX IF EXISTS canonical_events_calibration_idx;
DROP INDEX IF EXISTS canonical_events_tier_idx;
DROP INDEX IF EXISTS canonical_events_outcome_calibration_idx;

-- Round-3 fix M4 + round-4 fix B6 + round-5 fix N12-B: per-partition
-- row-count guard with ACCESS EXCLUSIVE LOCK + COUNT + EXECUTE 'DROP TABLE'
-- collapsed into ONE DO $$ block so the three operations share a single
-- implicit transaction even under the autocommit runner. EXECUTE is
-- required because DROP TABLE is not legal as a static statement inside
-- PL/pgSQL. The lock is held until END $$ commits, blocking all
-- concurrent reads + writes for the entire count → drop sequence.
DO $$
DECLARE
    rc BIGINT;
BEGIN
    LOCK TABLE canonical_events_2026_10 IN ACCESS EXCLUSIVE MODE;
    SELECT COUNT(*) INTO rc FROM canonical_events_2026_10;
    IF rc > 0 THEN
        RAISE EXCEPTION 'canonical_events_2026_10 has % rows; refusing to drop. Manual data migration required first.', rc;
    END IF;
    EXECUTE 'DROP TABLE IF EXISTS canonical_events_2026_10';
END $$;

DO $$
DECLARE
    rc BIGINT;
BEGIN
    LOCK TABLE canonical_events_2026_09 IN ACCESS EXCLUSIVE MODE;
    SELECT COUNT(*) INTO rc FROM canonical_events_2026_09;
    IF rc > 0 THEN
        RAISE EXCEPTION 'canonical_events_2026_09 has % rows; refusing to drop. Manual data migration required first.', rc;
    END IF;
    EXECUTE 'DROP TABLE IF EXISTS canonical_events_2026_09';
END $$;

DO $$
DECLARE
    rc BIGINT;
BEGIN
    LOCK TABLE canonical_events_2026_08 IN ACCESS EXCLUSIVE MODE;
    SELECT COUNT(*) INTO rc FROM canonical_events_2026_08;
    IF rc > 0 THEN
        RAISE EXCEPTION 'canonical_events_2026_08 has % rows; refusing to drop. Manual data migration required first.', rc;
    END IF;
    EXECUTE 'DROP TABLE IF EXISTS canonical_events_2026_08';
END $$;

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
