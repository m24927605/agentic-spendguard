-- Round-3 fix B2 acceptance test: outcome-before-decision quarantine +
-- release path carries the 18 prediction columns from quarantine row
-- through to canonical_events.
--
-- Spec: docs/audit-chain-prediction-extension-v1alpha1.md §11.2
--   (cross-storage consistency) + SLICE_01 §6 (mirror invariant).
--
-- ## How to run
--
--   psql "$PG_CANONICAL_URL" -v ON_ERROR_STOP=1 \
--        -f services/canonical_ingest/tests/migrations/test_quarantine_release_prediction_cols.sql
--
-- ## What it checks
--
--   1. Insert an audit.outcome into audit_outcome_quarantine with all 18
--      prediction columns populated (simulating SLICE_06 producer that
--      already knows the values when the outcome arrived before its
--      decision).
--   2. SELECT the row to confirm storage worked.
--   3. Simulate the release path: SELECT the 18 columns + INSERT into
--      canonical_events with the same values.
--   4. SELECT from canonical_events to confirm column values are
--      byte-identical to what was on quarantine.
--
-- Exit code 0 on PASS; non-zero on FAIL via ASSERT.

\set ON_ERROR_STOP on
\set VERBOSITY verbose

BEGIN;

-- Pre-arrange: the matching audit.decision must exist in
-- canonical_events_global_keys so the assert_audit_outcome_has_preceding_decision
-- trigger doesn't reject the test insert. Use a synthetic decision row.
-- Use a UUID that does NOT clash with any existing test data.
DO $$
BEGIN
    INSERT INTO canonical_events_global_keys (
        event_id, tenant_id, decision_id, event_type, recorded_month
    ) VALUES (
        '01999d80-0001-7000-8000-000000000010'::uuid,
        '00000000-0000-4000-8000-000000000010'::uuid,
        '00000000-0000-7000-8000-000000000020'::uuid,
        'spendguard.audit.decision',
        '2026-07-01'::date
    );
EXCEPTION
    WHEN unique_violation THEN
        -- Test re-run; fixture already exists.
        NULL;
END $$;

-- Insert a deterministic test schema_bundle so the canonical_events FK
-- doesn't reject the test INSERT. Uses a fixed UUID so re-runs are
-- idempotent (ON CONFLICT swallows the duplicate).
--
-- Round-4 fix M13: UUID + hash sanitised to obviously-fake patterns so
-- the fixture cannot accidentally collide with a production bundle row.
-- The hash is `deadbeef` repeated 8 times (32 bytes hex) — visibly
-- synthetic on grep and impossible to mistake for sha256 of any real
-- canonicalized proto bundle.
INSERT INTO schema_bundles (
    schema_bundle_id, schema_bundle_hash, canonical_schema_version,
    profile_versions, fetched_at
) VALUES (
    -- TEST FIXTURE; never matches a real bundle.
    '00000000-0000-1111-2222-333333333333'::uuid,
    '\xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef'::bytea,
    'test-fixture-spendguard.v1alpha1',
    '{}'::jsonb,
    clock_timestamp()
)
ON CONFLICT (schema_bundle_id) DO NOTHING;

-- ============================================================================
-- Step 1: Quarantine a synthetic audit.outcome with all 18 prediction
-- columns populated.
-- ============================================================================
INSERT INTO audit_outcome_quarantine (
    quarantine_id, event_id, tenant_id, decision_id,
    storage_class, producer_id, producer_sequence,
    producer_signature, signing_key_id,
    schema_bundle_id, schema_bundle_hash,
    event_type, specversion, source, event_time, datacontenttype,
    payload_json, payload_blob_ref,
    region_id, ingest_shard_id, ingest_log_offset, run_id,
    orphan_after,
    -- 18 prediction columns:
    predicted_a_tokens, predicted_b_tokens, predicted_c_tokens,
    reserved_strategy, prediction_strategy_used,
    prediction_policy_used, tokenizer_tier, tokenizer_version_id,
    prediction_confidence, prediction_sample_size,
    cold_start_layer_used,
    run_projection_at_decision_atomic,
    run_predicted_remaining_steps, run_steps_completed_so_far,
    actual_input_tokens, actual_output_tokens,
    delta_b_ratio, delta_c_ratio
) VALUES (
    '01999d80-0001-7000-8000-000000000030'::uuid,
    '01999d80-0001-7000-8000-000000000040'::uuid,
    '00000000-0000-4000-8000-000000000010'::uuid,
    '00000000-0000-7000-8000-000000000020'::uuid,
    'immutable_audit_log',
    'sidecar:test-prod-1',
    1,
    '\x00'::bytea,
    'sidecar:test-prod-1:key-1',
    -- Round-4 fix M13: synthetic schema_bundle UUID + hash.
    '00000000-0000-1111-2222-333333333333'::uuid,
    '\xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef'::bytea,
    'spendguard.audit.outcome',
    '1.0',
    'sidecar://test/wl-1',
    '2026-07-20T00:00:00Z'::timestamptz,
    'application/json',
    '{"test":"quarantine release"}'::jsonb,
    NULL,
    'us-east-1',
    'shard-0',
    0,
    NULL,
    '2026-07-20T00:00:30Z'::timestamptz,
    -- 18 prediction values:
    1000, 800, 900,
    'A', 'B', 'STRICT_CEILING', 'T2',
    NULL,                  -- tokenizer_version_id: NULL = Tier 3 fallback
    0.875, 64,
    'L2',
    1000000,
    3, 2,
    256, 384,
    0.75, 0.5
);

