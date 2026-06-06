# D20 — Tests

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`acceptance.md`](acceptance.md).

## 1. Test inventory

| Tier | Suite | Path | Count |
|------|-------|------|-------|
| Unit (mock Strands runtime) | `test_strands.py` | `sdk/python/tests/integrations/test_strands.py` | 18 |
| Integration (real strands + pytest-httpx + 3 backends) | `test_strands_real.py` | `sdk/python/tests/integrations/test_strands_real.py` | 9 |
| Default-estimator | `test_default_estimators.py::TestStrands` | `sdk/python/tests/integrations/test_default_estimators.py` | 4 |
| Demo regression | `verify_step_strands.sql` | `deploy/demo/verify_step_strands.sql` | 6 SQL assertions |
| Demo driver | `run_demo.py::run_strands_real_mode` + `run_strands_deny_mode` | `deploy/demo/demo/run_demo.py` | 3+3 steps |

Total new test surface: ~700 LOC tests + 150 LOC fixtures + 80 LOC SQL gates + 150 LOC demo driver.

## 2. Unit tests — `test_strands.py`

Mocks Strands' `HookRegistry` + `BeforeInvocationEvent` + `AfterInvocationEvent` + `Invocation` via small dataclass fakes (no real `strands` import required for unit suite — slice 1 dataclasses are protocol-only). Sidecar mocked via `_fake_sidecar.py` shared with D11/D12.

### 2.1 Construction + lifecycle

| # | Name | What it asserts |
|---|------|-----------------|
| U01 | `test_import_error_message_when_strands_missing` | Module-level `strands.hooks` import patched to raise → `ImportError` with `pip install 'spendguard-sdk[strands]'` substring. |
| U02 | `test_construct_with_minimal_args` | `SpendGuardHookProvider(client, budget_id, ..., claim_reconciler=...)` succeeds; `claim_estimator` defaults to `strands_default_claim_estimator`. |
| U03 | `test_construct_rejects_empty_budget_id` | Empty `budget_id` → `SpendGuardConfigError`. |
| U04 | `test_construct_rejects_empty_window_instance_id` | Empty `window_instance_id` → `SpendGuardConfigError`. |
| U05 | `test_construct_rejects_unit_with_no_unit_id` | `unit.unit_id == ""` → `SpendGuardConfigError`. |
| U06 | `test_register_hooks_binds_both_callbacks` | `register_hooks(registry)` calls `registry.add_callback` for both `BeforeInvocationEvent` and `AfterInvocationEvent`. |

### 2.2 `before_invocation` reserve (LOAD-BEARING)

| # | Name | What it asserts |
|---|------|-----------------|
| U07 | `test_before_invocation_reserves` | Invokes hook with fake `BeforeInvocationEvent`. Asserts fake sidecar recorded exactly 1 `RequestDecision` with `trigger=LLM_CALL_PRE`. Stash has 1 entry keyed by `invocation_id`. |
| U08 | `test_before_invocation_missing_invocation_id_raises` | Fake `Invocation` with `invocation_id=None` → `SpendGuardConfigError` mentioning Strands version pin. |
| U09 | `test_before_invocation_deny_raises_decision_denied` | Sidecar returns DENY → `DecisionDenied` raised → stash NOT populated. |
| U10 | `test_before_invocation_degrade_fails_closed` | Sidecar returns DEGRADE → `SidecarUnavailable` raised → stash NOT populated. |
| U11 | `test_before_invocation_fail_open_allows_on_degrade` | With `SPENDGUARD_STRANDS_FAIL_OPEN=1`, DEGRADE returns silently; stash NOT populated (no fake reserve to commit later). |
| U12 | `test_before_invocation_estimator_returns_zero_claims_raises` | Custom estimator returns `[]` → `SpendGuardConfigError`. |
| U13 | `test_before_invocation_estimator_returns_two_claims_raises` | Custom estimator returns 2 claims → `SpendGuardConfigError`. |
| U14 | `test_before_invocation_validates_estimator_claim_matches_binding` | Estimator returns claim with mismatched `budget_id` → `SpendGuardConfigError` with `claim_estimator` source label. |

