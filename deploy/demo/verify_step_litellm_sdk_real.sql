-- COV_D12 SLICE 7 — `DEMO_MODE=litellm_sdk_real` verification.
-- Ledger-DB assertions only (canonical_events lives in
-- spendguard_canonical and is asserted by a second psql block in the
-- Makefile target `demo-verify-litellm-sdk-real`, mirroring the
-- litellm_real / dify_plugin_real pattern).
--
-- D12 closes the LiteLLM Issue #8842 gap: direct ``litellm.acompletion()``
-- callers (and every transitive caller — CrewAI, DSPy, SmolAgents, etc.)
-- now go through the SpendGuard shim before the upstream HTTP fires.
-- The litellm_sdk_real driver runs 3 steps (ALLOW + STREAM + optional
-- transitive CrewAI); each Step that hit a sidecar reserve produces
-- ledger rows.

\echo
\echo === ledger_transactions: operation_kind counts (litellm_sdk_real) ===
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
\echo === commits: latest_state for the 2 deterministic ALLOW steps ===
SELECT latest_state, estimated_amount_atomic
  FROM commits
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
 ORDER BY created_at DESC
 LIMIT 5;

\echo
\echo === ASSERT: ALLOW + STREAM produce >=2 reserve + >=2 commit rows ===
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

    -- ALLOW + STREAM each produce 1 reserve. CrewAI transitive Step
    -- may add 0 or N depending on whether the framework bailed on the
    -- stub answer parse. >=2 is the floor; >2 is fine.
    IF v_reserve < 2 THEN
        RAISE EXCEPTION 'D12_SDK_GATE: ledger_transactions.reserve >= 2 expected (ALLOW + STREAM), got %', v_reserve;
    END IF;
    IF v_commit < 2 THEN
        RAISE EXCEPTION 'D12_SDK_GATE: ledger_transactions.commit_estimated >= 2 expected (ALLOW + STREAM), got %', v_commit;
    END IF;

    RAISE NOTICE 'D12_SDK LEDGER OK: reserve=% commit_estimated=%',
        v_reserve, v_commit;
END;
$$;

-- D12_SDK_GATE: the shim's _DirectCore sets
-- decision_context['mode'] = 'sdk' (services/sidecar's audit chain
-- threads this through to the ledger). This is how the litellm_sdk
-- audit rows are distinguished from litellm (callback) / litellm-
-- guardrail / litellm-direct rows in the same canonical chain.
\echo
\echo === ASSERT: at least 1 audit_outbox row carries spendguard.mode='sdk' ===
-- The decision_context_json the SDK passes lands under
-- audit_outbox.cloudevent_payload->'data_b64' (base64 encoded). The
-- shim's _DirectCore writes decision_context = {
--    "integration":"litellm", "mode":"sdk", "model":..., ...
-- }; the sidecar's audit chain threads that through as
-- data_b64 -> data.spendguard.{integration, mode, model, ...}.
DO $$
DECLARE
    v_sdk_decisions INT;
BEGIN
    SELECT COUNT(*) INTO v_sdk_decisions
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND event_type = 'spendguard.audit.decision'
       AND convert_from(decode(cloudevent_payload->>'data_b64', 'base64'),
                        'UTF8') LIKE '%"mode":"sdk"%'
       AND recorded_at > now() - interval '5 minute';
    IF v_sdk_decisions < 1 THEN
        RAISE EXCEPTION 'D12_SDK_GATE: audit_outbox decision rows with spendguard.mode=sdk >= 1 expected, got %', v_sdk_decisions;
    END IF;
    RAISE NOTICE 'D12_SDK AUDIT OK: mode=sdk decisions=%', v_sdk_decisions;
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
       AND event_type = 'spendguard.audit.outcome';
    IF v_first_reserve IS NULL THEN
        RAISE EXCEPTION 'D12_SDK_INV2_GATE: no reservations recorded';
    END IF;
    IF v_first_outcome IS NULL THEN
        RAISE EXCEPTION 'D12_SDK_INV2_GATE: no outcome events recorded';
    END IF;
    IF v_first_outcome < v_first_reserve THEN
        RAISE EXCEPTION 'D12_SDK_INV2_GATE: outcome before reserve (% < %); INV-2 violated',
            v_first_outcome, v_first_reserve;
    END IF;
    RAISE NOTICE 'D12_SDK INV-2 OK: first_reserve=% first_outcome=%',
        v_first_reserve, v_first_outcome;
END;
$$;
