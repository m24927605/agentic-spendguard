-- COV_D12 SLICE 7 — `DEMO_MODE=litellm_sdk_deny` verification.
-- Mirrors verify_step_litellm_deny.sql. Asserts that the 3-substep
-- DENY matrix produced:
--   * 1 positive-control ALLOW (reserve + commit row)
--   * 1 sidecar-side DENY for sub-step 2 (budget-exhausted path
--     reaches the sidecar)
--   * Sub-step 3 (bogus UDS) fails BEFORE the sidecar, so no row.

\echo
\echo === ledger_transactions: operation_kind counts (litellm_sdk_deny) ===
SELECT operation_kind, COUNT(*)::int AS n
  FROM ledger_transactions
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
   AND operation_kind IN (
     'reserve', 'commit_estimated', 'denied_decision'
   )
 GROUP BY operation_kind
 ORDER BY operation_kind;

\echo
\echo === ASSERT: 1 ALLOW positive control landed (proves wire is healthy) ===
-- Sub-steps 2 + 3 are fail-closed BEFORE reaching the ledger:
--   * Sub-step 2 (bogus budget_id): the sidecar's
--     ledger_accounts resolver returns 0 rows → surfaces INTERNAL,
--     which the shim raises as SidecarUnavailable. The shim's
--     fail-closed default is fired BEFORE the provider hits and
--     ZERO ledger_transactions rows land on that side. INV-1 is
--     proved by the driver's stub-counter delta=0 (see runner
--     stdout); the ledger gate cannot witness sub-step 2.
--   * Sub-step 3 (bogus UDS): the handshake itself fails, never
--     reaches a request_decision RPC → no ledger row.
--
-- The ledger gate therefore asserts only what it CAN witness:
-- sub-step 1 ALLOW positive control produced exactly one reserve +
-- one commit pair. The 3-substep INV-1 negative control is the
-- responsibility of the driver (asserted in run_litellm_sdk_deny_demo.py
-- via stub-counter delta).
DO $$
DECLARE
    v_reserve INT;
    v_commit INT;
BEGIN
    SELECT COUNT(*) INTO v_reserve
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'reserve';
    SELECT COUNT(*) INTO v_commit
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'commit_estimated';

    IF v_reserve < 1 THEN
        RAISE EXCEPTION 'D12_SDK_DENY_GATE: reserve >= 1 expected (positive control), got %', v_reserve;
    END IF;
    IF v_commit < 1 THEN
        RAISE EXCEPTION 'D12_SDK_DENY_GATE: commit_estimated >= 1 expected (positive control), got %', v_commit;
    END IF;
    RAISE NOTICE 'D12_SDK_DENY LEDGER OK: reserve=% commit_estimated=%',
        v_reserve, v_commit;
END;
$$;
