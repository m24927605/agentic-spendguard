# D24 — Tests

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`acceptance.md`](acceptance.md).

Test counts targeted by acceptance gate: **20 unit + 8 integration** = 28 total. All integration tests parametrized over both lineages.

## 1. Unit tests — `sdk/python/tests/integrations/test_autogen.py`

Uses a hand-rolled `FakeChatCompletionClient` (subclass of `autogen_core.models.ChatCompletionClient`) so unit tests don't require either lineage of agentchat installed. Mocks `SpendGuardClient.request_decision` + `emit_llm_call_post`.

### 1.1 Construction / contract

- `test_constructor_skips_super_init` — wrapper does not call `_ChatCompletionClient.__init__`; verifies inner state is intact.
- `test_import_error_without_autogen_core` — module import inside a sandboxed namespace without `autogen_core` raises ImportError pointing at the `[autogen]` extra.
- `test_lineage_probe_core_only` — when neither `autogen_agentchat` nor `ag2` installed, `LINEAGE == "core-only"`.
- `test_lineage_probe_both_installed` — when both installed, `LINEAGE == "both"`.

### 1.2 `create()` PRE/POST flow

- `test_create_emits_request_decision_with_llm_call_pre_trigger` — single ALLOW round-trip; assert `RequestDecision` called with `trigger="LLM_CALL_PRE"`, `route="llm.call"`.
- `test_create_passes_estimator_output_as_projected_claims` — claim estimator return value flows verbatim into `projected_claims`.
- `test_create_post_uses_reservation_from_decision` — POST `reservation_id == decision.reservation_ids[0]`.
- `test_create_post_estimated_amount_equals_prompt_plus_completion_tokens` — usage extraction sums prompt + completion (validated against `autogen_core.models.RequestUsage`).
- `test_create_post_estimated_amount_zero_when_usage_absent` — `result.usage is None` → `estimated_amount_atomic == "0"`.
- `test_create_skips_post_when_no_reservation` — DENY path: `decision.reservation_ids == []` → no `emit_llm_call_post` call.
- `test_create_propagates_extra_create_args_to_inner_verbatim` — wrapper does not mutate `extra_create_args` (copy semantics).
- `test_create_signature_includes_tools` — different `tools` arg → different `llm_call_id` → different idempotency key.
- `test_create_signature_includes_extra_create_args` — different `extra_create_args` → different `llm_call_id`.

### 1.3 Exception handling

- `test_create_failure_emits_post_failure` — inner raises `RuntimeError` → POST `outcome="FAILURE"` + re-raise.
- `test_create_cancelled_emits_post_cancelled` — inner raises `asyncio.CancelledError` → POST `outcome="CANCELLED"` + re-raise.
- `test_create_failure_skips_post_when_no_reservation` — DENY-then-fail path is unreachable but defensively asserted.

### 1.4 `create_stream()` POC

- `test_create_stream_passes_through_to_inner` — wrapper's `create_stream` returns inner's async iterator unchanged.
- `test_create_stream_does_not_call_request_decision` — POC scope: stream path does NOT fire PRE/POST (bracketed at next `create()` boundary). Test asserts this is the documented behavior, not a regression.

### 1.5 Pass-through introspection

- `test_count_tokens_pass_through`
- `test_total_usage_pass_through`
- `test_actual_usage_pass_through`
- `test_remaining_tokens_pass_through`
- `test_capabilities_pass_through`

### 1.6 Run context

- `test_create_raises_without_active_run_context` — calling `create()` outside `run_context()` raises `RuntimeError` with the same message contract as `openai_agents.current_run_context`.

## 2. Integration tests — `sdk/python/tests/integrations/test_autogen_real.py`