### 2.3 `after_invocation` commit/release

| # | Name | What it asserts |
|---|------|-----------------|
| U15 | `test_after_invocation_commits_on_success` | Fake `AfterInvocationEvent(exception=None, result=...)` → fake sidecar recorded `emit_llm_call_post(outcome=SUCCESS)` with `provider_event_id` from result. Stash popped. |
| U16 | `test_after_invocation_releases_on_exception` | `AfterInvocationEvent(exception=httpx.HTTPError(...))` → `emit_llm_call_post(outcome=FAILURE)`. Original exception NOT masked. Stash popped. |
| U17 | `test_after_invocation_cancelled_classification` | `exception=asyncio.CancelledError()` → `outcome=CANCELLED`. |
| U18 | `test_after_invocation_no_pending_is_noop` | `after_invocation` called without prior `before_invocation` → silent no-op (no fake sidecar call). |
| U19 | `test_after_invocation_reconciler_exception_falls_back_to_estimator` | Reconciler raises → estimator snapshot used + WARN log + commit succeeds. |
| U20 | `test_concurrent_invocations_no_stash_collision` | 5 concurrent `asyncio.gather` of `before_invocation` with distinct `invocation_id`; all 5 stash entries present; `after_invocation` on each clears it; no orphan. |

## 3. Integration tests — `test_strands_real.py`

Imports **real** `strands>=1.0`. Mocks the upstream provider HTTP endpoint via `pytest-httpx`. Sidecar still mocked via `_fake_sidecar.py`. Each test asserts wire-level ordering: sidecar RPC happens before httpx records the provider call.

The load-bearing multi-backend matrix:

| # | Name | Backend | What it asserts |
|---|------|---------|-----------------|
| I01 | `test_hook_provider_fires_for_bedrock` | `BedrockModel(model_id="anthropic.claude-3-5-sonnet-20241022-v2:0")` | Reserve RPC < Bedrock InvokeModel HTTP (asyncio.Event ordering check). Commit fires with `outcome=SUCCESS`. `decision_context.model_backend == "BedrockModel"`. |
| I02 | `test_hook_provider_fires_for_openai` | `OpenAIModel(model="gpt-4o-mini", api_key="sk-test")` | Reserve RPC < OpenAI HTTP. Commit fires. `decision_context.model_backend == "OpenAIModel"`. |
| I03 | `test_hook_provider_fires_for_litellm` | `LiteLLMModel(model="gemini/gemini-1.5-pro", api_key="sk-test")` | Reserve RPC < Gemini HTTP. Commit fires. `decision_context.model_backend == "LiteLLMModel"`. |
| I04 | `test_bedrock_deny_zero_provider_hits` | Bedrock | Sidecar DENY → `DecisionDenied` propagates → `httpx_mock.get_requests()` recorded ZERO Bedrock calls. INV-1 for Bedrock. |
| I05 | `test_openai_deny_zero_provider_hits` | OpenAI | Same as I04 for OpenAI. |
| I06 | `test_litellm_deny_zero_provider_hits` | LiteLLM | Same as I04 for LiteLLM (no api.openai.com / no api.bedrock / no api.gemini). |
| I07 | `test_real_strands_concurrent_invocations` | Bedrock (chosen for AWS-default path) | `asyncio.gather` of 5 `agent.invoke_async()` with same Agent; all 5 reserves + 5 commits recorded. |
| I08 | `test_real_strands_provider_exception_releases` | OpenAI | `pytest-httpx` returns 500 → Strands raises → `after_invocation` emits `FAILURE`. |
| I09 | `test_real_strands_model_swap_mid_run` | Start Bedrock → swap to OpenAI mid-run | Each invocation gets its own stash entry; no cross-contamination. |

