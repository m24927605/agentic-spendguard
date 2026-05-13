-- =====================================================================
-- Cost Advisor P0.6 — view smoke tests
-- =====================================================================
--
-- Three fixture scenarios verify reservations_with_ttl_status_v1 derives
-- state + ttl_seconds correctly. Each fixture wraps in SAVEPOINT +
-- ROLLBACK TO so state isn't persisted. The deferred audit-invariant
-- trigger (services/ledger/migrations/0011_immutability_triggers.sql,
-- "no audit, no effect") is DEFERRABLE INITIALLY DEFERRED — it fires
-- at outermost COMMIT only. Parent BEGIN below ends in ROLLBACK so the
-- trigger never runs against fixture rows.
--
-- FK ordering: audit_outbox.ledger_transaction_id FKs ledger_transactions
-- (checked at INSERT time, NOT deferred). So insert ledger_transactions
-- FIRST in each fixture, then the audit_outbox row that references it.
--
-- Invoke from a host shell:
--   docker exec -i $PG psql -U spendguard -d spendguard_ledger \
--       -v ON_ERROR_STOP=1 -f verify_p0_6_view.sql

\set ON_ERROR_STOP 1

-- Prereqs (ledger_units, ledger_shards, fencing_scopes) are seeded by
-- deploy/demo/init/migrations/30_seed_demo_state.sh. Required IDs:
--   tenant_id          = '00000000-0000-4000-8000-000000000001'
--   budget_id          = '44444444-4444-4444-8444-444444444444'
--   window_instance_id = '55555555-5555-4555-8555-555555555555'
--   fencing_scope_id   = '33333333-3333-4333-8333-333333333333'

BEGIN;

-- =====================================================================
-- Fixture 1: TTL_EXPIRED → derived_state='ttl_expired'
-- =====================================================================
SAVEPOINT fixture_1;

-- Reserve tx FIRST (audit_outbox FKs it).
INSERT INTO ledger_transactions (
    ledger_transaction_id, tenant_id, operation_kind, posting_state,
    idempotency_key, request_hash, lock_order_token, decision_id,
    audit_decision_event_id, effective_at, fencing_scope_id, fencing_epoch_at_post
) VALUES (
    'aaaa1111-0000-7000-8000-000000000001'::uuid,
    '00000000-0000-4000-8000-000000000001', 'reserve', 'posted',
    'reserve-key-fixture-1', '\x00'::bytea, 'lock-1',
    'dec00001-0000-7000-8000-000000000001'::uuid,
    'aad00001-0000-7000-8000-000000000001'::uuid,
    now() - interval '10 minutes',
    '33333333-3333-4333-8333-333333333333', 1
);

INSERT INTO audit_outbox (
    audit_outbox_id, audit_decision_event_id, decision_id, tenant_id,
    ledger_transaction_id, event_type, cloudevent_payload,
    cloudevent_payload_signature, ledger_fencing_epoch, workload_instance_id,
    recorded_at, recorded_month, producer_sequence, idempotency_key
) VALUES (
    'aaaa0001-0000-7000-8000-000000000001'::uuid,
    'aad00001-0000-7000-8000-000000000001'::uuid,
    'dec00001-0000-7000-8000-000000000001'::uuid,
    '00000000-0000-4000-8000-000000000001',
    'aaaa1111-0000-7000-8000-000000000001'::uuid,
    'spendguard.audit.decision',
    ('{"specversion":"1.0","type":"spendguard.audit.decision","data_b64":"' ||
        encode('{"kind":"reserve"}'::bytea, 'base64') || '"}')::jsonb,
    '\x00'::bytea, 1, 'demo-sidecar',
    now() - interval '10 minutes', '2026-05-01', 1, 'reserve-key-fixture-1'
);

-- Reservation in 'released' state with TTL passed.
INSERT INTO reservations (
    reservation_id, tenant_id, budget_id, window_instance_id, current_state,
    source_ledger_transaction_id, ttl_expires_at, idempotency_key, created_at
) VALUES (
    'cccc0001-0000-7000-8000-000000000001'::uuid,
    '00000000-0000-4000-8000-000000000001',
    '44444444-4444-4444-8444-444444444444',
    '55555555-5555-4555-8555-555555555555',
    'released',
    'aaaa1111-0000-7000-8000-000000000001'::uuid,
    now() - interval '5 minutes',
    'reserve-key-fixture-1',
    now() - interval '10 minutes'   -- 5 min TTL window
);

