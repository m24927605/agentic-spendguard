-- Acceptance test (SQL): immutability trigger covers all 18 prediction
-- columns added in 0046_audit_outbox_prediction_columns.sql.
-- Round-2 fix m5 (per SLICE_01 §8.1 + §8.3 acceptance criteria).
--
-- Spec: docs/audit-chain-prediction-extension-v1alpha1.md §5.2
--   "對 18 個新欄位的 UPDATE attempt（在 demo Postgres 上）全部 raise 42P10"
--
-- ## How to run
--
--   psql "$PG_LEDGER_URL" -v ON_ERROR_STOP=1 \
--        -f services/ledger/tests/migrations/test_prediction_columns_trigger.sql
--
-- ## What it checks
--
--   1. Insert a baseline audit_outbox row with all 18 prediction columns
--      populated to valid values.
--   2. For each of the 18 columns, attempt UPDATE and assert Postgres
--      raises SQLSTATE 42P10.
--   3. As a positive control, UPDATE pending_forward (a forwarder-state
--      column) and assert it succeeds — confirms the trigger isn't
--      over-firing.
--
-- Exit code 0 on PASS; non-zero (via RAISE EXCEPTION) on any FAIL.
--
-- ## Why SQL not Rust
--
-- This test exercises pure Postgres behavior — the SQL trigger and the
-- IS DISTINCT FROM tuple compare. A Rust integration test would add
-- testcontainers + sqlx setup cost without exercising anything the SQL
-- test doesn't already cover. SLICE_06's producer-side Rust tests will
-- supplement this with end-to-end mirror coverage.

\set ON_ERROR_STOP on
\set VERBOSITY verbose

BEGIN;

-- Round-3 fix M8: insert a ledger_transactions fixture row inside the
-- same transaction so the audit_outbox INSERT below doesn't FK-fail on
-- empty fixture environments. The misleading "savepoint" comment in
-- round-2 has been removed — this is just a baseline INSERT.
--
-- Use a UUID that does NOT clash with any existing test data. The row
-- only needs to exist (the audit_outbox FK is RESTRICT, not CASCADE);
-- we don't exercise any ledger_transaction fields here. Required NOT NULL
-- columns per services/ledger/migrations/0007_ledger_transactions.sql:
-- ledger_transaction_id (PK), tenant_id, operation_kind, idempotency_key,
-- request_hash, effective_at, recorded_at, lock_order_token.
INSERT INTO ledger_transactions (
    ledger_transaction_id,
    tenant_id,
    operation_kind,
    idempotency_key,
    request_hash,
    effective_at,
    recorded_at,
    lock_order_token
) VALUES (
    '01999d70-0001-7000-8000-0000000000ff'::uuid,
    '00000000-0000-4000-8000-000000000001'::uuid,
    'reserve',
    'test-prediction-trigger-fixture-1',
    '\x00'::bytea,
    '2026-07-15T00:00:00Z'::timestamptz,
    '2026-07-15T00:00:00Z'::timestamptz,
    'test-lock-order'
)
ON CONFLICT (tenant_id, operation_kind, idempotency_key) DO NOTHING;

