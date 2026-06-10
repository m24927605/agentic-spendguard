-- COV_D23 SLICE 4 (agent_real_beeai demo) — ledger-DB assertions.
--
-- Mirrors verify_step_agent_real_agno.sql: ledger-side gates only.
-- The Makefile target `demo-verify-agent-real-beeai` runs this file
-- against spendguard_ledger, then issues a second
-- `psql -d spendguard_canonical` block for the cross-DB
-- canonical_events check (decision/outcome counts) + the
-- decision_context_json->>'integration' = 'beeai' tag.
--
-- Review-standards §6 + acceptance §3 gates:
--   - reserve >= 1 (ALLOW path: subscribe_spendguard's `*.start`
--     handler reserves)
--   - commit_estimated >= 1 (ALLOW path: `*.success` handler commits)
--   - denied_decision >= 0 (DENY variant optional — surfaces via the
--     start handler raising DecisionDenied which BeeAI's Emitter wraps
--     as EmitterError preserving __cause__ — model HTTP never reached).
--
-- INV-2 strict-order proof: the runner-side observation already proves
-- the live ordering (the model is NEVER called on the DENY path
-- because the start handler awaits request_decision BEFORE any
-- provider call could be scheduled — verified by counting-stub hits
-- staying flat on the DENY turn). Here in the ledger we complement
-- with a DB-side assertion that the EARLIEST reserve row in this demo
-- run predates the EARLIEST `commit_estimated` row.

\echo
\echo === ledger_transactions: operation_kind counts (agent_real_beeai) ===
SELECT operation_kind, COUNT(*)::int AS n
  FROM ledger_transactions
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
   AND operation_kind IN (
     'reserve', 'commit_estimated', 'denied_decision'
   )
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
\echo === commits: latest_state for the ALLOW step ===
SELECT latest_state, estimated_amount_atomic
  FROM commits
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
 ORDER BY created_at DESC
 LIMIT 5;

\echo
\echo === ASSERT: at least 1 ALLOW produced ledger rows ===
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

    -- ALLOW produces 1 reserve + 1 commit. Counts use `>=` so the gate
    -- is robust against demo-mode retries or any prior-state bleed-
    -- through from the base compose seed.
    IF v_reserve < 1 THEN
        RAISE EXCEPTION 'COV_D23_GATE: ledger_transactions.reserve >= 1 expected (ALLOW), got %', v_reserve;
    END IF;
    IF v_commit < 1 THEN
        RAISE EXCEPTION 'COV_D23_GATE: ledger_transactions.commit_estimated >= 1 expected (ALLOW), got %', v_commit;
    END IF;

    RAISE NOTICE 'COV_D23 LEDGER OK: reserve=% commit=% denied=%',
        v_reserve, v_commit, v_denied;
END;
$$;

\echo
\echo === INV-2 strict-order: earliest reserve predates earliest commit row ===
DO $$
DECLARE
    v_first_reserve TIMESTAMPTZ;
    v_first_commit TIMESTAMPTZ;
BEGIN
    SELECT MIN(recorded_at) INTO v_first_reserve
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'reserve';
    SELECT MIN(recorded_at) INTO v_first_commit
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'commit_estimated';

    IF v_first_reserve IS NULL OR v_first_commit IS NULL THEN
        RAISE NOTICE 'COV_D23 INV-2: insufficient rows for strict-order check (reserve=%, commit=%)',
            v_first_reserve, v_first_commit;
    ELSIF v_first_reserve > v_first_commit THEN
        RAISE EXCEPTION 'COV_D23 INV-2 VIOLATED: earliest reserve % > earliest commit %',
            v_first_reserve, v_first_commit;
    ELSE
        RAISE NOTICE 'COV_D23 INV-2 OK: reserve % predates commit %',
            v_first_reserve, v_first_commit;
    END IF;
END;
$$;

\echo
\echo === COV_D23 SLICE 4 agent_real_beeai verification done ===