-- ============================================================================
-- Step 2: Confirm the row stored all 18 prediction columns.
-- ============================================================================
DO $$
DECLARE
    cnt INT;
BEGIN
    SELECT COUNT(*) INTO cnt FROM audit_outcome_quarantine
    WHERE event_id = '01999d80-0001-7000-8000-000000000040'::uuid
      AND predicted_a_tokens = 1000
      AND predicted_b_tokens = 800
      AND predicted_c_tokens = 900
      AND reserved_strategy = 'A'
      AND prediction_strategy_used = 'B'
      AND prediction_policy_used = 'STRICT_CEILING'
      AND tokenizer_tier = 'T2'
      AND prediction_confidence = 0.875
      AND prediction_sample_size = 64
      AND cold_start_layer_used = 'L2'
      AND run_projection_at_decision_atomic = 1000000
      AND run_predicted_remaining_steps = 3
      AND run_steps_completed_so_far = 2
      AND actual_input_tokens = 256
      AND actual_output_tokens = 384
      AND delta_b_ratio = 0.75
      AND delta_c_ratio = 0.5;
    IF cnt <> 1 THEN
        RAISE EXCEPTION 'FAIL: expected exactly 1 quarantine row with all 18 prediction columns populated, got %', cnt;
    END IF;
    RAISE NOTICE 'PASS: quarantine row stored 18 prediction columns';
END $$;

-- ============================================================================
-- Step 3: Simulate the release path INSERT into canonical_events with the
-- same 18 column values (the Rust code in
-- services/canonical_ingest/src/persistence/append.rs::release_quarantined_outcomes
-- does this in production; here we exercise the schema invariant only).
-- ============================================================================
INSERT INTO canonical_events (
    event_id, tenant_id, decision_id, run_id, event_type,
    storage_class,
    producer_id, producer_sequence, producer_signature, signing_key_id,
    schema_bundle_id, schema_bundle_hash,
    specversion, source, event_time, datacontenttype,
    payload_json, payload_blob_ref,
    region_id, ingest_shard_id, ingest_log_offset, ingest_at,
    recorded_month, failure_class,
    -- Carry forward all 18 prediction columns:
    predicted_a_tokens, predicted_b_tokens, predicted_c_tokens,
    reserved_strategy, prediction_strategy_used,
    prediction_policy_used, tokenizer_tier, tokenizer_version_id,
    prediction_confidence, prediction_sample_size,
    cold_start_layer_used,
    run_projection_at_decision_atomic,
    run_predicted_remaining_steps, run_steps_completed_so_far,
    actual_input_tokens, actual_output_tokens,
    delta_b_ratio, delta_c_ratio
)
SELECT
    event_id, tenant_id, decision_id, run_id, event_type,
    storage_class,
    producer_id, producer_sequence, producer_signature, signing_key_id,
    schema_bundle_id, schema_bundle_hash,
    specversion, source, event_time, datacontenttype,
    payload_json, payload_blob_ref,
    region_id, ingest_shard_id, 1::bigint, clock_timestamp(),
    date_trunc('month', event_time)::DATE, NULL,
    predicted_a_tokens, predicted_b_tokens, predicted_c_tokens,
    reserved_strategy, prediction_strategy_used,
    prediction_policy_used, tokenizer_tier, tokenizer_version_id,
    prediction_confidence, prediction_sample_size,
    cold_start_layer_used,
    run_projection_at_decision_atomic,
    run_predicted_remaining_steps, run_steps_completed_so_far,
    actual_input_tokens, actual_output_tokens,
    delta_b_ratio, delta_c_ratio
