-- =====================================================================
-- Cost Advisor P1.5 — failed_retry_burn_v1 + runaway_loop_v1 fixture
-- =====================================================================
--
-- Synthesizes canonical_events rows that fire both new rules. Wraps in
-- BEGIN/ROLLBACK so no persistent state lands.
--
-- Run from deploy/demo:
--   docker exec -i $PG psql -U spendguard -d spendguard_canonical \
--       -v ON_ERROR_STOP=1 -f verify_p1_5_cost_advisor.sql
--
-- Note: this fixture runs against spendguard_canonical (where
-- canonical_events lives), not spendguard_ledger.

\set ON_ERROR_STOP 1

BEGIN;

-- Ensure the schema_bundles row that canonical_events FKs to exists.
-- The demo's canonical-seed-init normally seeds this from compose;
-- when this fixture runs standalone (e.g. against a bare postgres
-- container) we self-seed.
INSERT INTO schema_bundles
    (schema_bundle_id, schema_bundle_hash, canonical_schema_version)
VALUES (
    '22222222-2222-4222-8222-222222222222',
    '\x00'::bytea,
    'v1alpha1'
) ON CONFLICT (schema_bundle_id) DO NOTHING;

DO $$
DECLARE
    v_tenant UUID := '00000000-0000-4000-8000-000000000001';
    v_run_a UUID := gen_random_uuid();
    v_run_b UUID := gen_random_uuid();
    v_prompt_x_b64 TEXT := replace(encode('{"prompt_hash":"x_hash","estimated_amount_atomic":"1000"}'::bytea, 'base64'), E'\n', '');
    v_prompt_y_b64 TEXT := replace(encode('{"prompt_hash":"y_hash","estimated_amount_atomic":"200"}'::bytea, 'base64'), E'\n', '');
    v_schema_bundle UUID := '22222222-2222-4222-8222-222222222222';
    v_event_id UUID;
    v_decision_id UUID;
    v_seq BIGINT := 5000;
    v_i INT;
    v_event_time TIMESTAMPTZ;
