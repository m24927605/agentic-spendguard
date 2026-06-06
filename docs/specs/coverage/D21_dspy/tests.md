# D21 — Tests

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`acceptance.md`](acceptance.md).

## 1. Test inventory

| Tier | Suite | Path | Count |
|------|-------|------|-------|
| Unit (mock dspy.LM) | `test_dspy.py` | `sdk/python/tests/integrations/test_dspy.py` | 16 |
| Integration (real dspy + pytest-httpx) | `test_dspy_real.py` | `sdk/python/tests/integrations/test_dspy_real.py` | 5 |
| Demo regression | `verify_step_agent_real_dspy.sql` | `deploy/demo/verify_step_agent_real_dspy.sql` | 5 SQL assertions |
| Demo driver | `run_demo.py::run_dspy_real_mode` | `deploy/demo/demo/run_demo.py` | 3 steps |

Total new test surface: ~460 LOC tests + ~70 LOC SQL gates + ~150 LOC demo driver.

## 2. Unit tests — `test_dspy.py`

Uses `_fake_sidecar.py` (shared with D11 / D12 surfaces) for the SpendGuard RPC mock. Mocks `dspy.LM` via a minimal subclass:

```python
class _MockLM:
    model = "openai/gpt-4o-mini"
```

DSPy `BaseCallback` invocation is simulated by directly calling `callback.on_lm_start(call_id, instance=_MockLM(), inputs={...})` / `callback.on_lm_end(call_id, outputs=..., exception=...)` — no real dspy required for unit suite. Real-dspy verification is in §3.

### 2.1 Module / registry lifecycle

| # | Name | What it asserts |
|---|------|-----------------|
| U01 | `test_import_error_when_dspy_missing` | Module-level `dspy` import patched to raise → `ImportError` with `pip install 'spendguard-sdk[dspy]'` substring. |
| U02 | `test_run_context_default_factory_emits_uuid7` | When `run_context_factory` is None, factory emits a fresh UUIDv7 per call (two calls → two distinct run_ids). |
| U03 | `test_pending_registry_ttl_sweep_drops_old_entries` | Inject a stale `_CallState` (started_at - 600s); next `on_lm_start` sweeps it; WARN log captured. |
| U04 | `test_shared_contextvar_is_same_object_as_d12` | `spendguard._litellm_shim._IN_FLIGHT is spendguard.integrations.dspy._SHIM_IN_FLIGHT`. Single object identity. |

### 2.2 `on_lm_start` happy path + state

| # | Name | What it asserts |
|---|------|-----------------|
| U05 | `test_on_lm_start_calls_request_decision` | Fake sidecar records exactly one `RequestDecision` with `trigger="LLM_CALL_PRE"`, `route="llm.call"`. |
| U06 | `test_on_lm_start_records_pending_state` | After call, `_PENDING[call_id]` holds a `_CallState` with `reservation_id` from outcome. |
| U07 | `test_on_lm_start_sets_in_flight_contextvar` | Inside `on_lm_start`, `_SHIM_IN_FLIGHT.get() == True`. After successful return, value remains True (cleared by `on_lm_end`). |
| U08 | `test_callback_first_in_dspy_callbacks_list_documented` | Doc string of `SpendGuardDSPyCallback.__init__` references "MUST appear FIRST" — guards against accidental doc drift. |

### 2.3 Reserve-before-provider ordering (LOAD-BEARING)

| # | Name | What it asserts |
|---|------|-----------------|
| U09 | `test_reserve_fires_before_lm_provider_call` | Test instrumentation records list of events: callback list iterated; `on_lm_start` event recorded BEFORE the simulated `dspy.LM.__call__` HTTP. Order list MUST equal `["reserve", "provider"]`. This is INV-2; failing means D21 thesis is broken. |
| U10 | `test_deny_blocks_provider_call` | Sidecar returns DENY → `DecisionDenied` raised → mock provider HTTP recorded ZERO calls AND `_PENDING` is empty AND `_SHIM_IN_FLIGHT.get() == False`. |
| U11 | `test_degrade_fails_closed` | Sidecar returns DEGRADE → `SidecarUnavailable` raised → mock provider zero calls. |
| U12 | `test_async_context_raises` | Inside `pytest.mark.asyncio` test, `callback.on_lm_start(...)` raises `SpendGuardDSPyCallback.SyncInAsyncContext` with hint pointing at sync entrypoint. |

