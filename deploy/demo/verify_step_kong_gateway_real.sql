-- D09 SLICE 7 (kong_gateway_real demo) — ledger-DB assertions.
--
-- Mirrors verify_step_envoy_extproc.sql / verify_step_litellm_guardrail.sql:
-- ledger-side gates only. The Makefile target
-- `demo-verify-kong-gateway-real` runs this file against
-- spendguard_ledger.
--
-- Review-standards.md §8 gates (kong port of envoy_extproc gates):
--   - reserve >= 2 (ALLOW + STREAM each produce a reservation)
--   - commit_estimated >= 2 (both ALLOW paths commit)
--   - denied_decision >= 1 (DENY step short-circuits at the sidecar)
--
-- INV-2 strict-order proof: the demo driver asserts the counting-stub
-- HTTP hit happens AFTER the sidecar reserve on each ALLOW step (via
-- pre/post counter comparison). Here in the ledger we assert that the
-- reserve row exists and predates the audit_outbox row that records
-- the upstream LLM_CALL_POST outcome.

\echo
\echo === ledger_transactions: operation_kind counts (kong_gateway_real) ===
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
\echo === commits: latest_state for the 2 ALLOW steps (ALLOW + STREAM) ===
SELECT latest_state, estimated_amount_atomic
  FROM commits
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
 ORDER BY created_at DESC
 LIMIT 5;

\echo
\echo === ASSERT: 2 ALLOW (ALLOW + STREAM) + 1 DENY produced ledger rows ===
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

    -- ALLOW + STREAM each produce 1 reserve. The DENY step never
    -- creates a reservation. Counts use `>=` so the gate is robust
    -- against demo-mode retries or prior-state bleed-through.
    IF v_reserve < 2 THEN
        RAISE EXCEPTION 'D09_GATE: ledger_transactions.reserve >= 2 expected (ALLOW + STREAM), got %', v_reserve;
    END IF;
    IF v_commit < 2 THEN
        RAISE EXCEPTION 'D09_GATE: ledger_transactions.commit_estimated >= 2 expected (ALLOW + STREAM), got %', v_commit;
    END IF;
    IF v_denied < 1 THEN
        RAISE EXCEPTION 'D09_GATE: ledger_transactions.denied_decision >= 1 expected (DENY step), got %', v_denied;
    END IF;

    RAISE NOTICE 'D09 LEDGER OK: reserve=% commit_estimated=% denied_decision=%',
        v_reserve, v_commit, v_denied;
END;
$$;

\echo
\echo === ASSERT: INV-2 strict-order — earliest reserve precedes earliest outcome ===
DO $$
DECLARE
    v_first_reserve TIMESTAMPTZ;
    v_first_outcome TIMESTAMPTZ;
BEGIN
    SELECT MIN(created_at) INTO v_first_reserve
      FROM reservations
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND current_state IN ('reserved', 'committed', 'released');
    SELECT MIN(recorded_at) INTO v_first_outcome
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND event_type = 'spendguard.audit.outcome'
       AND recorded_at > now() - interval '5 minute';

    IF v_first_reserve IS NULL THEN
        RAISE EXCEPTION 'D09_GATE: no reservation rows found for INV-2 check';
    END IF;
    IF v_first_outcome IS NULL THEN
        RAISE EXCEPTION 'D09_GATE: no outcome rows found for INV-2 check';
    END IF;
    IF v_first_reserve >= v_first_outcome THEN
        RAISE EXCEPTION 'D09_GATE: INV-2 violated — first reserve=% NOT before first outcome=%',
            v_first_reserve, v_first_outcome;
    END IF;
    RAISE NOTICE 'D09 INV-2 OK: first_reserve=% < first_outcome=%',
        v_first_reserve, v_first_outcome;
END;
$$;
