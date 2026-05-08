-- Phase 2B Step 7 verification: assert ledger_transactions has both a
-- `reserve` and a `commit_estimated` row for the demo decision_id, and
-- that the per-account balance derived from ledger_entries matches:
--   available_budget = 458 (= 500 seed - 42 committed)
--   reserved_hold = 0
--   committed_spend = 42

\echo
\echo === ledger_transactions: operation_kind counts ===
SELECT operation_kind, COUNT(*)::int AS n
  FROM ledger_transactions
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
   AND operation_kind IN ('reserve','commit_estimated')
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
\echo === commits projection ===
SELECT latest_state, estimated_amount_atomic, delta_to_reserved_atomic
  FROM commits
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
 ORDER BY created_at DESC
 LIMIT 5;

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
\echo === ASSERTIONS ===
DO $$
DECLARE
    v_reserve_count       INT;
    v_commit_count        INT;
    v_available_net       NUMERIC;
    v_reserved_hold_net   NUMERIC;
    v_committed_spend_net NUMERIC;
    v_committed_state_n   INT;
    v_decision_audit_n    INT;
    v_outcome_audit_n     INT;
BEGIN
    SELECT COUNT(*) INTO v_reserve_count
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'reserve';
    IF v_reserve_count <> 1 THEN
        RAISE EXCEPTION 'EXPECTED 1 reserve tx; got %', v_reserve_count;
    END IF;

    SELECT COUNT(*) INTO v_commit_count
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'commit_estimated';
    IF v_commit_count <> 1 THEN
        RAISE EXCEPTION 'EXPECTED 1 commit_estimated tx; got %', v_commit_count;
    END IF;

    -- per-account net_debit derivation. Account orientations:
    --   credit-positive: available_budget, reserved_hold, committed_spend
    --   so balance = -net_debit. We assert against derived balances.
    SELECT
        COALESCE(SUM(CASE WHEN la.account_kind = 'available_budget'
                          THEN (CASE WHEN le.direction='debit' THEN le.amount_atomic
                                     ELSE -le.amount_atomic END)
                          ELSE 0 END), 0),
        COALESCE(SUM(CASE WHEN la.account_kind = 'reserved_hold'
                          THEN (CASE WHEN le.direction='debit' THEN le.amount_atomic
                                     ELSE -le.amount_atomic END)
                          ELSE 0 END), 0),
        COALESCE(SUM(CASE WHEN la.account_kind = 'committed_spend'
                          THEN (CASE WHEN le.direction='debit' THEN le.amount_atomic
                                     ELSE -le.amount_atomic END)
                          ELSE 0 END), 0)
      INTO v_available_net, v_reserved_hold_net, v_committed_spend_net
      FROM ledger_entries le
      JOIN ledger_accounts la ON le.ledger_account_id = la.ledger_account_id
     WHERE le.tenant_id = '00000000-0000-4000-8000-000000000001'
       AND la.budget_id = '44444444-4444-4444-8444-444444444444'
       AND le.window_instance_id = '55555555-5555-4555-8555-555555555555'
       AND la.unit_id = '66666666-6666-4666-8666-666666666666';

    -- credit-positive accounts: balance = -net_debit
    IF -v_available_net <> 458 THEN
        RAISE EXCEPTION 'EXPECTED available_budget balance 458; got %', -v_available_net;
    END IF;
    IF -v_reserved_hold_net <> 0 THEN
        RAISE EXCEPTION 'EXPECTED reserved_hold balance 0; got %', -v_reserved_hold_net;
    END IF;
    IF -v_committed_spend_net <> 42 THEN
        RAISE EXCEPTION 'EXPECTED committed_spend balance 42; got %', -v_committed_spend_net;
    END IF;

    SELECT COUNT(*) INTO v_committed_state_n
      FROM reservations
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND budget_id = '44444444-4444-4444-8444-444444444444'
       AND current_state = 'committed';
    IF v_committed_state_n <> 1 THEN
        RAISE EXCEPTION 'EXPECTED 1 committed reservation; got %', v_committed_state_n;
    END IF;

    SELECT COUNT(*) INTO v_decision_audit_n
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND event_type = 'spendguard.audit.decision';
    SELECT COUNT(*) INTO v_outcome_audit_n
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND event_type = 'spendguard.audit.outcome';
    -- Expect at least 2 decision events (seed deposit + reserve) and 1 outcome
    -- (commit_estimated). agent-mode adds another reserve+commit pair so the
    -- assertions here use >=.
    IF v_decision_audit_n < 2 THEN
        RAISE EXCEPTION 'EXPECTED >=2 audit.decision events; got %', v_decision_audit_n;
    END IF;
    IF v_outcome_audit_n < 1 THEN
        RAISE EXCEPTION 'EXPECTED >=1 audit.outcome event; got %', v_outcome_audit_n;
    END IF;

    RAISE NOTICE 'Phase 2B Step 7 assertions PASS (available=458, reserved=0, committed=42)';
END
$$;
