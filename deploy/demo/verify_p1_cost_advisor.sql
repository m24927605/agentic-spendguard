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
BEGIN
    -- Mirror of services/cost_advisor/rules/detected_waste/
    -- idle_reservation_rate_v1.sql with concrete bindings for the
    -- fixture tenant + date.
    SELECT
        COUNT(*)::BIGINT,
        COUNT(*) FILTER (WHERE v.derived_state = 'ttl_expired')::BIGINT,
        PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY v.ttl_seconds)::INT,
        PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY v.ttl_seconds)::INT,
        (
            SELECT array_agg(decision_id ORDER BY released_at DESC)
              FROM (
                  SELECT lt.decision_id, v2.released_at
                    FROM reservations_with_ttl_status_v1 v2
                    JOIN ledger_transactions lt
                      ON lt.ledger_transaction_id = v2.source_ledger_transaction_id
                     AND lt.operation_kind = 'reserve'
                   WHERE v2.tenant_id = '00000000-0000-4000-8000-000000000001'
                     AND v2.derived_state = 'ttl_expired'
                     AND v2.created_at >= '2026-05-13'::date
                     AND v2.created_at < ('2026-05-13'::date + INTERVAL '1 day')
                     AND lt.decision_id IS NOT NULL
                   ORDER BY v2.released_at DESC
                   LIMIT 5
              ) sample
        )
      INTO v_total, v_ttl_expired, v_median_ttl, v_p95_ttl, v_sample
      FROM reservations_with_ttl_status_v1 v
      JOIN ledger_transactions reserve_tx
        ON reserve_tx.ledger_transaction_id = v.source_ledger_transaction_id
       AND reserve_tx.operation_kind = 'reserve'
     WHERE v.tenant_id = '00000000-0000-4000-8000-000000000001'
       AND v.created_at >= '2026-05-13'::date
       AND v.created_at < ('2026-05-13'::date + INTERVAL '1 day')
     GROUP BY v.tenant_id
    HAVING
        COUNT(*) >= 5
        AND (COUNT(*) FILTER (WHERE v.derived_state = 'ttl_expired')::NUMERIC
             / NULLIF(COUNT(*), 0)) > 0.20
        AND PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY v.ttl_seconds) <= 60;

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
        RAISE EXCEPTION 'p1_fixture FAIL: expected 5 sample decision_ids, got %',
            array_length(v_sample, 1);
    END IF;
    -- Codex CA-P1 r1 P1 verify: sampled IDs must be decision_ids (UUIDs
    -- that correspond to canonical_events.decision_id), not
    -- reservation_ids. Check that each ID resolves to a ledger_transactions
    -- row with operation_kind='reserve' for this tenant.
    IF NOT (
        SELECT bool_and(
            EXISTS (
                SELECT 1 FROM ledger_transactions lt
                 WHERE lt.decision_id = id
                   AND lt.operation_kind = 'reserve'
                   AND lt.tenant_id = '00000000-0000-4000-8000-000000000001'
            )
        )
        FROM unnest(v_sample) AS id
    ) THEN
        RAISE EXCEPTION
            'p1_fixture FAIL: sample contains IDs that are NOT canonical reserve decision_ids';
    END IF;

    RAISE NOTICE
        'p1_fixture: PASS (total=% ttl_expired=% idle_ratio=%, median_ttl=% p95_ttl=% sample IDs are canonical reserve decision_ids)',
        v_total, v_ttl_expired, ROUND(100.0 * v_ttl_expired / v_total, 0),
        v_median_ttl, v_p95_ttl;
END $$;

-- =====================================================================
-- Edge case 1: below-threshold idle ratio → rule must NOT fire.
-- Insert 5 more reservations on 2026-05-12 (a different bucket) with
-- only 1 TTL'd (20% idle == threshold; rule fires on `> 20%` so equal
-- should NOT fire).
-- =====================================================================
SAVEPOINT edge_below_threshold;

DO $$
DECLARE
    v_i INT; v_decision_id UUID; v_reserve_tx_id UUID; v_release_tx_id UUID;
    v_audit_decision_id UUID; v_audit_outcome_id UUID; v_reservation_id UUID;
    v_created_at TIMESTAMPTZ; v_ttl_expires_at TIMESTAMPTZ; v_seq BIGINT := 500;