BEGIN
    -- =================================================================
    -- Run A: prompt X retried 3 times with provider_5xx (billed)
    -- → should fire failed_retry_burn_v1
    -- =================================================================
    FOR v_i IN 1..3 LOOP
        v_decision_id := gen_random_uuid();
        v_event_time := '2026-05-13 12:00:00+00'::timestamptz + (v_i * INTERVAL '10 seconds');

        -- audit.decision row (must precede audit.outcome per
        -- canonical_events_audit_sequence_mirror trigger).
        v_event_id := gen_random_uuid();
        INSERT INTO canonical_events_global_keys
            (event_id, tenant_id, decision_id, event_type, recorded_month)
        VALUES (v_event_id, v_tenant, v_decision_id, 'spendguard.audit.decision', '2026-05-01');
        INSERT INTO canonical_events (
            event_id, tenant_id, decision_id, run_id, event_type, storage_class,
            producer_id, producer_sequence, producer_signature, signing_key_id,
            schema_bundle_id, schema_bundle_hash,
            specversion, source, event_time, datacontenttype,
            payload_json, payload_blob_ref,
            region_id, ingest_shard_id, ingest_log_offset,
            recorded_month, failure_class
        ) VALUES (
            v_event_id, v_tenant, v_decision_id, v_run_a, 'spendguard.audit.decision',
            'immutable_audit_log',
            'p1_5-fixture', v_seq, '\x00'::bytea, 'demo-key-1',
            v_schema_bundle, '\x00'::bytea,
            '1.0', 'p1_5://fixture', v_event_time, 'application/json',
            ('{"data_b64":"' || v_prompt_x_b64 || '"}')::jsonb, NULL,
            'demo', 'p1_5-shard', v_seq,
            '2026-05-01', NULL  -- decisions don't carry failure_class
        );
        v_seq := v_seq + 1;

        -- audit.outcome with provider_5xx failure
        v_event_id := gen_random_uuid();
        INSERT INTO canonical_events_global_keys
            (event_id, tenant_id, decision_id, event_type, recorded_month)
        VALUES (v_event_id, v_tenant, v_decision_id, 'spendguard.audit.outcome', '2026-05-01');
        INSERT INTO canonical_events (
            event_id, tenant_id, decision_id, run_id, event_type, storage_class,
            producer_id, producer_sequence, producer_signature, signing_key_id,
            schema_bundle_id, schema_bundle_hash,
            specversion, source, event_time, datacontenttype,
            payload_json, payload_blob_ref,
            region_id, ingest_shard_id, ingest_log_offset,
            recorded_month, failure_class
        ) VALUES (
            v_event_id, v_tenant, v_decision_id, v_run_a, 'spendguard.audit.outcome',
            'immutable_audit_log',
            'p1_5-fixture', v_seq, '\x00'::bytea, 'demo-key-1',
            v_schema_bundle, '\x00'::bytea,
            '1.0', 'p1_5://fixture', v_event_time + INTERVAL '1 second', 'application/json',
            ('{"data_b64":"' || v_prompt_x_b64 || '"}')::jsonb, NULL,
            'demo', 'p1_5-shard', v_seq,
            '2026-05-01', 'provider_5xx'
        );
        v_seq := v_seq + 1;
    END LOOP;

    -- =================================================================
    -- Run B: prompt Y called 7 times in 60s with no failure
    -- → should fire runaway_loop_v1
    -- =================================================================
    FOR v_i IN 1..7 LOOP
        v_decision_id := gen_random_uuid();
        v_event_time := '2026-05-13 13:00:00+00'::timestamptz + (v_i * INTERVAL '5 seconds');

        v_event_id := gen_random_uuid();
        INSERT INTO canonical_events_global_keys
            (event_id, tenant_id, decision_id, event_type, recorded_month)
        VALUES (v_event_id, v_tenant, v_decision_id, 'spendguard.audit.decision', '2026-05-01');
        INSERT INTO canonical_events (
            event_id, tenant_id, decision_id, run_id, event_type, storage_class,
            producer_id, producer_sequence, producer_signature, signing_key_id,
            schema_bundle_id, schema_bundle_hash,
            specversion, source, event_time, datacontenttype,
            payload_json, payload_blob_ref,
            region_id, ingest_shard_id, ingest_log_offset,
            recorded_month, failure_class
        ) VALUES (
            v_event_id, v_tenant, v_decision_id, v_run_b, 'spendguard.audit.decision',
            'immutable_audit_log',
            'p1_5-fixture', v_seq, '\x00'::bytea, 'demo-key-1',
            v_schema_bundle, '\x00'::bytea,
            '1.0', 'p1_5://fixture', v_event_time, 'application/json',
            ('{"data_b64":"' || v_prompt_y_b64 || '"}')::jsonb, NULL,
            'demo', 'p1_5-shard', v_seq,
            '2026-05-01', NULL
        );
        v_seq := v_seq + 1;

        v_event_id := gen_random_uuid();
        INSERT INTO canonical_events_global_keys
            (event_id, tenant_id, decision_id, event_type, recorded_month)
        VALUES (v_event_id, v_tenant, v_decision_id, 'spendguard.audit.outcome', '2026-05-01');
        INSERT INTO canonical_events (
            event_id, tenant_id, decision_id, run_id, event_type, storage_class,
            producer_id, producer_sequence, producer_signature, signing_key_id,
            schema_bundle_id, schema_bundle_hash,
            specversion, source, event_time, datacontenttype,
            payload_json, payload_blob_ref,
            region_id, ingest_shard_id, ingest_log_offset,
            recorded_month, failure_class
        ) VALUES (
            v_event_id, v_tenant, v_decision_id, v_run_b, 'spendguard.audit.outcome',
            'immutable_audit_log',
            'p1_5-fixture', v_seq, '\x00'::bytea, 'demo-key-1',
            v_schema_bundle, '\x00'::bytea,
            '1.0', 'p1_5://fixture', v_event_time + INTERVAL '1 second', 'application/json',
            ('{"data_b64":"' || v_prompt_y_b64 || '"}')::jsonb, NULL,
            'demo', 'p1_5-shard', v_seq,
            '2026-05-01', 'unknown'  -- successful provider call, no failure
        );
        v_seq := v_seq + 1;
    END LOOP;
END $$;

-- =====================================================================
-- Run failed_retry_burn_v1 + assert it fires.
-- =====================================================================
DO $$
DECLARE
    v_affected BIGINT; v_attempts BIGINT; v_billed BIGINT;
    v_sample UUID[];
