# D12 — Tests

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`acceptance.md`](acceptance.md).

## 1. Test inventory

| Tier | Suite | Path | Count |
|------|-------|------|-------|
| Unit (mock litellm) | `test_litellm_shim.py` | `sdk/python/tests/integrations/test_litellm_shim.py` | 22 |
| Integration (real litellm + pytest-httpx) | `test_litellm_shim_real.py` | `sdk/python/tests/integrations/test_litellm_shim_real.py` | 6 |
| Transitive coverage smoke | `test_crewai_via_shim.py` | `sdk/python/tests/integrations/test_crewai_via_shim.py` | 3 |
| Demo regression | `verify_step_litellm_sdk.sql` | `deploy/demo/verify_step_litellm_sdk.sql` | 6 SQL assertions |
| Demo driver | `run_demo.py::run_litellm_sdk_real_mode` + `run_litellm_sdk_deny_mode` | `deploy/demo/demo/run_demo.py` | 3+3 steps |

Total new test surface: ~900 LOC tests + ~80 LOC SQL gates + ~200 LOC demo driver.

## 2. Unit tests — `test_litellm_shim.py`

Mocks `litellm.acompletion` / `Router.acompletion` via `monkeypatch.setattr` so tests are deterministic and do not require network. The sidecar is mocked via `_fake_sidecar.py` (shared with D11). Each test wraps `install()` + `uninstall()` in a fixture so global state never leaks between tests.

### 2.1 Lifecycle + idempotency

| # | Name | What it asserts |
|---|------|-----------------|
| U01 | `test_import_error_message_when_litellm_missing` | Module-level `litellm` import patched to raise → `ImportError` with `pip install 'spendguard-sdk[litellm-shim]'` substring. |
| U02 | `test_is_installed_lifecycle` | `is_installed()` returns False initially, True after `install()`, False after `uninstall()`. |
| U03 | `test_install_idempotent_same_config` | Calling `install()` twice with identical args is a no-op (no second patching). |
| U04 | `test_install_different_config_raises` | Different `budget_resolver` → `SpendGuardShimAlreadyInstalled`. |
| U05 | `test_uninstall_restores_originals` | After `uninstall()`, `litellm.acompletion is _ORIGINAL_ACOMPLETION`, `litellm.completion is _ORIGINAL_COMPLETION`, `Router.acompletion is _ORIGINAL_ROUTER_ACOMPLETION`. |
| U06 | `test_uninstall_when_not_installed_is_noop` | `uninstall()` before `install()` does nothing, no exception. |

### 2.2 Patch surfaces

| # | Name | What it asserts |
|---|------|-----------------|
| U07 | `test_acompletion_patched_routes_through_core` | `litellm.acompletion(...)` after install dispatches to `state.core.__call__`. |
| U08 | `test_atext_completion_patched_routes_through_core` | Same for `atext_completion`. |
| U09 | `test_completion_sync_works_outside_loop` | `litellm.completion(...)` outside any loop bridges via `asyncio.run` and returns. |
| U10 | `test_completion_in_async_context_raises` | Inside `pytest.mark.asyncio` test, `litellm.completion(...)` raises `SpendGuardShimSyncInAsyncContext`. |
| U11 | `test_text_completion_sync_works` | Same as U09 for `text_completion`. |
| U12 | `test_router_acompletion_patched` | `Router(...).acompletion(...)` after install dispatches through core. |
| U13 | `test_router_subclass_at_install_time_patched` | If a subclass overrides `acompletion` BEFORE install, install patches the subclass too. |
| U14 | `test_router_subclass_created_after_install_inherits_patched` | Subclass created post-install gets the patched method via MRO. |
| U15 | `test_patch_router_false_skips_router` | `install(patch_router=False)` leaves `Router.acompletion` untouched. |
| U16 | `test_patch_sync_false_skips_completion` | `install(patch_sync=False)` leaves `litellm.completion` untouched. |

