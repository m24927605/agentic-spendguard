# D25 — Tests

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`acceptance.md`](acceptance.md).

Test counts targeted by acceptance gate: **20 unit + 8 integration** = 28 total. Integration tests parametrized over `InferenceClientModel` and `OpenAIServerModel`.

## 1. Unit tests — `sdk/python/tests/integrations/test_smolagents.py`

Uses a hand-rolled `FakeSmolModel` (subclass of `smolagents.Model`) so unit tests do not need any vendor SDK installed. Mocks `SpendGuardClient.request_decision` + `emit_llm_call_post` + `emit_agent_step_telemetry`.

### 1.1 Construction / contract

- `test_constructor_skips_super_init` — wrapper does not call `_SmolModel.__init__`; inner state is intact.
- `test_import_error_without_smolagents` — module import inside a sandboxed namespace without `smolagents` raises ImportError pointing at the `[smolagents]` extra.
- `test_getattr_forwards_unknown_method_to_inner` — accessing `wrapper.flatten_messages_as_text` returns the inner's bound method.
- `test_getattr_rejects_private_attrs` — `wrapper._something` raises `AttributeError` (no inner-state leakage).

### 1.2 `generate()` PRE/POST flow

- `test_generate_emits_request_decision_with_llm_call_pre_trigger` — single ALLOW round-trip; `RequestDecision` called with `trigger="LLM_CALL_PRE"`, `route="llm.call"`.
- `test_generate_passes_estimator_output_as_projected_claims` — claim estimator return value flows verbatim into `projected_claims`.
- `test_generate_post_uses_reservation_from_decision` — POST `reservation_id == decision.reservation_ids[0]`.
- `test_generate_post_estimated_amount_equals_input_plus_output_tokens` — usage extraction sums `token_usage.input_tokens + token_usage.output_tokens` (validated against `smolagents.models.TokenUsage`).
- `test_generate_post_estimated_amount_zero_when_token_usage_absent` — `result.token_usage is None` → `estimated_amount_atomic == "0"`.
- `test_generate_skips_post_when_no_reservation` — DENY path: `decision.reservation_ids == []` → no `emit_llm_call_post` call.
- `test_generate_signature_includes_stop_sequences` — different `stop_sequences` → different `llm_call_id`.
- `test_generate_signature_includes_tools_to_call_from` — different `tools_to_call_from` → different `llm_call_id`.
- `test_generate_signature_includes_response_format` — different `response_format` → different `llm_call_id`.
- `test_generate_kwargs_signature_is_sorted` — `kwargs={"b": 1, "a": 2}` and `kwargs={"a": 2, "b": 1}` produce the same signature.

### 1.3 `__call__` alias

- `test_call_alias_routes_through_generate` — `await wrapper(messages, ...)` produces the same `RequestDecision` call as `await wrapper.generate(messages, ...)`. Direct call to the inner `__call__` would bypass; this test guards against version-drift bypass.
- `test_call_alias_propagates_kwargs` — `__call__(messages, stop_sequences=[...], extra="x")` reaches inner's `generate` with identical args.

### 1.4 Exception handling

- `test_generate_failure_emits_post_failure` — inner raises `RuntimeError` → POST `outcome="FAILURE"` + re-raise.
- `test_generate_cancelled_emits_post_cancelled` — inner raises `asyncio.CancelledError` → POST `outcome="CANCELLED"` + re-raise (uses `type(exc).__name__ == "CancelledError"` per D24 pattern).
- `test_generate_failure_skips_post_when_no_reservation` — DENY-then-fail path is unreachable but defensively asserted.

### 1.5 `spendguard_step_callback` helper

- `test_step_callback_emits_telemetry_on_action_step` — callable invoked with an `ActionStep`-shaped object → `client.emit_agent_step_telemetry` called with `step_kind="ActionStep"`.
- `test_step_callback_emits_telemetry_on_planning_step` — `PlanningStep` → `step_kind="PlanningStep"`.
- `test_step_callback_swallows_exceptions` — `client.emit_agent_step_telemetry` raises `RuntimeError` → callable returns `None`, warning logged, **no propagation**. This is load-bearing: a raise would abort the host agent.
- `test_step_callback_does_not_call_request_decision` — informational path MUST NOT invoke the gating decision RPC.

### 1.6 Run context

- `test_generate_raises_without_active_run_context` — calling `generate()` outside `run_context()` raises `RuntimeError` with the same message contract as `openai_agents.current_run_context`.

## 2. Integration tests — `sdk/python/tests/integrations/test_smolagents_real.py`

```python
import pytest

INNERS = []
try:
    from smolagents import InferenceClientModel  # noqa: F401
    INNERS.append("inference_client")
except ImportError:
    pass
try:
    from smolagents import OpenAIServerModel  # noqa: F401
    INNERS.append("openai_server")
except ImportError:
    pass


@pytest.fixture(params=INNERS)
def inner_model(request, httpx_mock):
    """Construct an inner Model wired to a pytest-httpx mock transport."""
    if request.param == "inference_client":
        from smolagents import InferenceClientModel
        # InferenceClientModel routes via HF Inference HTTP; httpx_mock
        # intercepts.
        return InferenceClientModel(model_id="meta-llama/Llama-3.2-1B-Instruct")
    if request.param == "openai_server":
        from smolagents import OpenAIServerModel
        return OpenAIServerModel(
            model_id="gpt-4o-mini",
            api_base="https://api.openai.example/v1",
            api_key="test",
        )
    pytest.skip(f"unsupported inner {request.param}")
```

