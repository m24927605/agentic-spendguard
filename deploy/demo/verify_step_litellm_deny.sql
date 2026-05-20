-- Slice 7 verification: `DEMO_MODE=litellm_deny` 3 sub-steps per
-- ACCEPTANCE.md §5.2. ALL sub-steps fail-closed → counting provider
-- 0 hits → ledger should record the deny audit chain.
--
-- Sub-step (a) budget-exhausted via hard-cap → `denied_decision`
-- ledger row (sidecar STOP path).
-- Sub-step (b) sidecar_offline → no ledger row (callback rejected
-- before request_decision); ALLOW positive-control still produced
-- reserve+commit_estimated.
-- Sub-step (c) resolver_none → no ledger row (callback rejected at
-- resolver step); ALLOW positive-control still produced reserve+
-- commit_estimated.
--
-- ALLOW positive-controls (3 total) produce 3 reserves + 3 commits.

\echo
\echo === SLICE7 LEDGER: counts (reserve/commit_estimated/denied_decision) ===
SELECT operation_kind, COUNT(*)::int AS n
  FROM ledger_transactions
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
   AND operation_kind IN (
     'reserve', 'commit_estimated', 'denied_decision'
   )
 GROUP BY operation_kind
 ORDER BY operation_kind;

\echo
\echo === ASSERT: 3 positive-control ALLOWs + 1 hard-cap DENY ===
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

    -- 3 positive-control ALLOWs produce 3 reserves + 3 commits.
    IF v_reserve < 3 THEN
        RAISE EXCEPTION 'SLICE7_GATE: reserve >= 3 expected (3 ALLOW positives), got %', v_reserve;
    END IF;
    IF v_commit < 3 THEN
        RAISE EXCEPTION 'SLICE7_GATE: commit_estimated >= 3 expected, got %', v_commit;
    END IF;
    -- Only sub-step (a) reaches the sidecar and triggers DENY; (b)
    -- and (c) are callback-side rejections without sidecar contact.
    IF v_denied < 1 THEN
        RAISE EXCEPTION 'SLICE7_GATE: denied_decision >= 1 expected (sub-step a), got %', v_denied;
    END IF;
    RAISE NOTICE 'SLICE7 LEDGER OK: reserve=% commit_estimated=% denied_decision=%',
        v_reserve, v_commit, v_denied;
END;
$$;