The strict-order check uses an `asyncio.Event` set by the fake sidecar's `RequestDecision` mock. The `pytest-httpx` request callback checks `event.is_set()`; if False, the test fails with `out-of-order` evidence.

## 4. Default-estimator tests — `test_default_estimators.py::TestStrands`

| # | Name | What it asserts |
|---|------|-----------------|
| E01 | `test_strands_default_estimator_bedrock_anthropic` | Invocation with `model.model_id="anthropic.claude-3-5-sonnet-..."` → estimator produces a claim with `amount_atomic > 0` using the Anthropic token table. |
| E02 | `test_strands_default_estimator_openai` | Invocation with `model.model="gpt-4o-mini"` → estimator dispatches to OpenAI tiktoken family. |
| E03 | `test_strands_default_estimator_unknown_falls_back_to_chars_div_4` | Unknown model name → fallback estimator + WARN log. |
| E04 | `test_strands_default_estimator_handles_missing_messages` | `Invocation.messages` is empty → estimator returns minimum-floor claim (50 tokens). |

## 5. Demo regression — `verify_step_strands.sql`

Gates executed after `DEMO_MODE=agent_real_strands` and `DEMO_MODE=agent_real_strands_deny`. Layout mirrors `verify_step_litellm_sdk.sql`.

```sql
-- D20_STRANDS: at least 1 reserve for integration='strands'.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM ledger_transactions
   WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
     AND operation_kind = 'reserve';
  IF c < 1 THEN
    RAISE EXCEPTION 'D20_STRANDS_GATE: reserve >= 1 expected, got %', c;
  END IF;
END; $$;

-- D20_STRANDS: at least 1 commit_estimated row.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM ledger_transactions
   WHERE operation_kind = 'commit_estimated';
  IF c < 1 THEN RAISE EXCEPTION 'D20_STRANDS_GATE: commit >= 1 expected'; END IF;
END; $$;

-- D20_STRANDS: at least 1 denied_decision (deny mode only).
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM ledger_transactions
   WHERE operation_kind = 'denied_decision';
  IF c < 1 THEN
    RAISE EXCEPTION 'D20_STRANDS_GATE: denied_decision >= 1 expected for deny mode';
  END IF;
END; $$;

-- D20_STRANDS: decision_context.integration = 'strands' present.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM audit_outbox
   WHERE decision_context->>'integration' = 'strands';
  IF c < 1 THEN
    RAISE EXCEPTION 'D20_STRANDS_GATE: at least 1 audit with integration=strands';
  END IF;
END; $$;

-- D20_STRANDS: model_backend coverage matrix — real mode hit both Bedrock and OpenAI.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(DISTINCT decision_context->>'model_backend') INTO c FROM audit_outbox
   WHERE decision_context->>'integration' = 'strands';
  IF c < 2 THEN
    RAISE EXCEPTION
      'D20_STRANDS_GATE: model_backend variety < 2 (expected Bedrock + OpenAI in real mode)';
  END IF;
END; $$;

-- D20_STRANDS: canonical chain received the events.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM canonical_events
   WHERE source_integration = 'strands';
  IF c < 1 THEN RAISE EXCEPTION 'D20_STRANDS_GATE: canonical_events empty'; END IF;
END; $$;
```

The model_backend variety check is the load-bearing coverage matrix proof: it fails if `agent_real_strands` only exercised one backend.

## 6. Demo driver — `run_strands_real_mode` + `run_strands_deny_mode`

`run_strands_real_mode` (3 steps):

| Step | Body | Asserts |
|------|------|---------|
| 1 BEDROCK ALLOW | `Agent(model=BedrockModel(...)).invoke_async("Hello")` with in-proc Bedrock-mock | reserve fires before HTTP, stub counter +1, commit row visible |
| 2 OPENAI ALLOW | swap to `OpenAIModel(...)` on same Agent, invoke again | reserve fires before HTTP, stub counter +1, commit row with `model_backend=OpenAIModel` |
| 3 LITELLM ALLOW | swap to `LiteLLMModel(model="gemini/...")`, invoke | reserve+commit, `model_backend=LiteLLMModel` |

