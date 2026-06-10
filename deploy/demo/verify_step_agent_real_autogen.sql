-- COV_D24 SLICE 5 (agent_real_autogen demo) — ledger-DB assertions.
--
-- Mirrors verify_step_agent_real_agno.sql: ledger-side gates only.
-- The Makefile target `demo-verify-agent-real-autogen` runs this file
-- against spendguard_ledger, then issues a second
-- `psql -d spendguard_canonical` block for the cross-DB
-- canonical_events check (decision/outcome counts) + the
-- decision_context_json->>'integration' = 'autogen' tag.
--
-- Review-standards §7.1 + acceptance §3 gates:
--   - reserve >= 1 (ALLOW path: SpendGuardChatCompletionClient
--     reserves via request_decision before inner.create fires)
--   - commit_estimated >= 1 (ALLOW path: wrapper's POST commits with
--     real CreateResult.usage.prompt_tokens + completion_tokens)
--   - denied_decision >= 0 (DENY variant optional — surfaces via the
--     wrapper raising DecisionDenied directly out of create() — no
--     framework-side catch in either lineage means the raise reaches
--     the AssistantAgent caller cleanly; counting-stub hits stay flat).
--
-- INV-2 strict-order proof: the runner-side observation already proves
-- the live ordering (the model is NEVER called on the DENY path because
-- the wrapper raises DecisionDenied BEFORE inner.create — verified by
-- counting-stub hits staying flat on the DENY turn). Here in the
-- ledger we complement with a DB-side assertion that the EARLIEST
-- reserve row in this demo run predates the EARLIEST commit_estimated
-- row.

\echo
\echo === ledger_transactions: operation_kind counts (agent_real_autogen) ===
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
        RAISE EXCEPTION 'COV_D24_GATE: ledger_transactions.reserve >= 1 expected (ALLOW), got %', v_reserve;
    END IF;
    IF v_commit < 1 THEN
        RAISE EXCEPTION 'COV_D24_GATE: ledger_transactions.commit_estimated >= 1 expected (ALLOW), got %', v_commit;
    END IF;

    RAISE NOTICE 'COV_D24 LEDGER OK: reserve=% commit=% denied=%',
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
        RAISE NOTICE 'COV_D24 INV-2: insufficient rows for strict-order check (reserve=%, commit=%)',
            v_first_reserve, v_first_commit;
    ELSIF v_first_reserve > v_first_commit THEN
        RAISE EXCEPTION 'COV_D24 INV-2 VIOLATED: earliest reserve % > earliest commit %',
            v_first_reserve, v_first_commit;
    ELSE
        RAISE NOTICE 'COV_D24 INV-2 OK: reserve % predates commit %',
            v_first_reserve, v_first_commit;
    END IF;
END;
$$;

\echo
\echo === COV_D24 SLICE 5 agent_real_autogen verification done ===
