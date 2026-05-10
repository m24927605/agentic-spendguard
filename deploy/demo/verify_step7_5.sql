-- Phase 2B Step 7.5 verification (release-mode).
-- After: reserve(100) -> emit_llm_call_post(RUN_ABORTED) -> Release.
-- Expected:
--   available_budget = 500 (full refund of reserved 100)
--   reserved_hold    = 0
--   committed_spend  = 0 (never committed)
--   adjustment       = 500 (seed unchanged)
--   reservations.current_state = 'released'
--   commits = 0 rows (never committed)
--
-- audit_outbox events:
--   audit.decision: 3 (deposit_token + deposit_usd + reserve)
--                       — Phase 4 O4 added the USD opening deposit; the
--                       Phase 2B baseline (2 decisions) didn't track it.
--   audit.outcome:  1 (release; pairs with reserve's audit.decision)

\echo
\echo === ledger_transactions: operation_kind counts ===
SELECT operation_kind, COUNT(*)::int AS n
  FROM ledger_transactions
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
   AND operation_kind IN ('reserve','release','adjustment','commit_estimated','provider_report')
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
SELECT COUNT(*)::int AS commit_rows
  FROM commits
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001';

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
    v_release_count       INT;
    v_commit_count        INT;
    v_provider_count      INT;
    v_available_net       NUMERIC;
    v_reserved_hold_net   NUMERIC;
    v_committed_spend_net NUMERIC;
    v_released_state_n    INT;
    v_commit_rows         INT;
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

    SELECT COUNT(*) INTO v_release_count
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'release';
    IF v_release_count <> 1 THEN
        RAISE EXCEPTION 'EXPECTED 1 release tx; got %', v_release_count;
    END IF;

    SELECT COUNT(*) INTO v_commit_count
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'commit_estimated';
    IF v_commit_count <> 0 THEN
        RAISE EXCEPTION 'EXPECTED 0 commit_estimated tx; got %', v_commit_count;
    END IF;

    SELECT COUNT(*) INTO v_provider_count
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'provider_report';
    IF v_provider_count <> 0 THEN
        RAISE EXCEPTION 'EXPECTED 0 provider_report tx; got %', v_provider_count;
    END IF;

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

    IF -v_available_net <> 500 THEN
        RAISE EXCEPTION 'EXPECTED available_budget balance 500 (full refund); got %', -v_available_net;
    END IF;
    IF -v_reserved_hold_net <> 0 THEN
        RAISE EXCEPTION 'EXPECTED reserved_hold balance 0; got %', -v_reserved_hold_net;
    END IF;
    IF -v_committed_spend_net <> 0 THEN
        RAISE EXCEPTION 'EXPECTED committed_spend balance 0; got %', -v_committed_spend_net;
    END IF;

    SELECT COUNT(*) INTO v_released_state_n
      FROM reservations
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND budget_id = '44444444-4444-4444-8444-444444444444'
       AND current_state = 'released';
    IF v_released_state_n <> 1 THEN
        RAISE EXCEPTION 'EXPECTED 1 released reservation; got %', v_released_state_n;
    END IF;

    SELECT COUNT(*) INTO v_commit_rows
      FROM commits
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001';
    IF v_commit_rows <> 0 THEN
        RAISE EXCEPTION 'EXPECTED 0 commits projection rows; got %', v_commit_rows;
    END IF;

    SELECT COUNT(*) INTO v_decision_audit_n
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND event_type = 'spendguard.audit.decision';
    SELECT COUNT(*) INTO v_outcome_audit_n
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND event_type = 'spendguard.audit.outcome';
    IF v_decision_audit_n <> 3 THEN
        RAISE EXCEPTION 'EXPECTED 3 audit.decision events (deposit_token + deposit_usd + reserve); got %', v_decision_audit_n;
    END IF;
    IF v_outcome_audit_n <> 1 THEN
        RAISE EXCEPTION 'EXPECTED 1 audit.outcome event (release); got %', v_outcome_audit_n;
    END IF;

    RAISE NOTICE 'Phase 2B Step 7.5 assertions PASS (available=500, reserved=0, committed=0, reservations.current_state=released)';
END
$$;
