-- COV_D29 SLICE 5 (inngest_agent_kit demo) — ledger-DB assertions.
--
-- Mirrors verify_step_openai_agents_ts.sql: ledger-side gates only.
-- The Makefile target `demo-verify-inngest-agent-kit` runs this file
-- against spendguard_ledger, then issues a second `psql -d
-- spendguard_canonical` block for the cross-DB canonical_events
-- check (decision/outcome counts).
--
-- Review-standards §4 + §11 gates:
--   - reserve == 2 (ALLOW + RETRY_DEDUP each produce EXACTLY one
--                   reservation; RETRY_DEDUP produces ONE despite 3
--                   attempts thanks to the in-process idempotency cache.
--                   THIS IS THE D29 HEADLINE RETRY-DEDUP GATE.)
--   - commit_estimated >= 2 (ALLOW produces 1 SUCCESS commit;
--                            RETRY_DEDUP produces 3 commits — 2 PROVIDER_ERROR
--                            + 1 SUCCESS; total >= 2.)
--   - denied_decision >= 1 (DENY step short-circuits at the sidecar
--                            before the upstream HTTP call leaves the
--                            inngest-agent-kit-runner process)
--
-- The retry-dedup gate is the most important assertion here: D29's
-- contract is that N attempts of the same step body produce ONE
-- logical decision in the ledger. If RETRY_DEDUP leaks more than one
-- reservation, the headline contract is broken.

\echo
\echo === ledger_transactions: operation_kind counts (inngest_agent_kit) ===
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
\echo === commits: latest_state for ALLOW + RETRY_DEDUP commits ===
SELECT latest_state, estimated_amount_atomic
  FROM commits
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
 ORDER BY created_at DESC
 LIMIT 6;

\echo
\echo === ASSERT: D29 headline retry-dedup — reserve count is EXACTLY 2 ===
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

    -- D29 HEADLINE: ALLOW (1 reserve) + RETRY_DEDUP (1 reserve across
    -- 3 attempts) = 2 reserves total. The DENY step never creates a
    -- reservation (contract evaluator emits SPENDGUARD_DENY pre-call;
    -- sidecar writes a denied_decision row instead). If the retry-dedup
    -- contract is broken — the in-process idempotency cache failed,
    -- the substrate-side dedup also failed, or the seed derivation
    -- includes `attempt` — the reserve count climbs to 4 (1 + 3) and
    -- this gate trips.
    IF v_reserve < 2 THEN
        RAISE EXCEPTION 'COV_D29_GATE: ledger_transactions.reserve >= 2 expected (ALLOW + RETRY_DEDUP), got %', v_reserve;
    END IF;
    IF v_reserve > 2 THEN
        RAISE EXCEPTION 'COV_D29_DEDUP_GATE: ledger_transactions.reserve == 2 expected (RETRY_DEDUP must dedup 3 attempts → 1 reservation), got %; the in-process idempotency cache or sidecar-side dedup is broken — N attempts leaked N reservations.', v_reserve;
    END IF;
    IF v_commit < 2 THEN
        RAISE EXCEPTION 'COV_D29_GATE: ledger_transactions.commit_estimated >= 2 expected (ALLOW + RETRY_DEDUP attempts), got %', v_commit;
    END IF;
    IF v_denied < 1 THEN
        RAISE EXCEPTION 'COV_D29_GATE: ledger_transactions.denied_decision >= 1 expected (DENY step), got %', v_denied;
    END IF;

    RAISE NOTICE 'COV_D29 LEDGER OK: reserve=% commit_estimated=% denied_decision=% (HEADLINE RETRY-DEDUP CONTRACT HELD)',
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
        RAISE EXCEPTION 'COV_D29_GATE: no reservation rows found for INV-2 check';
    END IF;
    IF v_first_outcome IS NULL THEN
        RAISE EXCEPTION 'COV_D29_GATE: no outcome rows found for INV-2 check';
    END IF;
    IF v_first_reserve >= v_first_outcome THEN
        RAISE EXCEPTION 'COV_D29_GATE: INV-2 violated — first reserve=% NOT before first outcome=%',
            v_first_reserve, v_first_outcome;
    END IF;
    RAISE NOTICE 'COV_D29 INV-2 OK: first_reserve=% < first_outcome=%',
        v_first_reserve, v_first_outcome;
END;
$$;

\echo
\echo === ASSERT: audit_outbox carries inngest-agent-kit decision rows ===
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
        RAISE EXCEPTION 'COV_D29_GATE: audit_outbox decision rows >= 2 expected (ALLOW + DENY both emit decisions; RETRY_DEDUP collapses 3 attempts into 1), got %', v_decision;
    END IF;
    RAISE NOTICE 'COV_D29 AUDIT OK: decision rows=%', v_decision;
END;
$$;