-- Set up a known-good baseline row. Use the 2026-07 partition that
-- 0009 pre-creates so we don't depend on partition pre-creation order.
INSERT INTO audit_outbox (
    audit_outbox_id,
    audit_decision_event_id,
    decision_id,
    tenant_id,
    ledger_transaction_id,
    event_type,
    cloudevent_payload,
    cloudevent_payload_signature,
    ledger_fencing_epoch,
    workload_instance_id,
    pending_forward,
    forwarded_at,
    forward_attempts,
    recorded_at,
    recorded_month,
    producer_sequence,
    idempotency_key,
    -- 18 new prediction columns:
    predicted_a_tokens,
    predicted_b_tokens,
    predicted_c_tokens,
    reserved_strategy,
    prediction_strategy_used,
    prediction_policy_used,
    tokenizer_tier,
    tokenizer_version_id,
    prediction_confidence,
    prediction_sample_size,
    cold_start_layer_used,
    run_projection_at_decision_atomic,
    run_predicted_remaining_steps,
    run_steps_completed_so_far,
    actual_input_tokens,
    actual_output_tokens,
    delta_b_ratio,
    delta_c_ratio
) VALUES (
    '01999d70-0001-7000-8000-000000000001'::uuid,
    '01999d70-0001-7000-8000-000000000002'::uuid,
    '01999d70-0001-7000-8000-000000000003'::uuid,
    '00000000-0000-4000-8000-000000000001'::uuid,
    -- Round-3 fix M8: bind the FK to the fixture row inserted above
    -- (replaces the brittle SELECT FROM ledger_transactions LIMIT 1
    -- which would silently bind to whatever pre-existing row appeared
    -- first; deterministic-fixture binding is more debuggable).
    '01999d70-0001-7000-8000-0000000000ff'::uuid,
    'spendguard.audit.decision',
    '{"test": "round-2 trigger probe"}'::jsonb,
    '\x00'::bytea,
    1,
    'test-wl',
    TRUE,
    NULL,
    0,
    '2026-07-15T00:00:00Z'::timestamptz,
    '2026-07-01'::date,
    1,
    'test-key-1',
    -- 18 new prediction column values:
    1000, 800, 900, 'A', 'B', 'STRICT_CEILING', 'T2',
    NULL,  -- tokenizer_version_id; FK is RESTRICT but NULL bypasses
    0.875, 64, 'L2',
    1000000, 3, 2,
    256, 384, 0.75, 0.5
);

DO $$
DECLARE
    missing_input BIGINT;
    missing_output BIGINT;
    zero_input BIGINT;
    zero_output BIGINT;
BEGIN
    INSERT INTO audit_outbox (
        audit_outbox_id,
        audit_decision_event_id,
        decision_id,
        tenant_id,
        ledger_transaction_id,
        event_type,
        cloudevent_payload,
        cloudevent_payload_signature,
        ledger_fencing_epoch,
        workload_instance_id,
        pending_forward,
        recorded_at,
        recorded_month,
        producer_sequence,
        idempotency_key
    ) VALUES (
        '01999d70-0001-7000-8000-000000000010'::uuid,
        '01999d70-0001-7000-8000-000000000011'::uuid,
        '01999d70-0001-7000-8000-000000000012'::uuid,
        '00000000-0000-4000-8000-000000000001'::uuid,
        '01999d70-0001-7000-8000-0000000000ff'::uuid,
        'spendguard.audit.outcome',
        jsonb_build_object(
            'data_b64', encode(convert_to('{"estimated_amount_atomic":"42"}', 'UTF8'), 'base64'),
            'actual_input_tokens', 0,
            'actual_output_tokens', 0
        ),
        '\x00'::bytea,
        1,
        'test-wl',
        TRUE,
        '2026-07-15T00:00:00Z'::timestamptz,
        '2026-07-01'::date,
        2,
        'test-key-missing-actuals'
    )
    RETURNING actual_input_tokens, actual_output_tokens
    INTO missing_input, missing_output;

    IF missing_input IS NOT NULL OR missing_output IS NOT NULL THEN
        RAISE EXCEPTION 'FAIL: missing outcome usage mirrored as %, % instead of NULL, NULL',
            missing_input, missing_output;
    END IF;

    INSERT INTO audit_outbox (
        audit_outbox_id,
        audit_decision_event_id,
        decision_id,
        tenant_id,
        ledger_transaction_id,
        event_type,
        cloudevent_payload,
        cloudevent_payload_signature,
        ledger_fencing_epoch,
        workload_instance_id,
        pending_forward,
        recorded_at,
        recorded_month,
        producer_sequence,
        idempotency_key
    ) VALUES (
        '01999d70-0001-7000-8000-000000000020'::uuid,
        '01999d70-0001-7000-8000-000000000021'::uuid,
        '01999d70-0001-7000-8000-000000000022'::uuid,
        '00000000-0000-4000-8000-000000000001'::uuid,
        '01999d70-0001-7000-8000-0000000000ff'::uuid,
        'spendguard.audit.outcome',
        jsonb_build_object(
            'data_b64', encode(convert_to('{"actual_input_tokens":0,"actual_output_tokens":0}', 'UTF8'), 'base64'),
            'actual_input_tokens', 0,
            'actual_output_tokens', 0
        ),
        '\x00'::bytea,
        1,
        'test-wl',
        TRUE,
        '2026-07-15T00:00:00Z'::timestamptz,
        '2026-07-01'::date,
        3,
        'test-key-zero-actuals'
    )
    RETURNING actual_input_tokens, actual_output_tokens
    INTO zero_input, zero_output;

    IF zero_input <> 0 OR zero_output <> 0 THEN
        RAISE EXCEPTION 'FAIL: explicit zero outcome usage mirrored as %, % instead of 0, 0',
            zero_input, zero_output;
    END IF;

    RAISE NOTICE 'PASS: 0052 mirror keeps missing actual usage NULL and preserves explicit zero usage';