### 2.1 End-to-end per inner (parametrized)

- `test_code_agent_run_with_spendguard_wrap[inference_client]` and `[openai_server]` — instantiate `CodeAgent(model=SpendGuardSmolModel(inner=inner_model, ...), tools=[])`, call `agent.arun("solve 2+2")` inside `run_context()`, assert at least one `RequestDecision` fired with `trigger="LLM_CALL_PRE"` BEFORE any HTTP request reached the mock transport.
- `test_code_agent_deny_path[inference_client]` and `[openai_server]` — sidecar returns DENY → `agent.arun()` raises `SpendGuardDenied`. **Critically:** assert via `httpx_mock` that **zero HTTP requests** reached the inner transport. Fail-closed at PRE boundary.
- `test_tool_calling_agent_run_with_spendguard_wrap[inference_client]` and `[openai_server]` — `ToolCallingAgent` (not `CodeAgent`) variant covers the parallel agent-class code path.
- `test_polyglot_run_context_shared[openai_server]` — wrap one SmolAgents `CodeAgent` with D25, an `openai_agents.Agent` step with `SpendGuardAgentsModel`, both inside the same `run_context()` → both audit rows share `run_id`.

### 2.2 Step callback integration

- `test_step_callbacks_fire_for_each_action_step` — `CodeAgent(model=guarded, step_callbacks=[spendguard_step_callback(client, run_id=...)])` after a 2-step run produces 2 `emit_agent_step_telemetry` calls + 2 `RequestDecision` calls. Gating and telemetry are independent.

### 2.3 LiteLLMModel transitive coverage smoke

- `test_litellm_inner_is_documented_not_wrapped` — assert the docs page contains a link to `litellm-sdk-shim.md` and that wrapping `LiteLLMModel` is documented as redundant with the D12 shim. Implementation-level smoke: with D12 `install()` active and a raw (unwrapped) `LiteLLMModel`, `agent.run(...)` triggers SpendGuard reserves through the shim. Marked `@pytest.mark.skipif(not has_litellm_shim, ...)`.

## 3. Demo-mode regression

`deploy/demo/demo/run_demo.py` gains `run_agent_real_smolagents_mode()`. It:

1. Constructs `CodeAgent(model=SpendGuardSmolModel(inner=OpenAIServerModel(api_base="http://wiremock:8080/v1", api_key="test")), tools=[...])`.
2. Wraps with a budget that allows call #1 and denies call #2.
3. Calls `agent.arun("question 1")` then `agent.arun("question 2")` inside one `run_context()`.
4. Asserts:
   - Call #1 returns a non-empty result.
   - Call #2 raises `SpendGuardDenied` from the wrapper PRE boundary.
   - Postgres `audit_outbox` shows exactly 1 `LLM_CALL_PRE`+`LLM_CALL_POST` pair with `outcome='SUCCESS'` for #1, and exactly 1 `LLM_CALL_PRE` with `decision='DENY'` for #2 (no `POST` for #2).

`verify_step_smolagents.sql` encodes the row assertions.

## 4. Coverage matrix

| Surface | Unit | Integration | Demo |
|---------|------|-------------|------|
| `generate()` ALLOW | ✓ | ✓ ×2 inners | ✓ |
| `generate()` DENY (fail-closed before inner HTTP) | ✓ | ✓ ×2 inners | ✓ |
| `generate()` FAILURE | ✓ | — | — |
| `generate()` CANCELLED | ✓ | — | — |
| `__call__` alias routes through `generate` | ✓ ×2 | — | — |
| `_extract_total_tokens` from `TokenUsage` | ✓ ×2 | ✓ (real shape) | ✓ |
| Signature determinism (stop / tools / response_format / kwargs) | ✓ ×4 | — | — |
| `step_callbacks` helper telemetry | ✓ ×4 | ✓ | — |
| `step_callbacks` swallows exceptions | ✓ | — | — |
| Polyglot trace sharing with `openai_agents` | — | ✓ | — |
| LiteLLMModel transitive via D12 | — | ✓ (skipif) | — |
| `__getattr__` forward to inner | ✓ ×2 | — | — |

## 5. Test infrastructure

- `pytest-asyncio` (already in dev extras).
- `pytest-httpx` for ordering assertions on the deny path (already used by D11/D12/D24).
- New conftest `sdk/python/tests/integrations/conftest_smolagents.py` with `FakeSmolModel` returning configurable `ChatMessage(token_usage=TokenUsage(...))`.
- Integration tests use `pytest.mark.skipif(not INNERS, ...)` so CI without `smolagents` installed reports SKIPPED, not failure.
- Wiremock fixture for `agent_real_smolagents` demo replays a stub OpenAI-compatible chat-completions response (existing infra; reuse `deploy/demo/wiremock/openai/` mappings).
