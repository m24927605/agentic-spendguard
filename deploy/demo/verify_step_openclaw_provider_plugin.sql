-- COV_D40B_05 (openclaw_provider_plugin demo) - ledger-DB assertions.
--
-- Expected live steps:
--   ALLOW          reserve + commit_estimated
--   DENY           denied_decision, no upstream provider hit
--   STREAM         reserve + commit_estimated
--   PROVIDER_ERROR reserve + release lane (LLM_CALL_POST outcome != SUCCESS)

\set ON_ERROR_STOP on

\echo
\echo === COV_D40B_GATE: ledger_transactions operation_kind counts ===
SELECT operation_kind, COUNT(*)::int AS n
  FROM ledger_transactions
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
   AND operation_kind IN ('reserve', 'commit_estimated', 'release', 'denied_decision')
 GROUP BY operation_kind
 ORDER BY operation_kind;

\echo
\echo === COV_D40B_GATE: reservations.current_state ===
SELECT current_state, COUNT(*)::int AS n
  FROM reservations
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
   AND budget_id = '44444444-4444-4444-8444-444444444444'
 GROUP BY current_state
 ORDER BY current_state;

\echo
\echo === COV_D40B_GATE: commits latest rows ===
SELECT latest_state, estimated_amount_atomic
  FROM commits
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
 ORDER BY created_at DESC
 LIMIT 5;

\echo
\echo === COV_D40B_GATE: ASSERT ALLOW + DENY + STREAM + PROVIDER_ERROR rows ===
DO $$
DECLARE
    v_reserve INT;
    v_commit INT;
    v_release INT;
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
    SELECT COUNT(*) INTO v_release
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'release';
    SELECT COUNT(*) INTO v_denied
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'denied_decision';

    IF v_reserve < 3 THEN
        RAISE EXCEPTION 'COV_D40B_GATE: ledger_transactions.reserve >= 3 expected (ALLOW + STREAM + PROVIDER_ERROR), got %', v_reserve;
    END IF;
    IF v_commit < 2 THEN
        RAISE EXCEPTION 'COV_D40B_GATE: ledger_transactions.commit_estimated >= 2 expected (ALLOW + STREAM), got %', v_commit;
    END IF;
    IF v_release < 1 THEN
        RAISE EXCEPTION 'COV_D40B_GATE: ledger_transactions.release >= 1 expected (PROVIDER_ERROR release lane), got %', v_release;
    END IF;
    IF v_denied < 1 THEN
        RAISE EXCEPTION 'COV_D40B_GATE: ledger_transactions.denied_decision >= 1 expected (DENY), got %', v_denied;
    END IF;

    RAISE NOTICE 'COV_D40B_GATE LEDGER OK: reserve=% commit_estimated=% release=% denied_decision=%',
        v_reserve, v_commit, v_release, v_denied;
END;
$$;

\echo
\echo === COV_D40B_GATE: ASSERT earliest reserve precedes earliest outcome ===
DO $$
DECLARE
    v_first_reserve TIMESTAMPTZ;
    v_first_outcome TIMESTAMPTZ;
BEGIN
    SELECT MIN(created_at) INTO v_first_reserve
      FROM reservations
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND budget_id = '44444444-4444-4444-8444-444444444444'
       AND created_at > now() - interval '10 minutes';
    SELECT MIN(recorded_at) INTO v_first_outcome
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND event_type = 'spendguard.audit.outcome'
       AND recorded_at > now() - interval '10 minutes';

    IF v_first_reserve IS NULL THEN
        RAISE EXCEPTION 'COV_D40B_GATE: no recent reservation rows found';
    END IF;
    IF v_first_outcome IS NULL THEN
        RAISE EXCEPTION 'COV_D40B_GATE: no recent audit_outbox outcome rows found';
    END IF;
    IF v_first_reserve >= v_first_outcome THEN
        RAISE EXCEPTION 'COV_D40B_GATE: ordering violated, first reserve=% not before first outcome=%',
            v_first_reserve, v_first_outcome;
    END IF;

    RAISE NOTICE 'COV_D40B_GATE ORDER OK: first_reserve=% < first_outcome=%',
        v_first_reserve, v_first_outcome;
END;
$$;

\echo
\echo === COV_D40B_GATE: ASSERT audit decisions and outcomes ===
DO $$
DECLARE
    v_decision INT;
    v_outcome INT;
BEGIN
    SELECT COUNT(*) INTO v_decision
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND event_type = 'spendguard.audit.decision'
       AND recorded_at > now() - interval '10 minutes';
    SELECT COUNT(*) INTO v_outcome
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND event_type = 'spendguard.audit.outcome'
       AND recorded_at > now() - interval '10 minutes';

    IF v_decision < 4 THEN
        RAISE EXCEPTION 'COV_D40B_GATE: audit_outbox decision rows >= 4 expected (ALLOW + DENY + STREAM + PROVIDER_ERROR), got %', v_decision;
    END IF;
    IF v_outcome < 3 THEN
        RAISE EXCEPTION 'COV_D40B_GATE: audit_outbox outcome rows >= 3 expected (ALLOW + STREAM + PROVIDER_ERROR), got %', v_outcome;
    END IF;

    RAISE NOTICE 'COV_D40B_GATE AUDIT OK: decisions=% outcomes=%',
        v_decision, v_outcome;
END;
$$;
