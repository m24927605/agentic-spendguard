-- TTL Sweeper verification.
-- After: reserve(100) with TTL=5s → wait 12s → ttl-sweeper auto-releases
-- Expected:
--   1 reserve tx + 1 release tx (operation_kind='release')
--   reservations.current_state = 'released'
--   per-account: available=500 (full refund), reserved_hold=0
--   audit_outbox: 2 audit.decision (deposit + reserve) + 1 audit.outcome (release)
--   audit_outbox.cloudevent_payload data_b64 decoded JSON has reason='TTL_EXPIRED'
--   ledger_transactions.fencing_scope_id of release tx = ...060 (ttl-sweeper scope)

\echo
\echo === ledger_transactions: operation_kind counts ===
SELECT operation_kind, COUNT(*)::int AS n
  FROM ledger_transactions
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
   AND operation_kind IN ('reserve','release','adjustment','commit_estimated')
 GROUP BY operation_kind
 ORDER BY operation_kind;

\echo
\echo === reservations.current_state ===
SELECT current_state, COUNT(*)::int AS n
  FROM reservations
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
   AND budget_id = '44444444-4444-4444-8444-444444444444'
 GROUP BY current_state
 ORDER BY current_state;

\echo
\echo === per-account net (entries-derived) ===
SELECT la.account_kind,
       COALESCE(
         SUM(CASE WHEN le.direction='debit' THEN le.amount_atomic
                  WHEN le.direction='credit' THEN -le.amount_atomic END),
         0)::TEXT AS net_debit
  FROM ledger_entries le
  JOIN ledger_accounts la ON le.ledger_account_id = la.ledger_account_id
 WHERE le.tenant_id = '00000000-0000-4000-8000-000000000001'
   AND la.budget_id = '44444444-4444-4444-8444-444444444444'
   AND le.window_instance_id = '55555555-5555-4555-8555-555555555555'
   AND la.unit_id = '66666666-6666-4666-8666-666666666666'
 GROUP BY la.account_kind
 ORDER BY la.account_kind;

\echo
\echo === audit_outbox event_type counts ===
SELECT event_type, COUNT(*)::int AS n
  FROM audit_outbox
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
 GROUP BY event_type
 ORDER BY event_type;

\echo
\echo === release audit.outcome data_b64 decoded reason ===
SELECT
    a.event_type,
    convert_from(decode(a.cloudevent_payload->>'data_b64', 'base64'), 'UTF8')::JSONB AS payload
  FROM audit_outbox a
  JOIN ledger_transactions lt ON a.ledger_transaction_id = lt.ledger_transaction_id
 WHERE a.tenant_id = '00000000-0000-4000-8000-000000000001'
   AND lt.operation_kind = 'release';

\echo
\echo === ASSERTIONS ===
DO $$
DECLARE
    v_reserve_count       INT;
    v_release_count       INT;
    v_available_net       NUMERIC;
    v_reserved_hold_net   NUMERIC;
    v_released_state_n    INT;
    v_decision_audit_n    INT;
    v_outcome_audit_n     INT;
    v_release_reason      TEXT;
    v_release_scope_id    UUID;
BEGIN
    SELECT COUNT(*) INTO v_reserve_count
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'reserve';
    IF v_reserve_count <> 1 THEN
        RAISE EXCEPTION 'EXPECTED 1 reserve tx; got %', v_reserve_count;
    END IF;

    SELECT COUNT(*) INTO v_release_count
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'release';
    IF v_release_count <> 1 THEN
        RAISE EXCEPTION 'EXPECTED 1 release tx; got %', v_release_count;
    END IF;

    SELECT
        COALESCE(SUM(CASE WHEN la.account_kind = 'available_budget'
                          THEN (CASE WHEN le.direction='debit' THEN le.amount_atomic
                                     ELSE -le.amount_atomic END)
                          ELSE 0 END), 0),
        COALESCE(SUM(CASE WHEN la.account_kind = 'reserved_hold'
                          THEN (CASE WHEN le.direction='debit' THEN le.amount_atomic
                                     ELSE -le.amount_atomic END)
                          ELSE 0 END), 0)
      INTO v_available_net, v_reserved_hold_net
      FROM ledger_entries le
      JOIN ledger_accounts la ON le.ledger_account_id = la.ledger_account_id
     WHERE le.tenant_id = '00000000-0000-4000-8000-000000000001'
       AND la.budget_id = '44444444-4444-4444-8444-444444444444'
       AND le.window_instance_id = '55555555-5555-4555-8555-555555555555'
       AND la.unit_id = '66666666-6666-4666-8666-666666666666';

    IF -v_available_net <> 500 THEN
        RAISE EXCEPTION 'EXPECTED available_budget balance 500 (full refund); got %', -v_available_net;
    END IF;
    IF -v_reserved_hold_net <> 0 THEN
        RAISE EXCEPTION 'EXPECTED reserved_hold balance 0; got %', -v_reserved_hold_net;
    END IF;

    SELECT COUNT(*) INTO v_released_state_n
      FROM reservations
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND budget_id = '44444444-4444-4444-8444-444444444444'
       AND current_state = 'released';
    IF v_released_state_n <> 1 THEN
        RAISE EXCEPTION 'EXPECTED 1 released reservation; got %', v_released_state_n;
    END IF;

    SELECT COUNT(*) INTO v_decision_audit_n
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND event_type = 'spendguard.audit.decision';
    SELECT COUNT(*) INTO v_outcome_audit_n
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND event_type = 'spendguard.audit.outcome';
    -- 3 audit.decision (deposit_token + deposit_usd + reserve) + 1 audit.outcome (release)
    -- (Phase 4 O4 added the USD opening deposit; baseline used to be 2.)
    IF v_decision_audit_n <> 3 THEN
        RAISE EXCEPTION 'EXPECTED 3 audit.decision events; got %', v_decision_audit_n;
    END IF;
    IF v_outcome_audit_n <> 1 THEN
        RAISE EXCEPTION 'EXPECTED 1 audit.outcome event (release); got %', v_outcome_audit_n;
    END IF;

    -- Decode CloudEvent data_b64 to extract reason field.
    SELECT
        (convert_from(decode(a.cloudevent_payload->>'data_b64', 'base64'), 'UTF8')::JSONB
         ->>'reason')
      INTO v_release_reason
      FROM audit_outbox a
      JOIN ledger_transactions lt ON a.ledger_transaction_id = lt.ledger_transaction_id
     WHERE a.tenant_id = '00000000-0000-4000-8000-000000000001'
       AND lt.operation_kind = 'release'
     LIMIT 1;
    IF v_release_reason IS DISTINCT FROM 'TTL_EXPIRED' THEN
        RAISE EXCEPTION 'EXPECTED release CloudEvent.data.reason=TTL_EXPIRED; got %', v_release_reason;
    END IF;

    -- Defense in depth: release tx fencing_scope_id MUST be ttl-sweeper's
    -- (...060), NOT sidecar's (...333) or webhook receiver's (...050).
    SELECT fencing_scope_id INTO v_release_scope_id
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'release'
     LIMIT 1;
    IF v_release_scope_id IS DISTINCT FROM '00000000-0000-7000-a000-000000000060'::UUID THEN
        RAISE EXCEPTION 'EXPECTED release fencing_scope_id=...060 (ttl-sweeper); got %', v_release_scope_id;
    END IF;

    RAISE NOTICE 'TTL Sweeper assertions PASS (1 reserve + 1 release; available=500 full refund; reservation released; reason=TTL_EXPIRED via ttl-sweeper scope)';
END
$$;