BEGIN
    WITH step1 AS (
        SELECT
            c.event_id, c.run_id, c.event_time, c.decision_id, c.failure_class,
            cost_advisor_safe_decode_payload(c.payload_json) AS inner_data
          FROM canonical_events c
         WHERE c.tenant_id = '00000000-0000-4000-8000-000000000001'
           AND c.event_type = 'spendguard.audit.outcome'
           AND c.event_time >= '2026-05-13 12:00:00+00'::timestamptz
           AND c.event_time < '2026-05-13 13:00:00+00'::timestamptz
           AND c.run_id IS NOT NULL
           AND c.failure_class IS NOT NULL
    ),
    step2 AS (
        SELECT
            run_id, inner_data->>'prompt_hash' AS prompt_hash,
            COUNT(*) AS attempt_count,
            COUNT(*) FILTER (WHERE failure_class IN (
                'provider_5xx','provider_4xx_billed','malformed_json_response','timeout_billed')) AS billed_failure_count,
            (array_agg(decision_id ORDER BY event_time DESC) FILTER (
                WHERE failure_class IN (
                    'provider_5xx','provider_4xx_billed','malformed_json_response','timeout_billed')))[1:5] AS sample_decision_ids
          FROM step1
         WHERE inner_data->>'prompt_hash' IS NOT NULL
         GROUP BY run_id, inner_data->>'prompt_hash'
    ),
    step3 AS (
        SELECT * FROM step2 WHERE attempt_count >= 2 AND billed_failure_count >= 2
    )
    SELECT
        COUNT(*), SUM(attempt_count), SUM(billed_failure_count),
        (SELECT sample_decision_ids FROM step3 LIMIT 1)
      INTO v_affected, v_attempts, v_billed, v_sample
      FROM step3;

    IF v_affected IS NULL OR v_affected < 1 THEN
        RAISE EXCEPTION 'failed_retry_burn FAIL: expected >=1 affected group, got %', v_affected;
    END IF;
    IF v_attempts <> 3 THEN
        RAISE EXCEPTION 'failed_retry_burn FAIL: expected 3 attempts, got %', v_attempts;
    END IF;
    IF v_billed <> 3 THEN
        RAISE EXCEPTION 'failed_retry_burn FAIL: expected 3 billed failures, got %', v_billed;
    END IF;
    RAISE NOTICE 'failed_retry_burn_v1: PASS (affected=% attempts=% billed_failures=%)',
        v_affected, v_attempts, v_billed;
END $$;

-- =====================================================================
-- Run runaway_loop_v1 + assert it fires.
-- =====================================================================
DO $$
DECLARE
    v_affected BIGINT; v_calls BIGINT; v_max_depth BIGINT;
BEGIN
    WITH step1 AS (
        SELECT
            c.event_id, c.run_id, c.event_time, c.decision_id, c.failure_class,
            cost_advisor_safe_decode_payload(c.payload_json) AS inner_data
          FROM canonical_events c
         WHERE c.tenant_id = '00000000-0000-4000-8000-000000000001'
           AND c.event_type = 'spendguard.audit.outcome'
           AND c.event_time >= '2026-05-13 13:00:00+00'::timestamptz
           AND c.event_time < '2026-05-13 13:01:00+00'::timestamptz
           AND c.run_id IS NOT NULL
           AND (c.failure_class IS NULL OR c.failure_class = 'unknown')
    ),
    step2 AS (
        SELECT
            run_id, inner_data->>'prompt_hash' AS prompt_hash,
            COUNT(*) AS call_count
          FROM step1
         WHERE inner_data->>'prompt_hash' IS NOT NULL
         GROUP BY run_id, inner_data->>'prompt_hash'
    ),
    step3 AS (SELECT * FROM step2 WHERE call_count > 5)
    SELECT
        COUNT(*), SUM(call_count), MAX(call_count)
      INTO v_affected, v_calls, v_max_depth
      FROM step3;

    IF v_affected IS NULL OR v_affected < 1 THEN
        RAISE EXCEPTION 'runaway_loop FAIL: expected >=1 affected group, got %', v_affected;
    END IF;
    IF v_calls <> 7 THEN
        RAISE EXCEPTION 'runaway_loop FAIL: expected 7 calls, got %', v_calls;
    END IF;
    IF v_max_depth <> 7 THEN
        RAISE EXCEPTION 'runaway_loop FAIL: expected max_depth=7, got %', v_max_depth;
    END IF;
    RAISE NOTICE 'runaway_loop_v1: PASS (affected=% total_calls=% max_depth=%)',
        v_affected, v_calls, v_max_depth;
END $$;

SELECT 'all_p1_5_fixtures: PASS' AS final_status;

ROLLBACK;