`run_strands_deny_mode` (3 sub-steps, mirrors `run_litellm_deny_mode`):

| Step | Body | Asserts |
|------|------|---------|
| 1 BEDROCK ALLOW positive control | Bedrock-mock | reserve + provider hit (proves wiring) |
| 2 BEDROCK DENY budget exhausted | `spendguard_estimate_override=2000000000` on the estimator | `DecisionDenied`; stub counter unchanged |
| 3 BEDROCK DENY sidecar unreachable | inject `SidecarUnavailable` | `SidecarUnavailable`; stub counter unchanged |

Each driver writes a single-line summary on success: `[demo] agent_real_strands ALL 3 steps PASS` / `[demo] agent_real_strands_deny ALL 3 substeps PASS`. Gate-failure exits code 7.

## 7. Negative test surface

| What | Why | Where |
|------|-----|-------|
| Provider hit on DENY (any of 3 backends) | INV-1 cross-backend | I04 + I05 + I06 + demo deny step 2 + verify SQL stub-counter delta |
| Reserve fires AFTER provider | Cancels D20 thesis | U07 + I01-I03 strict-order check |
| Stash collision under concurrency | Would double-charge or lose audit | U20 + I07 |
| Missing `invocation_id` (SDK contract break) | Silent gating gap | U08 with explicit SDK version-pin guidance |
| Reconciler crash | Would lose audit row | U19 estimator-fallback |
| `_stash` not cleared on exception | Memory leak under DOS of failing agents | U16 + U17 verify pop happens in `finally` path |
| Cross-backend coverage regression | The matrix is the load-bearing claim | verify SQL `model_backend` variety check |
| Recursion when LiteLLMModel internally calls litellm.acompletion | D12 shim would double-reserve | I06 verifies single reserve (the hook layer wins; D12 shim contextvar `_IN_FLIGHT` short-circuits inner) |

## 8. Performance budgets (informational, not gates)

| Op | Target | Source |
|----|--------|--------|
| `before_invocation` overhead (excluding sidecar gRPC) | < 2ms p99 | dataclass construction + estimator dispatch + stash write |
| `after_invocation` overhead (excluding sidecar gRPC) | < 1ms p99 | stash pop + reconciler dispatch |
| `_stash` memory under 10k in-flight invocations | < 5MB | `_PendingInvocation` is ~400 bytes; 10k × 400 = 4MB |

Verified manually post-merge.

## 9. CI integration

- `sdk/python/tests/integrations/test_strands.py` + `test_strands_real.py` run under existing `pytest sdk/python` GitHub Actions matrix.
- New `[strands]` extra installs `aws-strands-agents>=1.0,<2` + `pytest-httpx>=0.30` for the test matrix.
- Strands' Bedrock/OpenAI/LiteLLM backends each have transitive deps (boto3, openai, litellm); CI installs all three so the backend matrix runs.
- `make demo-up DEMO_MODE=agent_real_strands` and `DEMO_MODE=agent_real_strands_deny` run as new matrix cells in `e2e-demo`.

## 10. Test isolation rules (mandatory)

- Each test using `SpendGuardHookProvider` constructs a fresh fake sidecar (no shared mutable state across tests).
- Concurrent tests in `pytest-xdist` are safe because the hook provider holds no module-level state — `_stash` is instance-scoped.
- The 3 recorded provider fixtures (`fixtures/strands/*.json`) are immutable; tests use them read-only.
- No test imports `strands` real Bedrock client without `pytest-httpx` intercepting — outbound network attempts to `bedrock-runtime.*.amazonaws.com` fail the test (caught by `httpx_mock.assert_all_responses_were_requested`).
