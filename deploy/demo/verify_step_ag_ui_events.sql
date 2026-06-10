-- COV_D39 SLICE 3 (ag_ui_events demo) — ledger-DB assertions.
--
-- House-style ledger gates (mirrors verify_step_langchain_ts.sql):
-- the AG-UI events are display-only, so what we assert here is that the
-- REAL enforcement-plane operations the demo rendered actually landed in
-- the ledger:
--   - reserve >= 1          (the ALLOW step's reservation)
--   - commit_estimated >= 1 (the ALLOW step's commit)
--   - denied_decision >= 1  (the DENY step — short-circuited at the
--     sidecar before the provider call; the AG-UI decision.denied event
--     merely reports it)
-- Counts use `>=` per the SQL-gate robustness convention (demo-mode
-- retries / prior-state bleed-through). The exact-sequence assertion
-- lives in verify_sse.py (exactly 4 frames); the SSE↔ledger
-- reservation_id join lives in the Makefile target.
--
-- Additionally: the budget.snapshot event's remaining_atomic comes from
-- SPENDGUARD_DEMO_OPENING_BALANCE_ATOMIC=500 in the overlay env — we
-- cross-check that against the actual seeded opening deposit
-- (deploy/demo/init/migrations/30_seed_demo_state.sh, ledger_entry_id
-- 00000000-0000-7000-a000-000000000030) so the snapshot is provably not
-- a fabricated number (design.md §9.2).

\echo
\echo === ledger_transactions: operation_kind counts (ag_ui_events) ===
SELECT operation_kind, COUNT(*)::int AS n
  FROM ledger_transactions
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
   AND operation_kind IN (
     'reserve', 'commit_estimated', 'denied_decision'
   )
 GROUP BY operation_kind
 ORDER BY operation_kind;

\echo
\echo === reservations.current_state (demo budget) ===
SELECT current_state, COUNT(*)::int AS n
  FROM reservations
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
   AND budget_id = '44444444-4444-4444-8444-444444444444'
 GROUP BY current_state
 ORDER BY current_state;

\echo
\echo === ASSERT: ALLOW (reserve + commit) + DENY produced ledger rows ===
DO $$
DECLARE
    v_reserve INT;
    v_commit INT;
    v_denied INT;
BEGIN
    SELECT COUNT(*) INTO v_reserve
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'reserve';
    SELECT COUNT(*) INTO v_commit
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'commit_estimated';
    SELECT COUNT(*) INTO v_denied
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'denied_decision';

    IF v_reserve < 1 THEN
        RAISE EXCEPTION 'COV_D39_GATE: ledger_transactions.reserve >= 1 expected (ALLOW step), got %', v_reserve;
    END IF;
    IF v_commit < 1 THEN
        RAISE EXCEPTION 'COV_D39_GATE: ledger_transactions.commit_estimated >= 1 expected (ALLOW step), got %', v_commit;
    END IF;
    IF v_denied < 1 THEN
        RAISE EXCEPTION 'COV_D39_GATE: ledger_transactions.denied_decision >= 1 expected (DENY step), got %', v_denied;
    END IF;

    RAISE NOTICE 'COV_D39 LEDGER OK: reserve=% commit_estimated=% denied_decision=%',
        v_reserve, v_commit, v_denied;
END;
$$;

\echo
\echo === ASSERT: budget.snapshot opening balance matches the seeded deposit ===
-- The snapshot event's remaining_atomic="500" is the overlay env value
-- SPENDGUARD_DEMO_OPENING_BALANCE_ATOMIC. Here we prove the seed really
-- credited exactly that amount to available_budget for the demo
-- (tenant, budget, window, unit) tuple — the display event describes
-- real ledger state, not a made-up figure.
DO $$
DECLARE
    v_amount NUMERIC;
BEGIN
    SELECT amount_atomic INTO v_amount
      FROM ledger_entries
     WHERE ledger_entry_id = '00000000-0000-7000-a000-000000000030'
       AND tenant_id = '00000000-0000-4000-8000-000000000001'
       AND budget_id = '44444444-4444-4444-8444-444444444444'
       AND window_instance_id = '55555555-5555-4555-8555-555555555555'
       AND unit_id = '66666666-6666-4666-8666-666666666666'
       AND direction = 'credit';

    IF v_amount IS NULL THEN
        RAISE EXCEPTION 'COV_D39_GATE: seeded opening-deposit ledger entry not found (30_seed_demo_state.sh)';
    END IF;
    IF v_amount <> 500 THEN
        RAISE EXCEPTION 'COV_D39_GATE: seeded opening deposit = % but the demo snapshot advertises 500 — overlay env SPENDGUARD_DEMO_OPENING_BALANCE_ATOMIC drifted from the seed', v_amount;
    END IF;
    RAISE NOTICE 'COV_D39 SNAPSHOT OK: seeded opening deposit = % (matches SPENDGUARD_DEMO_OPENING_BALANCE_ATOMIC)', v_amount;
END;
$$;
