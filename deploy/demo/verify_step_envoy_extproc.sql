-- COV_07 (D01 envoy_extproc demo) — ledger-DB assertions.
--
-- Mirrors verify_step_litellm_guardrail.sql: ledger-side gates only.
-- The Makefile target `demo-verify-envoy-extproc` runs this file
-- against spendguard_ledger, then issues a second `psql -d
-- spendguard_canonical` block for the cross-DB canonical_events
-- check (decision/outcome counts).
--
-- Review-standards §8.1 gates:
--   - reserve >= 2 (ALLOW + STREAM each produce a reservation)
--   - commit_estimated >= 2 (both ALLOW paths commit)
--   - denied_decision >= 1 (DENY step short-circuits at the sidecar)
--
-- INV-2 strict-order proof: the demo driver asserts the counting-stub
-- HTTP hit happens AFTER the sidecar reserve on each ALLOW step (via
-- pre/post counter comparison). Here in the ledger we assert that the
-- reserve row exists and predates the audit_outbox row that records
-- the upstream LLM_CALL_POST outcome. The counter comparison gives the
-- causal-ordering guarantee; this query gives the durable-row guarantee.

\echo
\echo === ledger_transactions: operation_kind counts (envoy_extproc) ===
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
    -- creates a reservation (contract evaluator emits SPENDGUARD_DENY
    -- pre-call, sidecar writes a denied_decision row instead). Counts
    -- use `>=` so the gate is robust against demo-mode retries or any
    -- prior-state bleed-through from the base compose seed.
    IF v_reserve < 2 THEN
        RAISE EXCEPTION 'COV_07_GATE: ledger_transactions.reserve >= 2 expected (ALLOW + STREAM), got %', v_reserve;
    END IF;
    IF v_commit < 2 THEN
        RAISE EXCEPTION 'COV_07_GATE: ledger_transactions.commit_estimated >= 2 expected (ALLOW + STREAM), got %', v_commit;
    END IF;
    IF v_denied < 1 THEN
        RAISE EXCEPTION 'COV_07_GATE: ledger_transactions.denied_decision >= 1 expected (DENY step), got %', v_denied;
    END IF;

    RAISE NOTICE 'COV_07 LEDGER OK: reserve=% commit_estimated=% denied_decision=%',
        v_reserve, v_commit, v_denied;
END;
$$;

\echo
\echo === ASSERT: INV-2 strict-order — earliest reserve precedes earliest outcome ===
-- INV-2 is "sidecar reserve happens BEFORE upstream call". The
-- driver-side counter comparison already proves the live ordering
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
        RAISE EXCEPTION 'COV_07_GATE: no reservation rows found for INV-2 check';
    END IF;
    IF v_first_outcome IS NULL THEN
        RAISE EXCEPTION 'COV_07_GATE: no outcome rows found for INV-2 check';
    END IF;
    IF v_first_reserve >= v_first_outcome THEN
        RAISE EXCEPTION 'COV_07_GATE: INV-2 violated — first reserve=% NOT before first outcome=%',
            v_first_reserve, v_first_outcome;
    END IF;
    RAISE NOTICE 'COV_07 INV-2 OK: first_reserve=% < first_outcome=%',
        v_first_reserve, v_first_outcome;
END;
$$;

\echo
\echo === ASSERT: audit_outbox carries envoy_extproc decision rows ===
-- The envoy_extproc binary emits ExtProc-shape decisions through the
-- shared sidecar adapter. The CloudEvent inner payload (the
-- base64-encoded `data_b64` field) carries the session_id derived in
-- services/envoy_extproc/src/decision.rs:122 — the literal prefix
-- `envoy-extproc:` proves the row originated from this demo path.
-- Acceptance gate 36 will also lift the explicit `runtime_kind=
-- envoy-ai-gateway` field once SLICE 7 follow-up wires it through
-- the sidecar adapter; the session_id substring is the SLICE 1-6
-- ground-truth signal.
--
-- We decode the data_b64 field with pgcrypto's decode() to do a
-- byte-level substring search; this avoids relying on the canonical
-- DB's cost_advisor_safe_decode_payload helper which is not present
-- in the ledger DB.
DO $$
DECLARE
    v_with_envoy INT;
BEGIN
    SELECT COUNT(*) INTO v_with_envoy
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND event_type = 'spendguard.audit.decision'
       AND recorded_at > now() - interval '5 minute'
       AND (
         -- Outer JSON match (covers any future runtime_kind field).
         cloudevent_payload::text LIKE '%envoy-extproc%'
         OR cloudevent_payload::text LIKE '%envoy-ai-gateway%'
         -- Inner base64-encoded data payload match. We decode then
         -- search for the session_id prefix the envoy_extproc binary
         -- always emits.
         OR convert_from(
              decode(cloudevent_payload->>'data_b64', 'base64'),
              'UTF8'
            ) LIKE '%envoy-extproc:%'
       );
    IF v_with_envoy < 2 THEN
        RAISE EXCEPTION 'COV_07_GATE: audit_outbox envoy decision rows >= 2 expected (ALLOW + DENY both emit decision rows), got %', v_with_envoy;
    END IF;
    RAISE NOTICE 'COV_07 AUDIT OK: envoy decision rows=%', v_with_envoy;
END;
$$;
