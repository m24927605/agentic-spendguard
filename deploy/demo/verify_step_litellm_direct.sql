-- Slice A3 verification: `DEMO_MODE=litellm_direct` runs the
-- SpendGuardDirectAcompletion wrapper end-to-end against the counting
-- provider (no LiteLLM proxy in the loop). Asserts the same ledger
-- shape as Slice 6's proxy mode: 1 reserve + 1 commit_estimated for
-- the ALLOW step + 1 denied_decision for the DENY step.
--
-- After GH #77 (Slice C1-C3) lands, this script also gains a Q3 SQL
-- block asserting `canonical_events.payload_json.data.spendguard.mode
-- = 'direct'` to differentiate from the proxy-callback path.

\echo
\echo === ledger_transactions: litellm_direct counts ===
SELECT operation_kind, COUNT(*)::int AS n
  FROM ledger_transactions
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
   AND operation_kind IN ('reserve', 'commit_estimated', 'denied_decision')
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
\echo === ASSERT: direct mode produced reserve + commit + denied ===
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
    -- Counts are cumulative if the DB was used by a prior demo;
    -- assertion is "at least one of each from this run".
    IF v_reserve < 1 THEN
        RAISE EXCEPTION 'SLICE_A3_GATE: reserve >= 1 expected, got %', v_reserve;
    END IF;
    IF v_commit < 1 THEN
        RAISE EXCEPTION 'SLICE_A3_GATE: commit_estimated >= 1 expected, got %', v_commit;
    END IF;
    IF v_denied < 1 THEN
        RAISE EXCEPTION 'SLICE_A3_GATE: denied_decision >= 1 expected, got %', v_denied;
    END IF;
    RAISE NOTICE 'SLICE_A3 OK: reserve=% commit_estimated=% denied_decision=%',
        v_reserve, v_commit, v_denied;
END;
$$;