BEGIN
    FOR v_i IN 1..5 LOOP
        v_decision_id := gen_random_uuid();
        v_reserve_tx_id := gen_random_uuid();
        v_release_tx_id := gen_random_uuid();
        v_audit_decision_id := gen_random_uuid();
        v_audit_outcome_id := gen_random_uuid();
        v_reservation_id := gen_random_uuid();
        v_created_at := '2026-05-12 12:00:00+00'::timestamptz + (v_i * INTERVAL '1 minute');
        v_ttl_expires_at := v_created_at + INTERVAL '30 seconds';

        INSERT INTO ledger_transactions (
            ledger_transaction_id, tenant_id, operation_kind, posting_state,
            idempotency_key, request_hash, lock_order_token, decision_id,
            audit_decision_event_id, effective_at, fencing_scope_id, fencing_epoch_at_post
        ) VALUES (
            v_reserve_tx_id, '00000000-0000-4000-8000-000000000001', 'reserve', 'posted',
            'edge-low-' || v_i, '\x00'::bytea, 'lock-low-' || v_i, v_decision_id,
            v_audit_decision_id, v_created_at,
            '33333333-3333-4333-8333-333333333333', 1);

        INSERT INTO audit_outbox (audit_outbox_id, audit_decision_event_id, decision_id, tenant_id,
            ledger_transaction_id, event_type, cloudevent_payload, cloudevent_payload_signature,
            ledger_fencing_epoch, workload_instance_id, recorded_at, recorded_month,
            producer_sequence, idempotency_key)
        VALUES (gen_random_uuid(), v_audit_decision_id, v_decision_id,
            '00000000-0000-4000-8000-000000000001', v_reserve_tx_id,
            'spendguard.audit.decision',
            ('{"data_b64":"' || encode('{"kind":"reserve"}'::bytea, 'base64') || '"}')::jsonb,
            '\x00'::bytea, 1, 'edge-fixture', v_created_at, '2026-05-01',
            v_seq, 'edge-low-' || v_i);
        v_seq := v_seq + 1;

        -- Only v_i=1 TTL'd (20% rate; 1/5 not > 20%, equal).
        INSERT INTO reservations (reservation_id, tenant_id, budget_id, window_instance_id,
            current_state, source_ledger_transaction_id, ttl_expires_at, idempotency_key, created_at)
        VALUES (v_reservation_id, '00000000-0000-4000-8000-000000000001',
            '44444444-4444-4444-8444-444444444444', '55555555-5555-4555-8555-555555555555',
            CASE WHEN v_i = 1 THEN 'released' ELSE 'committed' END,
            v_reserve_tx_id, v_ttl_expires_at, 'edge-low-' || v_i, v_created_at);

        IF v_i = 1 THEN
            INSERT INTO ledger_transactions (
                ledger_transaction_id, tenant_id, operation_kind, posting_state,
                idempotency_key, request_hash, lock_order_token, decision_id,
                audit_decision_event_id, effective_at, fencing_scope_id, fencing_epoch_at_post
            ) VALUES (
                v_release_tx_id, '00000000-0000-4000-8000-000000000001', 'release', 'posted',
                'edge-low-rel-' || v_i, '\x01'::bytea, 'lock-low-rel-' || v_i, v_decision_id,
                v_audit_outcome_id, v_ttl_expires_at,
                '33333333-3333-4333-8333-333333333333', 1);
            INSERT INTO audit_outbox (audit_outbox_id, audit_decision_event_id, decision_id, tenant_id,
                ledger_transaction_id, event_type, cloudevent_payload, cloudevent_payload_signature,
                ledger_fencing_epoch, workload_instance_id, recorded_at, recorded_month,
                producer_sequence, idempotency_key)
            VALUES (gen_random_uuid(), v_audit_outcome_id, v_decision_id,
                '00000000-0000-4000-8000-000000000001', v_release_tx_id,
                'spendguard.audit.outcome',
                ('{"data_b64":"' || encode('{"kind":"release","reason":"TTL_EXPIRED"}'::bytea, 'base64') || '"}')::jsonb,
                '\x00'::bytea, 1, 'edge-fixture', v_ttl_expires_at, '2026-05-01',
                v_seq, 'edge-low-rel-' || v_i);
            v_seq := v_seq + 1;
        END IF;
    END LOOP;
END $$;

DO $$
DECLARE v_fired BOOLEAN;
BEGIN
    SELECT EXISTS (
        SELECT 1
          FROM reservations_with_ttl_status_v1 v
          JOIN ledger_transactions reserve_tx
            ON reserve_tx.ledger_transaction_id = v.source_ledger_transaction_id
           AND reserve_tx.operation_kind = 'reserve'
         WHERE v.tenant_id = '00000000-0000-4000-8000-000000000001'
           AND v.created_at >= '2026-05-12'::date
           AND v.created_at < ('2026-05-12'::date + INTERVAL '1 day')
         GROUP BY v.tenant_id
        HAVING
            COUNT(*) >= 5
            AND (COUNT(*) FILTER (WHERE v.derived_state = 'ttl_expired')::NUMERIC
                 / NULLIF(COUNT(*), 0)) > 0.20
            AND PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY v.ttl_seconds) <= 60
    ) INTO v_fired;

    IF v_fired THEN
        RAISE EXCEPTION 'p1_edge_below_threshold FAIL: rule fired at exactly 20 percent idle ratio (expected suppression)';
    END IF;
    RAISE NOTICE 'p1_edge_below_threshold: PASS (rule correctly suppressed at 20 percent idle ratio)';
END $$;

ROLLBACK TO SAVEPOINT edge_below_threshold;

ROLLBACK;