FROM audit_outcome_quarantine
WHERE event_id = '01999d80-0001-7000-8000-000000000040'::uuid;

-- ============================================================================
-- Step 4: Confirm canonical_events row mirrors the quarantine row.
-- ============================================================================
DO $$
DECLARE
    cnt INT;
BEGIN
    SELECT COUNT(*) INTO cnt FROM canonical_events
    WHERE event_id = '01999d80-0001-7000-8000-000000000040'::uuid
      AND predicted_a_tokens = 1000
      AND predicted_b_tokens = 800
      AND predicted_c_tokens = 900
      AND reserved_strategy = 'A'
      AND prediction_strategy_used = 'B'
      AND prediction_policy_used = 'STRICT_CEILING'
      AND tokenizer_tier = 'T2'
      AND prediction_confidence = 0.875
      AND prediction_sample_size = 64
      AND cold_start_layer_used = 'L2'
      AND run_projection_at_decision_atomic = 1000000
      AND run_predicted_remaining_steps = 3
      AND run_steps_completed_so_far = 2
      AND actual_input_tokens = 256
      AND actual_output_tokens = 384
      AND delta_b_ratio = 0.75
      AND delta_c_ratio = 0.5;
    IF cnt <> 1 THEN
        RAISE EXCEPTION 'FAIL: canonical_events did not receive identical prediction columns from quarantine release';
    END IF;
    RAISE NOTICE 'PASS: canonical_events row mirrors all 18 prediction columns from quarantine';
END $$;

-- ============================================================================
-- Step 5: Confirm canonical_events_outcome_required_cols_chk CHECK passes
-- (this was the root failure mode B2 was opened to address — without the
-- 18 columns on quarantine, the CHECK would fail on release for outcomes
-- past the cutoff).
--
-- Round-4 fix B3: RAISE NOTICE wrapped in DO $$ ... END $$ block. Bare
-- RAISE at SQL top level is a psql syntax error — the round-3 form
-- never actually executed under -v ON_ERROR_STOP=1.
-- ============================================================================
-- The INSERT above would have failed if the CHECK fired; reaching here
-- means PASS. Explicit confirmation:
DO $$
BEGIN
    RAISE NOTICE 'PASS: canonical_events_outcome_required_cols_chk did not fire on quarantine release';
END $$;

-- ============================================================================
-- Round-4 fix M9 (Step 6 — cutoff coverage). The round-3 test used
-- event_time = 2026-07-20 which is BEFORE the 2027-01-01 cutoff in
-- canonical_events_outcome_required_cols_chk. The CHECK is gated on
-- event_time >= cutoff, so the round-3 case never actually exercised
-- the constraint — it tested the unconditional pass branch.
--
-- This step adds two cases past the cutoff:
--   (M9 case A) event_time = 2027-02-01 + actual_*_tokens populated.
--               Expected: PASS (the constraint enforces only that
--               actual_input_tokens and actual_output_tokens are
--               populated when event_time >= cutoff).
--   (M9 case B) event_time = 2027-02-01 + actual_*_tokens NULL.
--               Expected: FAIL with errcode 23514 (check_violation).
--
-- ============================================================================

-- Case A: post-cutoff outcome with required cols populated → PASS.
-- The pre-decision in canonical_events_global_keys uses a new UUID; we
-- have to add a matching decision row so the
-- assert_audit_outcome_has_preceding_decision trigger doesn't reject.
DO $$
BEGIN
    INSERT INTO canonical_events_global_keys (
        event_id, tenant_id, decision_id, event_type, recorded_month
    ) VALUES (
        '01999d80-0001-7000-8000-000000000011'::uuid,
        '00000000-0000-4000-8000-000000000011'::uuid,
        '00000000-0000-7000-8000-000000000021'::uuid,
        'spendguard.audit.decision',
        '2027-02-01'::date
    );
EXCEPTION
    WHEN unique_violation THEN
        NULL;
END $$;

