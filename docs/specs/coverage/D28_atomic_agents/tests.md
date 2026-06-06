# D28 — Tests

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`acceptance.md`](acceptance.md).

Test counts targeted by acceptance gate: **22 unit + 7 integration** = 29 total.

## 1. Unit tests — `sdk/python/tests/integrations/test_atomic_agents.py`

Uses hand-rolled `FakeInstructor` / `FakeAsyncInstructor` that mimic the `instructor.Instructor` / `instructor.AsyncInstructor` interface (`.chat.completions.create`, `.chat.completions.create_with_completion`) without requiring a real OpenAI key. Mocks `SpendGuardClient.request_decision*` + `emit_llm_call_post*`.

### 1.1 Construction / contract

- `test_import_error_without_instructor` — module import inside a sandboxed namespace without `instructor` raises ImportError pointing at the `[atomic-agents]` extra.
- `test_import_error_without_atomic_agents` — similarly for missing `atomic_agents`.
- `test_wrap_instructor_client_returns_sync_proxy_for_sync_instructor` — factory dispatch on `Instructor` returns `SpendGuardInstructorProxy`.
- `test_wrap_instructor_client_returns_async_proxy_for_async_instructor` — factory dispatch on `AsyncInstructor` returns `SpendGuardAsyncInstructorProxy`.
- `test_wrap_instructor_client_rejects_raw_openai_client` — passing a bare `openai.OpenAI` instance raises `TypeError` whose message points at `instructor.from_openai`.
- `test_getattr_delegates_to_inner` — `proxy.mode is inner.mode`, `proxy.create_kwargs == inner.create_kwargs` (any non-overridden attribute reaches inner).
- `test_getattr_does_not_shadow_explicit_attrs` — `proxy._client is sg_client` even when `inner._client` exists.

### 1.2 Sync `create_with_completion` PRE/POST flow

- `test_sync_create_with_completion_emits_request_decision_with_llm_call_pre_trigger` — single ALLOW round trip; assert `request_decision_sync` called with `trigger="LLM_CALL_PRE"`, `route="llm.call"`.
- `test_sync_create_with_completion_passes_estimator_output_as_projected_claims` — claim estimator return value flows verbatim into `projected_claims`. Estimator receives the full `kwargs` dict (`model`, `messages`, `response_model`, `tools`, `tool_choice`).
- `test_sync_create_with_completion_post_uses_reservation_from_decision` — POST `reservation_id == decision.reservation_ids[0]`.
- `test_sync_create_with_completion_post_estimated_amount_uses_total_tokens_from_raw_completion` — `raw_completion.usage.total_tokens=42` → `estimated_amount_atomic="42"`.
- `test_sync_create_with_completion_post_estimated_amount_falls_back_to_prompt_plus_completion` — `usage.total_tokens=None, prompt=10, completion=15` → `"25"`.
- `test_sync_create_with_completion_post_estimated_amount_zero_when_usage_absent` — `raw_completion.usage is None` → `"0"`.
- `test_sync_create_with_completion_skips_post_when_no_reservation` — DENY path: `decision.reservation_ids == []` → no POST emission.
- `test_sync_create_returns_parsed_only_reads_raw_from_underscore_raw_response` — `.create()` returns a parsed Pydantic model whose `_raw_response` carries the raw `ChatCompletion`; POST extracts usage from there. Validates against Instructor's actual private-attr convention.

### 1.3 Signature semantics