-- Release tx with TTL_EXPIRED reason (separate ledger_transaction).
INSERT INTO ledger_transactions (
    ledger_transaction_id, tenant_id, operation_kind, posting_state,
    idempotency_key, request_hash, lock_order_token, decision_id,
    audit_decision_event_id, effective_at, fencing_scope_id, fencing_epoch_at_post
) VALUES (
    'bbbb1111-0000-7000-8000-000000000001'::uuid,
    '00000000-0000-4000-8000-000000000001', 'release', 'posted',
    'release-key-fixture-1', '\x01'::bytea, 'lock-2',
    'dec00001-0000-7000-8000-000000000001'::uuid,
    'aae00001-0000-7000-8000-000000000001'::uuid,
    now() - interval '5 minutes',
    '33333333-3333-4333-8333-333333333333', 1
);

INSERT INTO audit_outbox (
    audit_outbox_id, audit_decision_event_id, decision_id, tenant_id,
    ledger_transaction_id, event_type, cloudevent_payload,
    cloudevent_payload_signature, ledger_fencing_epoch, workload_instance_id,
    recorded_at, recorded_month, producer_sequence, idempotency_key
) VALUES (
    'aaab0001-0000-7000-8000-000000000001'::uuid,
    'aae00001-0000-7000-8000-000000000001'::uuid,
    'dec00001-0000-7000-8000-000000000001'::uuid,
    '00000000-0000-4000-8000-000000000001',
    'bbbb1111-0000-7000-8000-000000000001'::uuid,
    'spendguard.audit.outcome',
    ('{"specversion":"1.0","type":"spendguard.audit.outcome","data_b64":"' ||
        encode('{"kind":"release","reason":"TTL_EXPIRED"}'::bytea, 'base64') ||
     '"}')::jsonb,
    '\x00'::bytea, 1, 'demo-ttl-sweeper',
    now() - interval '5 minutes', '2026-05-01', 2, 'release-key-fixture-1'
);

DO $$
DECLARE
    v_state TEXT;
    v_ttl   INT;
    v_reason TEXT;
BEGIN
    SELECT derived_state, ttl_seconds, release_reason
      INTO v_state, v_ttl, v_reason
      FROM reservations_with_ttl_status_v1
     WHERE reservation_id = 'cccc0001-0000-7000-8000-000000000001'::uuid;
    IF v_state <> 'ttl_expired' THEN
        RAISE EXCEPTION 'fixture_1 FAIL: derived_state expected ttl_expired, got %', v_state;
    END IF;
    IF v_ttl <> 300 THEN
        RAISE EXCEPTION 'fixture_1 FAIL: ttl_seconds expected 300 (5 min), got %', v_ttl;
    END IF;
    IF v_reason <> 'TTL_EXPIRED' THEN
        RAISE EXCEPTION 'fixture_1 FAIL: release_reason expected TTL_EXPIRED, got %', v_reason;
    END IF;
    RAISE NOTICE 'fixture_1: PASS (derived_state=%, ttl_seconds=%, reason=%)', v_state, v_ttl, v_reason;
END $$;

ROLLBACK TO SAVEPOINT fixture_1;

-- =====================================================================
-- Fixture 2: committed → derived_state='committed' (no release_reason)
-- =====================================================================
SAVEPOINT fixture_2;

INSERT INTO ledger_transactions (
    ledger_transaction_id, tenant_id, operation_kind, posting_state,
    idempotency_key, request_hash, lock_order_token, decision_id,
    audit_decision_event_id, effective_at, fencing_scope_id, fencing_epoch_at_post
) VALUES (
    'aaaa2222-0000-7000-8000-000000000001'::uuid,
    '00000000-0000-4000-8000-000000000001', 'reserve', 'posted',
    'reserve-key-fixture-2', '\x02'::bytea, 'lock-3',
    'dec00002-0000-7000-8000-000000000001'::uuid,
    'aad00002-0000-7000-8000-000000000001'::uuid,
    now() - interval '10 minutes',
    '33333333-3333-4333-8333-333333333333', 1
);