### 2.3 Reserve-before-provider ordering (LOAD-BEARING)

| # | Name | What it asserts |
|---|------|-----------------|
| U17 | `test_reserve_fires_before_provider_call` | Mock records call order: sidecar `RequestDecision` → original `litellm.acompletion`. Order list MUST equal `["reserve", "provider"]`. This is INV-2; failing this test means the entire D12 thesis is broken. |
| U18 | `test_deny_blocks_provider_call` | Sidecar returns DENY → `DecisionDenied` raised → mock for original `litellm.acompletion` recorded ZERO calls. |
| U19 | `test_degrade_fails_closed` | Sidecar returns DEGRADE → `SidecarUnavailable` raised → mock recorded ZERO provider calls. |
| U20 | `test_fail_open_dev_allows_on_degrade` | With `SPENDGUARD_LITELLM_FAIL_OPEN=1`, DEGRADE allows the call; no commit row reaches fake sidecar. WARN logged. |

### 2.4 Re-entry / recursion

| # | Name | What it asserts |
|---|------|-----------------|
| U21 | `test_recursion_guard_no_double_reserve` | A test that has the original `acompletion` internally call `litellm.acompletion` again (simulates LiteLLM fallback chain) records exactly ONE sidecar reserve, not two. |
| U22 | `test_in_flight_contextvar_isolated_per_task` | Two concurrent `asyncio.gather` calls each reserve independently; no cross-task contamination of `_IN_FLIGHT`. |

### 2.5 Commit / release paths

| # | Name | What it asserts |
|---|------|-----------------|
| U23 | `test_success_path_commits_with_real_usage` | Mocked response has `usage.completion_tokens = 42`; sidecar `emit_llm_call_post` recorded with `outcome=SUCCESS` + `provider_event_id` from `response.id`. |
| U24 | `test_provider_exception_releases` | Original `acompletion` raises `httpx.HTTPError`; shim calls `emit_llm_call_post(outcome=FAILURE)`; original exception re-raised. |
| U25 | `test_cancellation_releases` | `asyncio.CancelledError` mid-call → `outcome=CANCELLED`; original `CancelledError` re-raised. |
| U26 | `test_streaming_commits_at_iterator_exhaustion` | `stream=True` returns wrapped `AsyncIterator`; commit fires at `StopAsyncIteration`; uses estimator-snapshot if no `usage` frame. |

## 3. Integration tests — `test_litellm_shim_real.py`

Imports **real** `litellm` (not mocked). Mocks the upstream OpenAI HTTP endpoint via `pytest-httpx`. Sidecar is still mocked via `_fake_sidecar.py` (no docker required). Each test asserts the wire-level ordering: sidecar RPC happens before httpx records the OpenAI call.

| # | Name | What it asserts |
|---|------|-----------------|
| I01 | `test_real_litellm_acompletion_reserves_then_calls_openai` | Real `await litellm.acompletion(model="gpt-4o-mini", messages=[{...}], api_key="sk-test")`. `pytest-httpx` records the call to `api.openai.com`. `_fake_sidecar` records reserve. Strict-order check: reserve timestamp < httpx-record timestamp. |
| I02 | `test_real_litellm_acompletion_deny_zero_openai_calls` | Sidecar configured to DENY. Call raises `DecisionDenied`. `pytest-httpx` recorded ZERO requests to `api.openai.com`. |
| I03 | `test_real_litellm_completion_sync_outside_loop` | Sync `litellm.completion(...)` in a non-async test; reserve fires before HTTP. |
| I04 | `test_real_router_acompletion` | `router = litellm.Router(model_list=[...]); await router.acompletion(...)` after install — reserve fires before HTTP. |
| I05 | `test_real_atext_completion_text_endpoint` | `await litellm.atext_completion(model="gpt-3.5-turbo-instruct", prompt="hi")` — reserve fires before the `/v1/completions` endpoint hit. |
| I06 | `test_install_uninstall_real_litellm_baseline_unchanged` | `install()` then `uninstall()`; subsequent `await litellm.acompletion(...)` reaches `api.openai.com` with NO sidecar reserve (proves restore is complete). |

