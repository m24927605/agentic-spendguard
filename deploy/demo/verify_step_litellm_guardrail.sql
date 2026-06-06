-- COV_D11 SLICE 6 verification — `DEMO_MODE=litellm_guardrail`.
-- Ledger-DB assertions only (canonical_events lives in
-- spendguard_canonical and is asserted by a second `psql -d
-- spendguard_canonical` block in the Makefile target
-- `demo-verify-litellm-guardrail`, mirroring the litellm_real pattern).
--
-- Per tests.md §4, gate that the NEW guardrail-registry path
-- (SpendGuardGuardrail composing _LoopBoundCallback) produced the
-- same reserve→commit lifecycle as the legacy callback path, plus
-- a denied_decision row for the DENY step.

\echo
\echo === ledger_transactions: operation_kind counts (litellm_guardrail) ===
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

-- review-standards §6.4 — INV-2 strict-order: the demo driver asserts
-- the sidecar reserve fires BEFORE the counting provider is hit on
-- each ALLOW step. The in-process counting provider runs in the same
-- demo container as run_demo.py; the strict-order proof is captured
-- by the driver's pre/post counter comparison around each call
-- (counting_post == counting_pre + 1 for ALLOW; unchanged for DENY).
-- Here we assert the ledger reflects 2 ALLOWs + 1 DENY made it to
-- the audit chain.
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
    -- pre-call, sidecar writes a denied_decision row instead).
    IF v_reserve < 2 THEN
        RAISE EXCEPTION 'D11_GUARDRAIL_GATE: ledger_transactions.reserve >= 2 expected (ALLOW + STREAM), got %', v_reserve;
    END IF;
    IF v_commit < 2 THEN
        RAISE EXCEPTION 'D11_GUARDRAIL_GATE: ledger_transactions.commit_estimated >= 2 expected (ALLOW + STREAM), got %', v_commit;
    END IF;
    IF v_denied < 1 THEN
        RAISE EXCEPTION 'D11_GUARDRAIL_GATE: ledger_transactions.denied_decision >= 1 expected (DENY step), got %', v_denied;
    END IF;

    RAISE NOTICE 'D11_GUARDRAIL LEDGER OK: reserve=% commit_estimated=% denied_decision=%',
        v_reserve, v_commit, v_denied;
END;
$$;

\echo
\echo === ASSERT: audit_outbox carries litellm spendguard enrichment (GH #77) ===
-- The SDK sets spendguard_enrichment.integration='litellm' via the
-- `_build_decision_context` path in
-- sdk/python/src/spendguard/integrations/litellm.py (lines 120-145).
-- The sidecar's allowlist (services/sidecar/src/decision/transaction.rs
-- lines 88-100) threads the value through to the audit.decision
-- CloudEvent's `data.spendguard.integration` field. After the outbox
-- forwarder drains, canonical_events carries the same payload (which
-- the Makefile cross-DB block then asserts under spendguard_canonical).
--
-- Here in the ledger DB we sanity-check `cloudevent_payload` carries
-- the literal integration string somewhere in its bytes — the
-- canonical-DB assertion in Makefile does the structural check.
DO $$
DECLARE
    v_with_litellm INT;
BEGIN
    SELECT COUNT(*) INTO v_with_litellm
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND event_type = 'spendguard.audit.decision'
       AND cloudevent_payload::text LIKE '%litellm%'
       AND recorded_at > now() - interval '5 minute';
    IF v_with_litellm < 2 THEN
        RAISE EXCEPTION 'D11_GUARDRAIL_GATE: audit_outbox litellm decision rows >= 2 expected (ALLOW + DENY both emit decision rows), got %', v_with_litellm;
    END IF;
    RAISE NOTICE 'D11_GUARDRAIL AUDIT OK: litellm decisions=%', v_with_litellm;
END;
$$;