- `test_signature_includes_response_model_identity` — same messages + different `response_model` (two distinct Pydantic classes) → different `llm_call_id`. Guards against silent schema-flip under one reservation.
- `test_signature_includes_model` — same messages + different `model="gpt-4o"` vs `"gpt-4o-mini"` → different `llm_call_id`.
- `test_signature_includes_tools_and_tool_choice` — different `tools`/`tool_choice` → different `llm_call_id`.
- `test_signature_diverges_across_instructor_validation_retries` — feed two successive `kwargs` dicts where `messages` differs by an injected validation-error message (mirrors Instructor's retry shape) → different `llm_call_id` → different idempotency key → each gets its own reservation.

### 1.4 Exception handling

- `test_sync_create_with_completion_failure_emits_post_failure` — inner raises `RuntimeError` → POST `outcome="FAILURE"` + re-raise.
- `test_sync_create_with_completion_cancelled_emits_post_cancelled` — inner raises a synthetic class named `CancelledError` → POST `outcome="CANCELLED"` (matches D26 / D12 / D24 pattern using `type(exc).__name__` comparison) + re-raise.

### 1.5 Async path

- `test_async_create_with_completion_full_round_trip` — async ALLOW with `pytest-asyncio`; assert `request_decision` + `emit_llm_call_post` both awaited.

### 1.6 Run context

- `test_sync_create_raises_without_active_run_context` — calling `proxy.chat.completions.create(...)` outside `run_context()` raises `RuntimeError` with the same message contract as `openai_agents.current_run_context`.

## 2. Integration tests — `sdk/python/tests/integrations/test_atomic_agents_real.py`

```python
import pytest

instructor_pkg = pytest.importorskip("instructor", minversion="1.5")
atomic_agents_pkg = pytest.importorskip("atomic_agents", minversion="1.0")
```

### 2.1 End-to-end with real Atomic Agents + Instructor

- `test_real_atomic_agents_base_agent_round_trip` — wire a `BaseAgent` whose `client=wrap_instructor_client(instructor.from_openai(OpenAI(...)))`. Provider HTTP intercepted by `pytest-httpx`. Call `agent.run({...})`. Assert one `LLM_CALL_PRE` + one paired `LLM_CALL_POST` row with `estimated_amount_atomic` matching the mocked usage.

- `test_real_atomic_agents_pydantic_output_schema_round_trip` — define a `class Answer(BaseModel): final: str`, set as `output_schema`, run `agent.run({"query": "..."})`. Assert the returned object is an `Answer` instance and the SpendGuard sidecar saw exactly one PRE/POST pair.

- `test_real_instructor_validation_retry_creates_per_retry_reservation` — configure `BaseAgentConfig(model=..., ...)` with an `output_schema` whose first response payload (mocked via `pytest-httpx`) deliberately fails Pydantic validation, and the second succeeds. Assert: two `RequestDecision` calls, two distinct `llm_call_id`s, two distinct `decision_id`s, two `LLM_CALL_POST` emissions (one per attempt). This is the load-bearing test that justifies wrapping the Instructor object (not the raw SDK).

- `test_real_atomic_agents_deny_path_zero_provider_http` — sidecar returns DENY → `agent.run(...)` raises `SpendGuardDenied`. **Critically:** assert via `pytest-httpx` request inspection that ZERO HTTP requests reached the inner OpenAI transport.

- `test_real_atomic_agents_async_round_trip` — async path using `instructor.from_openai(AsyncOpenAI(...))` + `await agent.run_async(...)`. Assert one PRE/POST pair.

### 2.2 Polyglot trace sharing

- `test_polyglot_run_context_shared_with_openai_agents` — one step uses D28 wrap, a downstream step uses `openai_agents.SpendGuardAgentsModel`, both inside the same `run_context()` → both audit rows share `run_id`.

### 2.3 Rejected-alternative regression

- `test_raw_openai_wrap_rejected_by_factory` — `wrap_instructor_client(openai.OpenAI())` raises `TypeError` whose message contains both `instructor.from_openai` and a pointer to the docs page. Guards against operator drift toward the rejected alternative documented in `design.md` §1.

## 3. Demo-mode regression

`deploy/demo/demo/run_demo.py` gains `run_agent_real_atomic_agents_mode()`:

1. Constructs a `BaseAgent` whose `client = wrap_instructor_client(instructor.from_openai(OpenAI(...)))` against a `pytest-httpx`-style mocked provider transport (or a synthetic fixture if running under docker compose where `pytest-httpx` is unavailable — the demo bootstrap installs a thin `httpx.MockTransport` substitute).
2. Configures a budget that allows the first call and denies the second (budget exhaustion).
3. Asserts:
   - First `agent.run(...)` returns a non-`None` Pydantic-parsed `output_schema` instance.
   - Second `agent.run(...)` raises `SpendGuardDenied` from the wrapper PRE boundary.
   - Postgres `audit_outbox` shows exactly 1 `LLM_CALL_PRE`+`LLM_CALL_POST` pair with `outcome='SUCCESS'` for call #1, and exactly 1 `LLM_CALL_PRE` with `decision='DENY'` for call #2 (no POST for #2).

`verify_step_atomic_agents.sql` reused.

## 4. Coverage matrix

| Surface | Unit | Integration | Demo |
|---------|------|-------------|------|
| Sync `create_with_completion()` ALLOW | ✓ | ✓ | ✓ |
| Sync `create_with_completion()` DENY (fail-closed before HTTP) | ✓ | ✓ zero-HTTP assertion | ✓ |
| Sync `create_with_completion()` FAILURE | ✓ | — | — |
| Sync `create_with_completion()` CANCELLED | ✓ | — | — |
| Sync `create()` (parsed only, raw via `_raw_response`) | ✓ | — | — |
| Async `create_with_completion()` ALLOW | ✓ | ✓ | — |
| Instructor validation-retry → per-retry reservation | ✓ (unit synth) | ✓ (real Instructor) | — |
| `__getattr__` delegation (`mode`, etc.) | ✓ ×2 | — | — |
| Polyglot trace sharing with `openai_agents` | — | ✓ | — |
| Raw-OpenAI-wrap rejection (docs guard) | ✓ | ✓ | — |

## 5. Test infrastructure

- `pytest-asyncio` (already in dev extras).
- `pytest-httpx` for ordering / zero-HTTP assertions on the deny path AND the Instructor-retry test (already used by D11/D12/D24/D26).
- New conftest `sdk/python/tests/integrations/conftest_atomic_agents.py` with:
  - `FakeInstructor` — mimics `instructor.Instructor` interface; configurable `.chat.completions.create*` returning a stub `ChatCompletion` with controllable `usage`.
  - `FakeAsyncInstructor` — async sibling.
  - `mocked_openai_transport` — `httpx.MockTransport` fixture that records every outbound request; used by both the deny-path zero-HTTP assertion and the Instructor-retry test.
- Integration tests use `pytest.importorskip("instructor", minversion="1.5")` AND `pytest.importorskip("atomic_agents", minversion="1.0")` so CI without those installed reports SKIPPED, not failure.