The strict-order check uses a sentinel `asyncio.Event` set by the fake sidecar on `RequestDecision`. The `pytest-httpx` callback checks `event.is_set()`; if False, the test fails with `out-of-order` evidence.

## 4. Transitive coverage smoke — `test_crewai_via_shim.py`

The load-bearing proof that D12 closes coverage for the 7 frameworks (CrewAI, DSPy, SmolAgents, Strands, BeeAI, AutoGen, Atomic Agents) that route through `litellm`. CrewAI is the chosen probe because it is the most widely-installed of the seven (per the strategy memo's adoption table) and exercises a non-trivial multi-step flow.

| # | Name | What it asserts |
|---|------|-----------------|
| T01 | `test_crewai_kickoff_triggers_spendguard_reserve` | `pytest.importorskip("crewai")`; create `Agent` + `Task` + `Crew`; `await crew.kickoff_async()`; assert `fake_sidecar.reserve_call_count >= 1`. |
| T02 | `test_crewai_deny_blocks_kickoff` | Sidecar configured to DENY. `crew.kickoff_async()` raises (propagated from `litellm.acompletion`); `pytest-httpx` recorded ZERO OpenAI calls. |
| T03 | `test_dspy_predict_triggers_spendguard_reserve` | `pytest.importorskip("dspy")`; configure `dspy.LM("openai/gpt-4o-mini")`; `dspy.Predict(...)("hi")`; same reserve assertion. |

T01-T03 are SKIPPED on environments without those frameworks installed. The demo driver (slice 7) installs `crewai` + `dspy` in its container so CI exercises both at least via `DEMO_MODE=litellm_sdk_real`.

## 5. Demo regression — `verify_step_litellm_sdk.sql`

Gates executed after `DEMO_MODE=litellm_sdk_real` and `DEMO_MODE=litellm_sdk_deny`. Layout mirrors `verify_step_litellm_direct.sql`.

```sql
-- D12_SDK: at least 1 reserve carrying the new 'sdk' mode literal.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM ledger_transactions
   WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
     AND operation_kind = 'reserve';
  IF c < 1 THEN
    RAISE EXCEPTION 'D12_SDK_GATE: reserve >= 1 expected, got %', c;
  END IF;
END; $$;

-- D12_SDK: at least 1 commit_estimated row.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM ledger_transactions
   WHERE operation_kind = 'commit_estimated';
  IF c < 1 THEN RAISE EXCEPTION 'D12_SDK_GATE: commit >= 1 expected'; END IF;
END; $$;

-- D12_SDK: at least 1 denied_decision (sdk_deny mode only).
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM ledger_transactions
   WHERE operation_kind = 'denied_decision';
  IF c < 1 THEN
    RAISE EXCEPTION 'D12_SDK_GATE: denied_decision >= 1 expected for deny mode';
  END IF;
END; $$;

-- D12_SDK: decision_context.mode = 'sdk' to differentiate from proxy / direct / egress.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM audit_outbox
   WHERE decision_context->>'integration' = 'litellm'
     AND decision_context->>'mode' = 'sdk';
  IF c < 1 THEN
    RAISE EXCEPTION 'D12_SDK_GATE: at least 1 audit with mode=sdk expected, got %', c;
  END IF;
END; $$;

-- D12_SDK: canonical chain received the events.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM canonical_events
   WHERE source_integration = 'litellm';
  IF c < 1 THEN RAISE EXCEPTION 'D12_SDK_GATE: canonical_events empty'; END IF;
END; $$;

-- D12_SDK: stub counter delta matches expected ALLOW count
-- (the bootstrap driver records expected count into decision_context.expected_allow_count).
-- Asserts INV-1: DENY never hits provider.
```

A 6th assertion compares the recorded stub-counter delta against the expected ALLOW count carried in `decision_context.stub_hits`. This catches the case where DENY accidentally reached the provider.

## 6. Demo driver — `run_litellm_sdk_real_mode` + `run_litellm_sdk_deny_mode`

`run_litellm_sdk_real_mode` (3 steps):

| Step | Body | Asserts |
|------|------|---------|
| 1 ALLOW | small messages, fits budget | reserve fires before provider, stub counter +1, commit row visible |
| 2 STREAM | `stream=True`, small messages | reserve before provider, stub counter +1, end-of-stream commit |
| 3 TRANSITIVE | `await crew.kickoff_async()` inside the same driver (CrewAI Agent created inline) | each LiteLLM call inside CrewAI triggers a sidecar reserve; transitive proof |

`run_litellm_sdk_deny_mode` (3 sub-steps, mirrors `run_litellm_deny_mode`):

| Step | Body | Asserts |
|------|------|---------|
| 1 ALLOW positive control | small messages | reserve + provider hit; positive control proves wiring |
| 2 DENY budget exhausted | `spendguard_estimate_override=2000000000` | `DecisionDenied`; stub counter unchanged |
| 3 DENY sidecar unreachable | resolver injects `SidecarUnavailable` | `SidecarUnavailable`; stub counter unchanged |

Each driver writes a single-line summary on success: `[demo] litellm_sdk_real ALL 3 steps PASS` / `[demo] litellm_sdk_deny ALL 3 substeps PASS`. Gate-failure exits code 7.

## 7. Negative test surface

| What | Why | Where |
|------|-----|-------|
| Provider hit on DENY | Most severe correctness bug | U18 + T02 + demo deny driver + verify SQL stub-counter delta |
| Reserve fires AFTER provider | Cancels the entire D12 thesis | U17 + I01 strict-order check |
| Recursion on internal litellm fallback | Would double-charge | U21 |
| Sync `completion()` deadlock in async context | Easy to write; silent hang in CI | U10 |
| Subclass of Router not patched | Operator with custom Router escapes gating | U13 + U14 |
| Cross-task contamination of `_IN_FLIGHT` | Would let one task disable gating in a sibling | U22 |
| Patched state survives test boundaries | Global state pollution in CI | install/uninstall fixture in every test |

## 8. Performance budgets (informational, not gates)

| Op | Target | Source |
|----|--------|--------|
| `_patched_acompletion` overhead (shim only, excluding sidecar gRPC) | < 2ms p99 | composition cost + contextvar set/reset |
| `install()` cold time | < 100ms | walks Router subclasses + patches 6 entry points |
| `uninstall()` cold time | < 50ms | reverses originals dict |

Verified manually post-merge.

## 9. CI integration

- `sdk/python/tests/integrations/test_litellm_shim.py` + `test_litellm_shim_real.py` + `test_crewai_via_shim.py` run under existing `pytest sdk/python` GitHub Actions matrix.
- New `[litellm-shim]` extra installs `litellm>=1.50` + `pytest-httpx>=0.30` for the test matrix.
- The transitive-coverage suite installs `crewai` + `dspy` in a dedicated job cell (frameworks have heavy deps; do not add to default test matrix).
- `make demo-up DEMO_MODE=litellm_sdk_real` and `DEMO_MODE=litellm_sdk_deny` run as new matrix cells in `e2e-demo`.

## 10. Test isolation rules (mandatory)

- Every test using `install()` MUST wrap in a `try/finally` calling `uninstall()`. A pytest fixture `shim_clean` enforces this with `addfinalizer`.
- No test imports `litellm_shim` and expects module-load side effects. Patching is explicit.
- No test mutates `litellm.acompletion` directly; only `install()` may.
- Concurrent tests in `pytest-xdist` workers are safe because patching is process-local AND `_IN_FLIGHT` is contextvar-scoped per task.
