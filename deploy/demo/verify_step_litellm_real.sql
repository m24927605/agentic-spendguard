-- Slice 6 verification queries for `DEMO_MODE=litellm_real` (steps 1+2
-- of the 4-step demo per ACCEPTANCE.md §5.1). Asserts that the LiteLLM
-- proxy callback drove a complete reserve→commit lifecycle for the
-- ALLOW step AND a deny path for the DENY step (no commit).
--
-- Ground-truth schema (verified by Slice 1 R3 Staff panel, see
-- slice-06.md inherited-findings table):
--   canonical_events: payload_json column has a `data_b64` field
--     containing the base64-encoded CloudEvent body; event_type is the
--     CloudEvent type string (e.g. "spendguard.audit.decision").
--   ledger_transactions.operation_kind IN ('reserve','commit_estimated',
--     'invoice_reconcile','denied_decision').
--   Time columns are `event_time` / `ingest_at` (NOT `recorded_at`).

\echo
\echo === ledger_transactions: operation_kind counts (litellm_real) ===
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
\echo === ALLOW step: commit row for the demo ALLOW call ===
SELECT latest_state, estimated_amount_atomic
  FROM commits
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
 ORDER BY created_at DESC
 LIMIT 5;

\echo
\echo === per-account net (entries-derived; ALLOW step committed) ===
SELECT la.account_kind,
       COALESCE(
         SUM(CASE WHEN le.direction='debit'  THEN le.amount_atomic
                  WHEN le.direction='credit' THEN -le.amount_atomic END),
         0)::TEXT AS net_atomic
  FROM ledger_entries le
  JOIN ledger_accounts la ON le.ledger_account_id = la.ledger_account_id
 WHERE le.tenant_id = '00000000-0000-4000-8000-000000000001'
   AND la.budget_id = '44444444-4444-4444-8444-444444444444'
 GROUP BY la.account_kind
 ORDER BY la.account_kind;

-- Slice 6 R1 Code Reviewer P1 fix: assert the demo actually produced
-- the expected ledger rows. Without explicit RAISE EXCEPTION the
-- script exits 0 on an empty result set, silently degrading the
-- "demo as quality gate" contract.
--
-- Slice 6 expectations (steps 1+2; ALLOW step = 1 reserve + 1
-- commit_estimated; DENY step is scope-cut to Slice 7's over-budget
-- seed per IMPLEMENTATION.md §920-924, so we do NOT assert
-- denied_decision >= 1 here):
\echo
\echo === ASSERT: ALLOW step produced reserve + commit_estimated ===
-- Slice 6 R2 P0-2 fix: `canonical_events` lives in the
-- `spendguard_canonical` DB, not `spendguard_ledger`. The cross-DB
-- assertions for canonical_events are run via a separate `psql -d
-- spendguard_canonical -c "DO $$ … $$"` block in the Makefile target
-- `demo-verify-litellm-real`. Keep ONLY ledger-DB assertions here.
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

    -- Slice 6 ships steps 1+2 (1 ALLOW + 1 DENY). Slice 9 adds
    -- step 3 (STREAM ALLOW) + step 4 (2 multi-team ALLOWs), bringing
    -- the ALLOW total to 4. Assertion: reserve >= 1 / commit >= 1 is
    -- the v1 Slice-6 baseline; >= 4 is the post-Slice-9 baseline. We
    -- keep the relaxed >= 1 here so the Slice 6-only operator gate
    -- still passes; Slice 9 introduces a stricter assertion via a
    -- separate verify file if needed.
    IF v_reserve < 1 THEN
        RAISE EXCEPTION 'SLICE6_GATE: ledger_transactions.reserve >= 1 expected, got %', v_reserve;
    END IF;
    IF v_commit < 1 THEN
        RAISE EXCEPTION 'SLICE6_GATE: ledger_transactions.commit_estimated >= 1 expected, got %', v_commit;
    END IF;

    RAISE NOTICE 'SLICE6/9 LEDGER OK: reserve=% commit_estimated=%',
        v_reserve, v_commit;
END;
$$;

\echo
\echo === ASSERT: DENY step produced denied_decision row ===
DO $$
DECLARE
    v_denied INT;
BEGIN
    SELECT COUNT(*) INTO v_denied
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'denied_decision';
    IF v_denied < 1 THEN
        RAISE EXCEPTION 'SLICE6_GATE: ledger_transactions.denied_decision >= 1 expected, got % (DENY step did not fire SPENDGUARD_DENY)', v_denied;
    END IF;
    RAISE NOTICE 'SLICE6 DENY OK: denied_decision=%', v_denied;
END;
$$;