INSERT INTO audit_outbox (
    audit_outbox_id, audit_decision_event_id, decision_id, tenant_id,
    ledger_transaction_id, event_type, cloudevent_payload,
    cloudevent_payload_signature, ledger_fencing_epoch, workload_instance_id,
    recorded_at, recorded_month, producer_sequence, idempotency_key
) VALUES (
    'aaaa0002-0000-7000-8000-000000000001'::uuid,
    'aad00002-0000-7000-8000-000000000001'::uuid,
    'dec00002-0000-7000-8000-000000000001'::uuid,
    '00000000-0000-4000-8000-000000000001',
    'aaaa2222-0000-7000-8000-000000000001'::uuid,
    'spendguard.audit.decision',
    ('{"specversion":"1.0","type":"spendguard.audit.decision","data_b64":"' ||
        encode('{"kind":"reserve"}'::bytea, 'base64') || '"}')::jsonb,
    '\x00'::bytea, 1, 'demo-sidecar',
    now() - interval '10 minutes', '2026-05-01', 3, 'reserve-key-fixture-2'
);

INSERT INTO reservations (
    reservation_id, tenant_id, budget_id, window_instance_id, current_state,
    source_ledger_transaction_id, ttl_expires_at, idempotency_key, created_at
) VALUES (
    'cccc0002-0000-7000-8000-000000000001'::uuid,
    '00000000-0000-4000-8000-000000000001',
    '44444444-4444-4444-8444-444444444444',
    '55555555-5555-4555-8555-555555555555',
    'committed',
    'aaaa2222-0000-7000-8000-000000000001'::uuid,
    now() + interval '1 hour',
    'reserve-key-fixture-2',
    now() - interval '10 minutes'
);

DO $$
DECLARE
    v_state TEXT;
    v_reason TEXT;
BEGIN
    SELECT derived_state, release_reason
      INTO v_state, v_reason
      FROM reservations_with_ttl_status_v1
     WHERE reservation_id = 'cccc0002-0000-7000-8000-000000000001'::uuid;
    IF v_state <> 'committed' THEN
        RAISE EXCEPTION 'fixture_2 FAIL: derived_state expected committed, got %', v_state;
    END IF;
    IF v_reason IS NOT NULL THEN
        RAISE EXCEPTION 'fixture_2 FAIL: release_reason expected NULL, got %', v_reason;
    END IF;
    RAISE NOTICE 'fixture_2: PASS (derived_state=%, release_reason IS NULL)', v_state;
END $$;

ROLLBACK TO SAVEPOINT fixture_2;

-- =====================================================================
-- Fixture 3: explicit release (reason=RUN_ABORTED) → derived_state='released'
-- =====================================================================
SAVEPOINT fixture_3;

INSERT INTO ledger_transactions (
    ledger_transaction_id, tenant_id, operation_kind, posting_state,
    idempotency_key, request_hash, lock_order_token, decision_id,
    audit_decision_event_id, effective_at, fencing_scope_id, fencing_epoch_at_post
) VALUES (
    'aaaa3333-0000-7000-8000-000000000001'::uuid,
    '00000000-0000-4000-8000-000000000001', 'reserve', 'posted',
    'reserve-key-fixture-3', '\x03'::bytea, 'lock-4',
    'dec00003-0000-7000-8000-000000000001'::uuid,
    'aad00003-0000-7000-8000-000000000001'::uuid,
    now() - interval '10 minutes',
    '33333333-3333-4333-8333-333333333333', 1
);

