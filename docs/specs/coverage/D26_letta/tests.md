# D26 — Tests

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`acceptance.md`](acceptance.md).

Test counts targeted by acceptance gate: **18 unit + 6 integration** = 24 total.

## 1. Unit tests — `sdk/python/tests/integrations/test_letta.py`

Uses a hand-rolled `FakeLLMClient` (subclass of `letta.llm_api.llm_client_base.LLMClientBase`) so unit tests don't require a real OpenAI/Anthropic key. Mocks `SpendGuardClient.request_decision` + `emit_llm_call_post`.

### 1.1 Construction / contract

- `test_constructor_skips_super_init` — wrapper does not call `LLMClientBase.__init__`; inner state intact and accessible via `__getattr__`.
- `test_import_error_without_letta` — module import inside a sandboxed namespace without `letta.llm_api.llm_client_base` raises ImportError pointing at the `[letta]` extra and `letta>=0.8,<1.0`.
- `test_wrap_llm_client_factory_returns_spendguard_letta_client` — factory composition correctness.
- `test_getattr_delegates_to_inner` — `wrapped.llm_config is inner.llm_config`, `wrapped.provider == inner.provider`, `wrapped.build_request_data(...) == inner.build_request_data(...)`.
- `test_getattr_does_not_shadow_explicit_attrs` — `wrapped._client is sg_client` even though `inner._client` exists.

### 1.2 `send_llm_request()` PRE/POST flow

- `test_send_llm_request_emits_request_decision_with_llm_call_pre_trigger` — single ALLOW round-trip; assert `RequestDecision` called with `trigger="LLM_CALL_PRE"`, `route="llm.call"`.
- `test_send_llm_request_passes_estimator_output_as_projected_claims` — claim estimator return value flows verbatim into `projected_claims`.
- `test_send_llm_request_post_uses_reservation_from_decision` — POST `reservation_id == decision.reservation_ids[0]`.
- `test_send_llm_request_post_estimated_amount_uses_total_tokens_when_present` — `usage.total_tokens=42` → `estimated_amount_atomic="42"`.
- `test_send_llm_request_post_estimated_amount_falls_back_to_prompt_plus_completion` — `usage.total_tokens=None, prompt=10, completion=15` → `"25"`.
- `test_send_llm_request_post_estimated_amount_zero_when_usage_absent` — `result.usage is None` → `"0"`.
- `test_send_llm_request_skips_post_when_no_reservation` — DENY path: `decision.reservation_ids == []` → no `emit_llm_call_post` call.
- `test_send_llm_request_signature_includes_tools` — different `tools` → different `llm_call_id` → different idempotency key.
- `test_send_llm_request_signature_includes_force_tool_use` — `force_tool_use=True` vs `False` → different `llm_call_id`.
- `test_send_llm_request_provider_event_id_from_result_id` — POST `provider_event_id` reflects `result.id`.

### 1.3 Exception handling

- `test_send_llm_request_failure_emits_post_failure` — inner raises `RuntimeError` → POST `outcome="FAILURE"` + re-raise.
- `test_send_llm_request_cancelled_emits_post_cancelled` — inner raises `asyncio.CancelledError` → POST `outcome="CANCELLED"` + re-raise.

### 1.4 Sync path

- `test_send_llm_request_sync_outside_loop_runs_async_path` — call `wrapped.send_llm_request_sync(...)` from a fresh thread with no loop → succeeds and emits PRE/POST.
- `test_send_llm_request_sync_inside_running_loop_raises` — inside an `asyncio.run()` coroutine, calling `send_llm_request_sync` raises `RuntimeError` whose message contains both `send_llm_request_sync` and the async-path pointer.

### 1.5 Run context

- `test_send_llm_request_raises_without_active_run_context` — calling `send_llm_request` outside `run_context()` raises `RuntimeError` with the same message contract as `openai_agents.current_run_context`.

## 2. Integration tests — `sdk/python/tests/integrations/test_letta_real.py`

```python
import pytest

letta = pytest.importorskip("letta", minversion="0.8")
from letta.llm_api.openai_client import OpenAIClient  # noqa: E402
```