### 2.4 `on_lm_end` commit / release paths

| # | Name | What it asserts |
|---|------|-----------------|
| U13 | `test_on_lm_end_commits_with_real_usage` | Mock outputs have `usage = {"total_tokens": 42}`; fake sidecar records `emit_llm_call_post(outcome="SUCCESS", estimated_amount_atomic="42")`. |
| U14 | `test_on_lm_end_failure_outcome_propagates` | `on_lm_end(call_id, outputs=None, exception=httpx.HTTPError)`; sidecar records `emit_llm_call_post(outcome="FAILURE")`; `_PENDING` cleared; `_SHIM_IN_FLIGHT` reset. |
| U15 | `test_on_lm_end_cancellation_outcome` | `exception=asyncio.CancelledError` → `outcome="CANCELLED"`. |
| U16 | `test_on_lm_end_without_start_logs_and_returns` | `on_lm_end` for unknown `call_id` logs WARN and returns; no commit fires; no exception. |
| U17 | `test_custom_lm_subclass_no_usage_falls_back` | Mock outputs missing `.usage` → `_extract_total_tokens` returns 0; commit fires with `estimated_amount_atomic="0"`. WARN log captured. |
| U18 | `test_in_flight_reset_after_on_lm_end` | After `on_lm_end`, `_SHIM_IN_FLIGHT.get() == False`. Token reset is durable across the start/end pair. |

(Note: U18 in §2.4 collides with the count above — final test count is **16** unit tests; renumber on impl per `tests.md` is acceptable.)

## 3. Integration tests — `test_dspy_real.py`

Imports **real** `dspy` (not mocked). Mocks the upstream OpenAI HTTP endpoint via `pytest-httpx`. Sidecar is mocked via `_fake_sidecar.py`. Each test asserts wire-level ordering: sidecar RPC happens before httpx records the OpenAI call.

| # | Name | What it asserts |
|---|------|-----------------|
| I01 | `test_real_dspy_predict_reserves_then_calls_openai` | `dspy.configure(lm=dspy.LM("openai/gpt-4o-mini"), callbacks=[callback])`; `dspy.Predict("question -> answer")(question="hi")`. `pytest-httpx` records OpenAI call. `_fake_sidecar` records reserve. Strict order: reserve event set before httpx record. |
| I02 | `test_real_dspy_chain_of_thought_end_to_end` | `dspy.ChainOfThought("question -> answer")(question="2+2?")`. Asserts callback fired exactly once for the wrapped LM call. `result.answer` is the mocked response content. |
| I03 | `test_real_dspy_deny_zero_openai_calls` | Sidecar configured to DENY. Call raises `DecisionDenied` (propagated from inside DSPy). `pytest-httpx` recorded ZERO requests to `api.openai.com`. |
| I04 | `test_real_dspy_with_d12_shim_no_double_reserve` | Both `D21` callback AND `D12` shim active. Single `dspy.Predict(...)` call → fake sidecar records exactly ONE reserve (not two). Asserts the `_IN_FLIGHT` contextvar coordination works. |
| I05 | `test_real_dspy_callback_first_in_list` | When `SpendGuardDSPyCallback` is the FIRST callback in the list, reserve fires before a user-provided observer callback (whose `on_lm_start` appends to a sentinel list). Ordering: reserve event timestamp < observer timestamp. |

The strict-order check uses `asyncio.Event` set by the fake sidecar on `RequestDecision`. The `pytest-httpx` callback checks `event.is_set()`; if False, the test fails with `out-of-order` evidence.

## 4. Demo regression — `verify_step_agent_real_dspy.sql`

Gates executed after `DEMO_MODE=agent_real_dspy`. Layout mirrors `verify_step_litellm_direct.sql`.