END $$;

DO $$
DECLARE
    test_id UUID := '01999d70-0001-7000-8000-000000000001'::uuid;
    test_month DATE := '2026-07-01'::date;
    test_columns TEXT[] := ARRAY[
        'predicted_a_tokens',
        'predicted_b_tokens',
        'predicted_c_tokens',
        'reserved_strategy',
        'prediction_strategy_used',
        'prediction_policy_used',
        'tokenizer_tier',
        -- tokenizer_version_id is NULL so we test with a UUID change
        'tokenizer_version_id',
        'prediction_confidence',
        'prediction_sample_size',
        'cold_start_layer_used',
        'run_projection_at_decision_atomic',
        'run_predicted_remaining_steps',
        'run_steps_completed_so_far',
        'actual_input_tokens',
        'actual_output_tokens',
        'delta_b_ratio',
        'delta_c_ratio'
    ];
    -- A throwaway new-value expression per column type. Order MUST
    -- match test_columns above.
    new_values TEXT[] := ARRAY[
        '2000',                                        -- predicted_a_tokens
        '1500',                                        -- predicted_b_tokens
        '1700',                                        -- predicted_c_tokens
        '''B''',                                       -- reserved_strategy
        '''A''',                                       -- prediction_strategy_used
        '''ADAPTIVE_CEILING''',                        -- prediction_policy_used
        '''T1''',                                      -- tokenizer_tier
        '''01999d70-0001-7000-8000-000000000099''',    -- tokenizer_version_id (was NULL)
        '0.999',                                       -- prediction_confidence
        '128',                                         -- prediction_sample_size
        '''L4''',                                      -- cold_start_layer_used
        '2000000',                                     -- run_projection_at_decision_atomic
        '5',                                           -- run_predicted_remaining_steps
        '4',                                           -- run_steps_completed_so_far
        '512',                                         -- actual_input_tokens
        '768',                                         -- actual_output_tokens
        '0.9',                                         -- delta_b_ratio
        '0.7'                                          -- delta_c_ratio
    ];
    col_name TEXT;
    new_val TEXT;
    i INT;
    sqlstate_value TEXT;
    update_sql TEXT;
BEGIN
    FOR i IN 1..array_length(test_columns, 1) LOOP
        col_name := test_columns[i];
        new_val := new_values[i];
        update_sql := format(
            'UPDATE audit_outbox SET %I = %s WHERE audit_outbox_id = %L AND recorded_month = %L',
            col_name, new_val, test_id, test_month
        );
        BEGIN
            EXECUTE update_sql;
            RAISE EXCEPTION 'FAIL: UPDATE on column % succeeded — expected 42P10 from immutability trigger', col_name;
        EXCEPTION
            WHEN OTHERS THEN
                GET STACKED DIAGNOSTICS sqlstate_value = RETURNED_SQLSTATE;
                IF sqlstate_value <> '42P10' THEN
                    RAISE EXCEPTION 'FAIL: UPDATE on column % raised SQLSTATE % instead of 42P10', col_name, sqlstate_value;
                END IF;
                RAISE NOTICE 'PASS: UPDATE on column % correctly raised 42P10', col_name;
        END;
    END LOOP;

    -- Positive control: forwarder-state column UPDATE must SUCCEED.
    UPDATE audit_outbox
       SET pending_forward = FALSE,
           forwarded_at = clock_timestamp()
     WHERE audit_outbox_id = test_id
       AND recorded_month = test_month;
    RAISE NOTICE 'PASS: forwarder-state UPDATE succeeded (positive control)';

    RAISE NOTICE 'ALL TESTS PASSED: 18 prediction columns covered by immutability trigger + forwarder UPDATE path intact';
END $$;

-- Always roll back: this test does not persist any state.
ROLLBACK;