INSERT INTO audit_outbox (
    audit_outbox_id, audit_decision_event_id, decision_id, tenant_id,
    ledger_transaction_id, event_type, cloudevent_payload,
    cloudevent_payload_signature, ledger_fencing_epoch, workload_instance_id,
    recorded_at, recorded_month, producer_sequence, idempotency_key
) VALUES (
    'aaaa0003-0000-7000-8000-000000000001'::uuid,
    'aad00003-0000-7000-8000-000000000001'::uuid,
    'dec00003-0000-7000-8000-000000000001'::uuid,
    '00000000-0000-4000-8000-000000000001',
    'aaaa3333-0000-7000-8000-000000000001'::uuid,
    'spendguard.audit.decision',
    ('{"specversion":"1.0","type":"spendguard.audit.decision","data_b64":"' ||
        encode('{"kind":"reserve"}'::bytea, 'base64') || '"}')::jsonb,
    '\x00'::bytea, 1, 'demo-sidecar',
    now() - interval '10 minutes', '2026-05-01', 4, 'reserve-key-fixture-3'
);

INSERT INTO reservations (
    reservation_id, tenant_id, budget_id, window_instance_id, current_state,
    source_ledger_transaction_id, ttl_expires_at, idempotency_key, created_at
) VALUES (
    'cccc0003-0000-7000-8000-000000000001'::uuid,
    '00000000-0000-4000-8000-000000000001',
    '44444444-4444-4444-8444-444444444444',
    '55555555-5555-4555-8555-555555555555',
    'released',
    'aaaa3333-0000-7000-8000-000000000001'::uuid,
    now() + interval '1 hour',  -- TTL hadn't expired
    'reserve-key-fixture-3',
    now() - interval '10 minutes'
);

INSERT INTO ledger_transactions (
    ledger_transaction_id, tenant_id, operation_kind, posting_state,
    idempotency_key, request_hash, lock_order_token, decision_id,
    audit_decision_event_id, effective_at, fencing_scope_id, fencing_epoch_at_post
) VALUES (
    'bbbb3333-0000-7000-8000-000000000001'::uuid,
    '00000000-0000-4000-8000-000000000001', 'release', 'posted',
    'release-key-fixture-3', '\x04'::bytea, 'lock-5',
    'dec00003-0000-7000-8000-000000000001'::uuid,
    'aae00003-0000-7000-8000-000000000001'::uuid,
    now() - interval '5 minutes',
    '33333333-3333-4333-8333-333333333333', 1
);

INSERT INTO audit_outbox (
    audit_outbox_id, audit_decision_event_id, decision_id, tenant_id,
    ledger_transaction_id, event_type, cloudevent_payload,
    cloudevent_payload_signature, ledger_fencing_epoch, workload_instance_id,
    recorded_at, recorded_month, producer_sequence, idempotency_key
) VALUES (
    'aaab0003-0000-7000-8000-000000000001'::uuid,
    'aae00003-0000-7000-8000-000000000001'::uuid,
    'dec00003-0000-7000-8000-000000000001'::uuid,
    '00000000-0000-4000-8000-000000000001',
    'bbbb3333-0000-7000-8000-000000000001'::uuid,
    'spendguard.audit.outcome',
    ('{"specversion":"1.0","type":"spendguard.audit.outcome","data_b64":"' ||
        encode('{"kind":"release","reason":"RUN_ABORTED"}'::bytea, 'base64') ||
     '"}')::jsonb,
    '\x00'::bytea, 1, 'demo-sidecar',
    now() - interval '5 minutes', '2026-05-01', 5, 'release-key-fixture-3'
);

DO $$
DECLARE
    v_state TEXT;
    v_reason TEXT;
BEGIN
    SELECT derived_state, release_reason
      INTO v_state, v_reason
      FROM reservations_with_ttl_status_v1
     WHERE reservation_id = 'cccc0003-0000-7000-8000-000000000001'::uuid;
    IF v_state <> 'released' THEN
        RAISE EXCEPTION 'fixture_3 FAIL: derived_state expected released (NOT ttl_expired), got %', v_state;
    END IF;
    IF v_reason <> 'RUN_ABORTED' THEN
        RAISE EXCEPTION 'fixture_3 FAIL: release_reason expected RUN_ABORTED, got %', v_reason;
    END IF;
    RAISE NOTICE 'fixture_3: PASS (derived_state=%, release_reason=%)', v_state, v_reason;
END $$;

ROLLBACK TO SAVEPOINT fixture_3;

SELECT 'all_fixtures: PASS' AS final_status;

ROLLBACK;   -- parent tx: leave no persistent state, no deferred trigger fires
