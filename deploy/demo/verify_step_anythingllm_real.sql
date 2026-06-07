-- Verification SQL for DEMO_MODE=anythingllm_real (D33 SLICE 1).
--
-- Asserts that one Workspace chat sent from AnythingLLM through the
-- SpendGuard egress proxy produced one `reserve` row and one
-- `commit_estimated` row in the ledger audit chain in the last 10
-- minutes. Modelled on `verify_step_litellm_real.sql` — same schema
-- (`ledger_transactions.operation_kind`, `tenant_id`, `event_time`).
--
-- Tenant UUID is the demo seed planted by `30_seed_demo_state.sh`
-- and matched by the egress proxy's `SPENDGUARD_PROXY_DEFAULT_TENANT_ID`.

\set ON_ERROR_STOP on

\echo
\echo === ledger_transactions: AnythingLLM smoke (last 10m) ===
SELECT operation_kind, COUNT(*)::int AS n
  FROM ledger_transactions
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
   AND operation_kind IN ('reserve','commit_estimated')
   AND event_time > now() - interval '10 minutes'
 GROUP BY operation_kind
 ORDER BY operation_kind;

\echo
\echo === ASSERT: AnythingLLM Workspace chat produced reserve + commit_estimated ===
DO $$
DECLARE
    v_reserve INT;
    v_commit  INT;
BEGIN
    SELECT COUNT(*) INTO v_reserve
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'reserve'
       AND event_time > now() - interval '10 minutes';
    SELECT COUNT(*) INTO v_commit
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'commit_estimated'
       AND event_time > now() - interval '10 minutes';

    -- Fail-closed per the demo-as-quality-gate contract: the smoke
    -- shell script depends on this DO block raising on missing rows
    -- to surface a wire-level break (e.g. a Generic OpenAI provider
    -- update-env that silently drops GenericOpenAiBasePath).
    IF v_reserve < 1 THEN
        RAISE EXCEPTION 'D33_GATE: ledger_transactions.reserve >= 1 expected, got % (AnythingLLM call did not reach the SpendGuard egress proxy)', v_reserve;
    END IF;
    IF v_commit < 1 THEN
        RAISE EXCEPTION 'D33_GATE: ledger_transactions.commit_estimated >= 1 expected, got % (upstream provider responded but commit lane did not fire)', v_commit;
    END IF;

    RAISE NOTICE 'D33 LEDGER OK: reserve=% commit_estimated=%', v_reserve, v_commit;
END $$;
