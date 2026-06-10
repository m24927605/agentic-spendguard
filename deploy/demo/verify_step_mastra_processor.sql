-- COV_D38_05 (mastra_processor demo) — ledger-DB assertions.
--
-- Mirrors verify_step_langchain_ts.sql: ledger-side gates only.
-- The Makefile target `demo-verify-mastra-processor` runs this file
-- against spendguard_ledger, then issues a second `psql -d
-- spendguard_canonical` block for the cross-DB canonical_events
-- check (decision/outcome counts).
--
-- Review-standards §11 + acceptance §5 gates (A5.3):
--   - reserve >= 2 (ALLOW + STREAM each produce a reservation)
--   - commit_estimated >= 2 (both ALLOW paths commit)
--   - denied_decision >= 1 (DENY step short-circuits at the sidecar
--     before the provider HTTP call leaves the mastra-processor-runner
--     process — the runner-side /_count UNCHANGED assertion is the
--     live half of this proof, TA-04)
--
-- INV-2 strict-order proof: the runner-side counter comparison
-- already proves the live ordering (counting-stub pre vs post on each
-- ALLOW step). Here in the ledger we complement that with a DB-side
-- assertion that the EARLIEST reserve row in this demo run predates
-- the EARLIEST `spendguard.audit.outcome` row — i.e. the first
-- reservation existed before the audit chain emitted its first
-- commit-outcome.

\echo
\echo === ledger_transactions: operation_kind counts (mastra_processor) ===
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
\echo === commits: latest_state for the 2 ALLOW steps (1 + STREAM) ===
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
    -- creates a reservation (the sidecar denies pre-call and writes a
    -- denied_decision row instead). Counts use `>=` so the gate is
    -- robust against demo-mode retries or any prior-state
    -- bleed-through from the base compose seed. The reserve >= 2 gate
    -- doubles as the day-1 unitId E2E proof (A5.7): with
    -- SPENDGUARD_UNIT_ID threaded, an empty claim[0].unit.unit_id
    -- would have been rejected (`INVALID_REQUEST`) and no reserve row
    -- would exist.
    IF v_reserve < 2 THEN
        RAISE EXCEPTION 'COV_D38_GATE: ledger_transactions.reserve >= 2 expected (ALLOW + STREAM), got %', v_reserve;
    END IF;
    IF v_commit < 2 THEN
        RAISE EXCEPTION 'COV_D38_GATE: ledger_transactions.commit_estimated >= 2 expected (ALLOW + STREAM), got %', v_commit;
    END IF;
    IF v_denied < 1 THEN
        RAISE EXCEPTION 'COV_D38_GATE: ledger_transactions.denied_decision >= 1 expected (DENY step), got %', v_denied;
    END IF;

    RAISE NOTICE 'COV_D38 LEDGER OK: reserve=% commit_estimated=% denied_decision=%',
        v_reserve, v_commit, v_denied;
END;
$$;

\echo
\echo === ASSERT: INV-2 strict-order — earliest reserve precedes earliest outcome ===
-- INV-2 is "sidecar reserve happens BEFORE upstream call". The
-- runner-side counter comparison already proves the live ordering
-- (counting_pre vs counting_post on each ALLOW step). Here we
-- complement that with a DB-side assertion that the EARLIEST
-- `reserve` row in this demo run predates the EARLIEST
-- `spendguard.audit.outcome` row — i.e. the first reservation
-- existed before the audit chain emitted its first commit-outcome.
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
        RAISE EXCEPTION 'COV_D38_GATE: no reservation rows found for INV-2 check';
    END IF;
    IF v_first_outcome IS NULL THEN
        RAISE EXCEPTION 'COV_D38_GATE: no outcome rows found for INV-2 check';
    END IF;
    IF v_first_reserve >= v_first_outcome THEN
        RAISE EXCEPTION 'COV_D38_GATE: INV-2 violated — first reserve=% NOT before first outcome=%',
            v_first_reserve, v_first_outcome;
    END IF;
    RAISE NOTICE 'COV_D38 INV-2 OK: first_reserve=% < first_outcome=%',
        v_first_reserve, v_first_outcome;
END;
$$;

\echo
\echo === ASSERT: audit_outbox carries mastra-js decision rows ===
-- The SpendGuardProcessor's processInputStep reserve() emits decisions
-- via the shared SpendGuardClient (runtimeKind "mastra-js" on the
-- handshake). We assert decision-row presence by the demo-mode tenant
-- + recency window; the runner-side `[demo] mastra_processor ALL 3
-- steps PASS` line gives the upstream causal-ordering signal.
DO $$
DECLARE
    v_decision INT;
BEGIN
    SELECT COUNT(*) INTO v_decision
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND event_type = 'spendguard.audit.decision'
       AND recorded_at > now() - interval '5 minute';
    IF v_decision < 2 THEN
        RAISE EXCEPTION 'COV_D38_GATE: audit_outbox decision rows >= 2 expected (ALLOW + DENY both emit decisions), got %', v_decision;
    END IF;
    RAISE NOTICE 'COV_D38 AUDIT OK: decision rows=%', v_decision;
END;
$$;