```sql
-- D21_DSPY: at least 1 reserve from the dspy demo
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM ledger_transactions
   WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
     AND operation_kind = 'reserve';
  IF c < 1 THEN
    RAISE EXCEPTION 'D21_DSPY_GATE: reserve >= 1 expected, got %', c;
  END IF;
END; $$;

-- D21_DSPY: at least 1 commit_estimated row
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM ledger_transactions
   WHERE operation_kind = 'commit_estimated';
  IF c < 1 THEN RAISE EXCEPTION 'D21_DSPY_GATE: commit >= 1 expected'; END IF;
END; $$;

-- D21_DSPY: at least 1 denied_decision (deny substep)
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM ledger_transactions
   WHERE operation_kind = 'denied_decision';
  IF c < 1 THEN
    RAISE EXCEPTION 'D21_DSPY_GATE: denied_decision >= 1 expected';
  END IF;
END; $$;

-- D21_DSPY: decision_context.integration = 'dspy' to differentiate
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM audit_outbox
   WHERE decision_context->>'integration' = 'dspy';
  IF c < 1 THEN
    RAISE EXCEPTION 'D21_DSPY_GATE: at least 1 audit with integration=dspy expected, got %', c;
  END IF;
END; $$;

-- D21_DSPY: canonical chain received the events
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM canonical_events
   WHERE source_integration = 'dspy';
  IF c < 1 THEN RAISE EXCEPTION 'D21_DSPY_GATE: canonical_events empty'; END IF;
END; $$;
```

## 5. Demo driver — `run_dspy_real_mode`

`run_dspy_real_mode` (3 steps):

| Step | Body | Asserts |
|------|------|---------|
| 1 ALLOW | `dspy.ChainOfThought("question -> answer")(question="2+2?")` — fits budget | reserve fires before provider; stub counter +1; commit row visible; `result.answer` non-empty |
| 2 DENY | resolver wired to return a budget binding that triggers DENY via `spendguard_estimate_override=2000000000` | `DecisionDenied`; stub counter unchanged; `denied_decision` row added |
| 3 CUSTOM-LM | inline custom `dspy.LM` subclass with `model="custom-bypass"` that calls the OpenAI HTTP directly | reserve fires before custom LM HTTP; commit fires; demonstrates direct-path coverage independent of LiteLLM |

Each driver writes a single-line summary on success: `[demo] agent_real_dspy ALL 3 steps PASS`. Gate-failure exits code 7.

## 6. Negative test surface

| What | Why | Where |
|------|-----|-------|
| Provider hit on DENY | Most severe correctness bug | U10 + I03 + demo step 2 + verify SQL stub-counter delta |
| Reserve fires AFTER provider | Cancels the entire D21 thesis | U09 + I01 strict-order check |
| Double reserve when D12 also installed | Cost over-charge | I04 |
| Sync callback in async context (deadlock) | Easy to misconfigure | U12 |
| `_PENDING` leak when DSPy crashes mid-call | Memory growth in long-running services | U03 TTL sweep |
| `_IN_FLIGHT` left True after exception | D12 wrapper permanently blocked | U10 + U14 + U15 |

## 7. Performance budgets (informational, not gates)

| Op | Target | Source |
|----|--------|--------|
| `on_lm_start` overhead (callback only, excluding sidecar gRPC) | < 3ms p99 | TTL sweep + signature hash + contextvar set |
| `on_lm_end` overhead (callback only, excluding sidecar gRPC) | < 2ms p99 | dict pop + commit dispatch |
| TTL sweep cost when `_PENDING` empty | < 50µs | early-return check |
| TTL sweep cost when `_PENDING` has 100 entries | < 1ms | linear scan |

Verified manually post-merge.

## 8. CI integration

- `sdk/python/tests/integrations/test_dspy.py` runs under existing `pytest sdk/python` GitHub Actions matrix.
- `test_dspy_real.py` requires `pytest.importorskip("dspy")` so it skips on environments without the framework — but the `agent_real_dspy` demo-mode CI cell installs `dspy-ai>=2.6` so the path is covered there.
- New `[dspy]` extra installs `dspy-ai>=2.6` + `pytest-httpx>=0.30` for the test matrix.
- `make demo-up DEMO_MODE=agent_real_dspy` runs as a new matrix cell in `e2e-demo`.

## 9. Test isolation rules (mandatory)

- Every test that calls `on_lm_start` MUST pair with `on_lm_end` (or assert the exception path triggered cleanup). A pytest fixture `dspy_pending_clean` checks `_PENDING == {}` at teardown.
- Every test that sets `_SHIM_IN_FLIGHT` MUST assert it returns to `False` at teardown via the same fixture.
- No test imports `spendguard.integrations.dspy` and expects module-load side effects. Registration via `dspy.configure(callbacks=[...])` is explicit.
- Concurrent tests in `pytest-xdist` workers are safe because `_PENDING` is a module dict keyed on UUID `call_id` (unique per call) and `_SHIM_IN_FLIGHT` is `contextvars.ContextVar` (per-task).
