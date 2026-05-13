-- =====================================================================
-- Cost Advisor P1 — idle_reservation_rate_v1 fixture + assertion
-- =====================================================================
--
-- Seeds 10 reservations for tenant `00000000-0000-4000-8000-000000000001`
-- on `2026-05-13`:
--   * 7 TTL'd-then-released with reason='TTL_EXPIRED' (70% idle ratio,
--     above the 20% threshold) and a tight 30s TTL (below the 60s
--     median ceiling).
--   * 3 committed normally (act as the denominator).
--
-- Then runs the rule SQL with $1=tenant, $2=date and asserts it
-- returns the expected aggregate row. Rolls back at the end so no
-- persistent state lands.
--
-- Run from deploy/demo:
--   docker exec -i $PG psql -U spendguard -d spendguard_ledger \
--       -v ON_ERROR_STOP=1 -f verify_p1_cost_advisor.sql

\set ON_ERROR_STOP 1

BEGIN;

-- =====================================================================
-- Seed 10 reservations (7 TTL'd, 3 committed) for one tenant + date
-- =====================================================================
DO $$
DECLARE
    v_i INT;
    v_decision_id UUID;
    v_reserve_tx_id UUID;
    v_release_tx_id UUID;
    v_audit_decision_id UUID;
    v_audit_outcome_id UUID;
    v_reservation_id UUID;
    v_created_at TIMESTAMPTZ;
    v_ttl_expires_at TIMESTAMPTZ;
    v_seq BIGINT := 100;  -- avoid colliding with seed data
BEGIN
    FOR v_i IN 1..10 LOOP
        v_decision_id := gen_random_uuid();
        v_reserve_tx_id := gen_random_uuid();
        v_release_tx_id := gen_random_uuid();
        v_audit_decision_id := gen_random_uuid();
        v_audit_outcome_id := gen_random_uuid();
        v_reservation_id := gen_random_uuid();
        v_created_at := '2026-05-13 12:00:00+00'::timestamptz + (v_i * INTERVAL '1 minute');
        v_ttl_expires_at := v_created_at + INTERVAL '30 seconds';  -- 30s TTL

        -- Reserve tx + audit.decision
        INSERT INTO ledger_transactions (
            ledger_transaction_id, tenant_id, operation_kind, posting_state,
            idempotency_key, request_hash, lock_order_token, decision_id,
            audit_decision_event_id, effective_at, fencing_scope_id, fencing_epoch_at_post
        ) VALUES (
            v_reserve_tx_id,
            '00000000-0000-4000-8000-000000000001',
            'reserve', 'posted',
            'p1-fixture-reserve-' || v_i,
            '\x00'::bytea, 'lock-' || v_i, v_decision_id,
            v_audit_decision_id, v_created_at,
            '33333333-3333-4333-8333-333333333333', 1
        );

        INSERT INTO audit_outbox (
            audit_outbox_id, audit_decision_event_id, decision_id, tenant_id,
            ledger_transaction_id, event_type, cloudevent_payload,
            cloudevent_payload_signature, ledger_fencing_epoch, workload_instance_id,
            recorded_at, recorded_month, producer_sequence, idempotency_key
        ) VALUES (
            gen_random_uuid(), v_audit_decision_id, v_decision_id,
            '00000000-0000-4000-8000-000000000001', v_reserve_tx_id,
            'spendguard.audit.decision',
            ('{"specversion":"1.0","type":"spendguard.audit.decision","data_b64":"' ||
              encode('{"kind":"reserve"}'::bytea, 'base64') || '"}')::jsonb,
            '\x00'::bytea, 1, 'p1-fixture',
            v_created_at, '2026-05-01', v_seq, 'p1-fixture-reserve-' || v_i
        );
        v_seq := v_seq + 1;

        -- Reservation row.
        -- First 7 land as 'released' with TTL_EXPIRED. Last 3 are 'committed'.
        INSERT INTO reservations (
            reservation_id, tenant_id, budget_id, window_instance_id, current_state,
            source_ledger_transaction_id, ttl_expires_at, idempotency_key, created_at
        ) VALUES (
            v_reservation_id,
            '00000000-0000-4000-8000-000000000001',
            '44444444-4444-4444-8444-444444444444',
            '55555555-5555-4555-8555-555555555555',
            CASE WHEN v_i <= 7 THEN 'released' ELSE 'committed' END,
            v_reserve_tx_id, v_ttl_expires_at,
            'p1-fixture-reserve-' || v_i, v_created_at
        );

        -- For the 7 TTL'd ones, write a release tx + audit.outcome with
        -- reason=TTL_EXPIRED.
        IF v_i <= 7 THEN
            INSERT INTO ledger_transactions (
                ledger_transaction_id, tenant_id, operation_kind, posting_state,
                idempotency_key, request_hash, lock_order_token, decision_id,
                audit_decision_event_id, effective_at, fencing_scope_id, fencing_epoch_at_post
            ) VALUES (
                v_release_tx_id,
                '00000000-0000-4000-8000-000000000001',
                'release', 'posted',
                'p1-fixture-release-' || v_i,
                '\x01'::bytea, 'lock-rel-' || v_i, v_decision_id,
                v_audit_outcome_id, v_ttl_expires_at,
                '33333333-3333-4333-8333-333333333333', 1
            );

            INSERT INTO audit_outbox (
                audit_outbox_id, audit_decision_event_id, decision_id, tenant_id,
                ledger_transaction_id, event_type, cloudevent_payload,
                cloudevent_payload_signature, ledger_fencing_epoch, workload_instance_id,
                recorded_at, recorded_month, producer_sequence, idempotency_key
            ) VALUES (
                gen_random_uuid(), v_audit_outcome_id, v_decision_id,
                '00000000-0000-4000-8000-000000000001', v_release_tx_id,
                'spendguard.audit.outcome',
                ('{"specversion":"1.0","type":"spendguard.audit.outcome","data_b64":"' ||
                  encode('{"kind":"release","reason":"TTL_EXPIRED"}'::bytea, 'base64') || '"}')::jsonb,
                '\x00'::bytea, 1, 'p1-fixture',
                v_ttl_expires_at, '2026-05-01', v_seq, 'p1-fixture-release-' || v_i
            );
            v_seq := v_seq + 1;
        END IF;
    END LOOP;
END $$;

-- =====================================================================
-- Run the actual rule SQL inline + assert it fires.
-- =====================================================================
DO $$
DECLARE
    v_total BIGINT;
    v_ttl_expired BIGINT;
    v_median_ttl INT;
    v_p95_ttl INT;
    v_sample UUID[];
    v_waste BIGINT;
BEGIN
    SELECT
        COUNT(*)::BIGINT,
        COUNT(*) FILTER (WHERE derived_state = 'ttl_expired')::BIGINT,
        PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY ttl_seconds)::INT,
        PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY ttl_seconds)::INT,
        (
            SELECT array_agg(reservation_id ORDER BY released_at DESC)
              FROM (
                  SELECT reservation_id, released_at
                    FROM reservations_with_ttl_status_v1
                   WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
                     AND derived_state = 'ttl_expired'
                     AND created_at >= '2026-05-13'::date
                     AND created_at < ('2026-05-13'::date + INTERVAL '1 day')
                   ORDER BY released_at DESC
                   LIMIT 5
              ) sample
        ),
        (COUNT(*) FILTER (WHERE derived_state = 'ttl_expired')::BIGINT * 100000)
      INTO v_total, v_ttl_expired, v_median_ttl, v_p95_ttl, v_sample, v_waste
      FROM reservations_with_ttl_status_v1
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND created_at >= '2026-05-13'::date
       AND created_at < ('2026-05-13'::date + INTERVAL '1 day')
     GROUP BY tenant_id
    HAVING
        COUNT(*) > 0
        AND (COUNT(*) FILTER (WHERE derived_state = 'ttl_expired')::NUMERIC
             / NULLIF(COUNT(*), 0)) > 0.20
        AND PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY ttl_seconds) <= 60;

    -- Rule should have fired.
    IF v_total IS NULL THEN
        RAISE EXCEPTION 'p1_fixture FAIL: rule did not fire (expected 10 reservations + 7 TTL''d)';
    END IF;
    IF v_total <> 10 THEN
        RAISE EXCEPTION 'p1_fixture FAIL: expected total=10, got %', v_total;
    END IF;
    IF v_ttl_expired <> 7 THEN
        RAISE EXCEPTION 'p1_fixture FAIL: expected ttl_expired=7, got %', v_ttl_expired;
    END IF;
    IF v_median_ttl <> 30 THEN
        RAISE EXCEPTION 'p1_fixture FAIL: expected median_ttl=30, got %', v_median_ttl;
    END IF;
    IF array_length(v_sample, 1) <> 5 THEN
        RAISE EXCEPTION 'p1_fixture FAIL: expected 5 sample reservations, got %',
            array_length(v_sample, 1);
    END IF;
    IF v_waste <> 700000 THEN  -- 7 × 100_000 microUSD
        RAISE EXCEPTION 'p1_fixture FAIL: expected waste=700000, got %', v_waste;
    END IF;

    RAISE NOTICE
        'p1_fixture: PASS (total=%, ttl_expired=%, idle_ratio=%, median_ttl=%s, p95_ttl=%s, waste=%μUSD)',
        v_total, v_ttl_expired, ROUND(100.0 * v_ttl_expired / v_total, 0),
        v_median_ttl, v_p95_ttl, v_waste;
END $$;

ROLLBACK;
