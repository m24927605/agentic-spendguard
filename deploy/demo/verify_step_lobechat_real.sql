-- Verification SQL for DEMO_MODE=lobechat_real (D34 SLICE 1).
--
-- Asserts that one chat sent from LobeChat through the SpendGuard
-- egress proxy produced one `reserve` row and one `commit_estimated`
-- row in the ledger audit chain in the last 10 minutes. Modelled on
-- `verify_step_anythingllm_real.sql` (D33) — same schema
-- (`ledger_transactions.operation_kind`, `tenant_id`, `event_time`),
-- same 10-minute window so a re-run with stale rows does not
-- false-positive.
--
-- Tenant UUID is the demo seed planted by `30_seed_demo_state.sh`
-- and matched by the egress proxy's `SPENDGUARD_PROXY_DEFAULT_TENANT_ID`.
--
-- D34 anti-acceptance A1 / A2 / A7 hinge on this assertion: if
-- OPENAI_PROXY_URL was silently dropped at boot, LobeChat calls
-- api.openai.com directly and this SQL stays empty (raising
-- D34_GATE).

\set ON_ERROR_STOP on

\echo
\echo === ledger_transactions: LobeChat smoke (last 10m) ===
SELECT operation_kind, COUNT(*)::int AS n
  FROM ledger_transactions
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
   AND operation_kind IN ('reserve','commit_estimated')
   AND event_time > now() - interval '10 minutes'
 GROUP BY operation_kind
 ORDER BY operation_kind;

\echo
\echo === ASSERT: LobeChat server-route chat produced reserve + commit_estimated ===
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
    -- to surface a wire-level break (e.g. OPENAI_PROXY_URL dropped at
    -- container boot, or LobeChat upstream renaming the env var).
    IF v_reserve < 1 THEN
        RAISE EXCEPTION 'D34_GATE: ledger_transactions.reserve >= 1 expected, got % (LobeChat call did not reach the SpendGuard egress proxy - OPENAI_PROXY_URL likely dropped at boot)', v_reserve;
    END IF;
    IF v_commit < 1 THEN
        RAISE EXCEPTION 'D34_GATE: ledger_transactions.commit_estimated >= 1 expected, got % (upstream provider responded but commit lane did not fire)', v_commit;
    END IF;

    RAISE NOTICE 'D34 LEDGER OK: reserve=% commit_estimated=%', v_reserve, v_commit;
END $$;