### 2.1 End-to-end with real Letta

- `test_real_letta_openai_client_round_trip` — wrap a `FakeHTTPLLMClient` (provider HTTP mocked via `pytest-httpx`); call `wrapped.send_llm_request(...)` directly; assert one `LLM_CALL_PRE` + one paired `LLM_CALL_POST` row.
- `test_real_letta_agent_step_uses_wrapper` — instantiate Letta `Agent` with `llm_client=wrapped`, call `agent.step(message)`. Assert `RequestDecision` fired at least once. Memory recall (Letta's built-in archival memory) MUST NOT call `RequestDecision` for embedding lookups (embedding gating is out of scope).
- `test_real_letta_deny_path_zero_provider_http` — sidecar returns DENY → `agent.step()` raises `SpendGuardDenied`. **Critically:** assert via `pytest-httpx` request inspection that ZERO HTTP requests reached the inner OpenAI transport.
- `test_real_letta_passthrough_build_request_data` — `wrapped.build_request_data(...)` returns identical output to `inner.build_request_data(...)`. Documents that wrapper is transparent for non-gated methods.

### 2.2 Polyglot trace sharing

- `test_polyglot_run_context_shared_with_openai_agents` — one agent step uses D26 wrap, a downstream step uses `openai_agents.SpendGuardAgentsModel`, both inside the same `run_context()` → both audit rows share `run_id`.

### 2.3 Server-mode redirect (negative test)

- `test_letta_server_mode_redirect_documented` — assert `docs/site/docs/integrations/letta.md` decision-table row for `letta server` exists and points at D02/D03. This is a docs-presence test; it guards against the docs page silently shifting to lead with D26 when the server-mode path is the recommended one.

## 3. Demo-mode regression

`deploy/demo/demo/run_demo.py` gains `run_agent_real_letta_mode()`:

1. Constructs a Letta `Agent` with a `FakeHTTPLLMClient` wrapped by `SpendGuardLettaClient`.
2. Configures a budget that allows the first call and denies the second (budget exhaustion).
3. Asserts:
   - First `agent.step(...)` returns a non-empty response.
   - Second `agent.step(...)` raises `SpendGuardDenied` from the wrapper PRE boundary.
   - Postgres `audit_outbox` shows exactly 1 `LLM_CALL_PRE`+`LLM_CALL_POST` pair with `outcome='SUCCESS'` for call #1, and exactly 1 `LLM_CALL_PRE` with `decision='DENY'` for call #2 (no `POST` for #2).

`verify_step_letta.sql` reused.

## 4. Coverage matrix

| Surface | Unit | Integration | Demo |
|---------|------|-------------|------|
| `send_llm_request()` ALLOW | ✓ | ✓ | ✓ |
| `send_llm_request()` DENY (fail-closed before inner HTTP) | ✓ | ✓ zero-HTTP assertion | ✓ |
| `send_llm_request()` FAILURE | ✓ | — | — |
| `send_llm_request()` CANCELLED | ✓ | — | — |
| `send_llm_request_sync()` outside loop | ✓ | — | — |
| `send_llm_request_sync()` inside loop raises | ✓ | — | — |
| `__getattr__` delegation (`llm_config`, `provider`, `build_request_data`, ...) | ✓ ×3 | ✓ ×1 | — |
| Polyglot trace sharing with `openai_agents` | — | ✓ | — |
| Server-mode redirect docs presence | — | ✓ (docs assert) | — |

## 5. Test infrastructure

- `pytest-asyncio` (already in dev extras).
- `pytest-httpx` for ordering / zero-HTTP assertions on the deny path (already used by D11/D12/D24).
- New conftest `sdk/python/tests/integrations/conftest_letta.py` with:
  - `FakeLLMClient` — minimal `LLMClientBase` subclass returning configurable `ChatCompletionResponse(usage=Usage(...))`.
  - `FakeHTTPLLMClient` — `OpenAIClient`-shaped client whose HTTP transport is intercepted by `pytest-httpx`.
- Integration tests use `pytest.importorskip("letta", minversion="0.8")` so CI without Letta installed reports SKIPPED, not failure.