```python
import pytest

LINEAGES = []
try:
    import autogen_agentchat  # noqa
    LINEAGES.append("autogen")
except ImportError:
    pass
try:
    import ag2  # noqa
    LINEAGES.append("ag2")
except ImportError:
    pass


@pytest.fixture(params=LINEAGES)
def assistant_agent_cls(request):
    """AssistantAgent class for the requested lineage."""
    if request.param == "autogen":
        from autogen_agentchat.agents import AssistantAgent
        return AssistantAgent
    if request.param == "ag2":
        from ag2.agents import AssistantAgent  # AG2 mirrors namespace
        return AssistantAgent
    pytest.skip(f"unsupported lineage {request.param}")
```

### 2.1 End-to-end per lineage (parametrized)

- `test_assistant_agent_on_messages_with_spendguard_wrap[autogen]` and `[ag2]` — instantiate `AssistantAgent(model_client=SpendGuardChatCompletionClient(inner=<FakeOpenAILike>))`, call `on_messages([UserMessage(...)], cancellation_token)`, assert at least one `RequestDecision` fired.
- `test_assistant_agent_deny_path[autogen]` and `[ag2]` — sidecar returns DENY → `AssistantAgent.on_messages()` raises `SpendGuardDenied`. **Critically:** assert the inner client's HTTP transport (mocked via `pytest-httpx`) NEVER received a request — fail-closed at PRE boundary.
- `test_assistant_agent_stream_path[autogen]` and `[ag2]` — `on_messages_stream()` succeeds (POC: stream itself isn't gated; finalization `create()` IS).
- `test_polyglot_run_context_shared[autogen]` and `[ag2]` — wrap one agent with D24, another step with `openai_agents.SpendGuardAgentsModel`, both inside the same `run_context()` → both audit rows share `run_id`.

### 2.2 Pass-through with real inner

- `test_count_tokens_with_real_inner[autogen]` and `[ag2]` — `SpendGuardChatCompletionClient.count_tokens(messages)` matches inner's count exactly (no mutation).

## 3. Demo-mode regression

`deploy/demo/demo/run_demo.py` gains `run_agent_real_autogen_mode()` and `run_agent_real_ag2_mode()`. Each:

1. Constructs the appropriate `AssistantAgent` per lineage.
2. Wraps the inner client in `SpendGuardChatCompletionClient` with a budget that allows the first call and denies the second (budget exhaustion).
3. Asserts:
   - First call returns a `CreateResult` with non-empty content.
   - Second call raises `SpendGuardDenied` from the wrapper PRE boundary.
   - Postgres `audit_outbox` shows exactly 1 `LLM_CALL_PRE`+`LLM_CALL_POST` pair with `outcome='SUCCESS'` for call #1, and exactly 1 `LLM_CALL_PRE` with `decision='DENY'` for call #2 (no `POST` for #2).

`verify_step_autogen.sql` is shared across both modes and parametrized via `psql -v lineage=`.

## 4. Coverage matrix

| Surface | Unit | Integration | Demo |
|---------|------|-------------|------|
| `create()` ALLOW | ✓ | ✓ ×2 lineages | ✓ ×2 modes |
| `create()` DENY (fail-closed before inner HTTP) | ✓ | ✓ ×2 lineages | ✓ ×2 modes |
| `create()` FAILURE | ✓ | — | — |
| `create()` CANCELLED | ✓ | — | — |
| `create_stream()` POC pass-through | ✓ | ✓ ×2 lineages | — |
| `count_tokens` / `total_usage` etc. | ✓ ×5 | ✓ ×2 lineages | — |
| `LINEAGE` probe | ✓ ×4 paths | — | — |
| Polyglot trace sharing with `openai_agents` | — | ✓ ×2 lineages | — |

## 5. Test infrastructure

- `pytest-asyncio` (already in dev extras).
- `pytest-httpx` for ordering assertions on the deny path (already used by D11/D12).
- New conftest `sdk/python/tests/integrations/conftest_autogen.py` with `FakeChatCompletionClient` returning configurable `CreateResult(usage=RequestUsage(...))`.
- Integration tests use `pytest.mark.skipif(not LINEAGES, ...)` so CI without either lineage installed reports SKIPPED, not failure.