INSERT INTO canonical_events (
    event_id, tenant_id, decision_id, run_id, event_type,
    storage_class,
    producer_id, producer_sequence, producer_signature, signing_key_id,
    schema_bundle_id, schema_bundle_hash,
    specversion, source, event_time, datacontenttype,
    payload_json, payload_blob_ref,
    region_id, ingest_shard_id, ingest_log_offset, ingest_at,
    recorded_month, failure_class,
    actual_input_tokens, actual_output_tokens
) VALUES (
    '01999d80-0001-7000-8000-000000000041'::uuid,
    '00000000-0000-4000-8000-000000000011'::uuid,
    '00000000-0000-7000-8000-000000000021'::uuid,
    NULL, 'spendguard.audit.outcome',
    'immutable_audit_log',
    'sidecar:test-prod-1', 2, '\x00'::bytea, 'sidecar:test-prod-1:key-1',
    '00000000-0000-1111-2222-333333333333'::uuid,
    '\xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef'::bytea,
    '1.0', 'sidecar://test/wl-1', '2027-02-01T00:00:00Z'::timestamptz, 'application/json',
    '{"test":"post-cutoff PASS"}'::jsonb, NULL,
    'us-east-1', 'shard-0', 2::bigint, clock_timestamp(),
    '2027-02-01'::date, NULL,
    -- actual_input_tokens + actual_output_tokens populated → CHECK passes.
    128, 256
);

DO $$
BEGIN
    RAISE NOTICE 'PASS: post-cutoff outcome INSERT accepted with actual_*_tokens populated';
END $$;

-- Case B: post-cutoff outcome WITHOUT required cols → FAIL with errcode
-- 23514. We use a savepoint to capture the failure without aborting the
-- whole transaction (which we still want to ROLLBACK at the end).
DO $$
DECLARE
    sqlstate_caught TEXT;
BEGIN
    BEGIN
        -- Pre-arrange the matching decision row.
        BEGIN
            INSERT INTO canonical_events_global_keys (
                event_id, tenant_id, decision_id, event_type, recorded_month
            ) VALUES (
                '01999d80-0001-7000-8000-000000000012'::uuid,
                '00000000-0000-4000-8000-000000000012'::uuid,
                '00000000-0000-7000-8000-000000000022'::uuid,
                'spendguard.audit.decision',
                '2027-02-01'::date
            );
        EXCEPTION WHEN unique_violation THEN NULL;
        END;

        INSERT INTO canonical_events (
            event_id, tenant_id, decision_id, run_id, event_type,
            storage_class,
            producer_id, producer_sequence, producer_signature, signing_key_id,
            schema_bundle_id, schema_bundle_hash,
            specversion, source, event_time, datacontenttype,
            payload_json, payload_blob_ref,
            region_id, ingest_shard_id, ingest_log_offset, ingest_at,
            recorded_month, failure_class,
            actual_input_tokens, actual_output_tokens
        ) VALUES (
            '01999d80-0001-7000-8000-000000000042'::uuid,
            '00000000-0000-4000-8000-000000000012'::uuid,
            '00000000-0000-7000-8000-000000000022'::uuid,
            NULL, 'spendguard.audit.outcome',
            'immutable_audit_log',
            'sidecar:test-prod-1', 3, '\x00'::bytea, 'sidecar:test-prod-1:key-1',
            '00000000-0000-1111-2222-333333333333'::uuid,
            '\xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef'::bytea,
            '1.0', 'sidecar://test/wl-1', '2027-02-01T00:00:00Z'::timestamptz, 'application/json',
            '{"test":"post-cutoff FAIL"}'::jsonb, NULL,
            'us-east-1', 'shard-0', 3::bigint, clock_timestamp(),
            '2027-02-01'::date, NULL,
            -- actual_input_tokens + actual_output_tokens NULL → CHECK fails.
            NULL, NULL
        );

        -- If we reach here the CHECK didn't fire — test fail.
        RAISE EXCEPTION 'FAIL: post-cutoff outcome INSERT with NULL actual_*_tokens should have raised 23514 but did not';
    EXCEPTION
        WHEN check_violation THEN
            GET STACKED DIAGNOSTICS sqlstate_caught = RETURNED_SQLSTATE;
            RAISE NOTICE 'PASS: post-cutoff outcome rejected with errcode % as expected', sqlstate_caught;
    END;
END $$;

DO $$
BEGIN
    RAISE NOTICE 'ALL TESTS PASSED: B2 quarantine release path preserves 18 prediction columns end-to-end + B3 syntax + M9 cutoff coverage';
END $$;

ROLLBACK;
