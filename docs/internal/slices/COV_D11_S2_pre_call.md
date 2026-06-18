# COV_D11_S2 — D11 LiteLLM proxy plugin: pre_call hook wiring

> **Deliverable**: D11 LiteLLM `async_pre_call_hook` proxy guardrail plugin
> **Slice**: 2 of 7 (S)
> **Spec set**: [`docs/specs/coverage/D11_litellm_proxy_plugin/`](../../specs/coverage/D11_litellm_proxy_plugin/)

## Scope

Wire the real `async_pre_call_hook` body. Replace the SLICE 1 `NotImplementedError` with a delegation to `self._delegate.async_pre_call_hook(...)` (the existing `_LoopBoundCallback` already implements the reserve path against the SpendGuard sidecar). On DENY: raise `litellm.exceptions.BadRequestError` (or the LiteLLM-defined guardrail-deny exception) so the proxy short-circuits the upstream call.

Concretely:
- `sdk/python/src/spendguard/integrations/litellm_guardrail.py`:
  - `async def async_pre_call_hook(self, user_api_key_dict, cache, data, call_type) -> dict | None`:
    - Extract messages + model + metadata from `data` (LiteLLM proxy passes the OpenAI-shape body)
    - Call `await self._delegate.async_pre_call_hook(...)` with the marshalled fields
    - On `SpendGuardDeniedError` from delegate: re-raise as `litellm.exceptions.BadRequestError(message="SpendGuard budget denied", code=429, model=data.get("model"))` (or the LiteLLM 1.55+ specific GuardrailRaiseException if available)
    - On `SpendGuardDegradeError` (DEGRADE policy): return modified `data` (e.g., switched model per ALLOW_WITH_CAPS) — pass-through delegate's mutation
    - On ALLOW: return `data` unchanged
  - Preserve the SLICE 1 NotImplementedError on the other two hooks (commit/release land in SLICE 3)
- `sdk/python/tests/integrations/test_litellm_guardrail_pre_call.py`:
  - Mock `_LoopBoundCallback._delegate_reserve` returns ALLOW → hook returns `data` unchanged
  - Mock delegate raises `SpendGuardDeniedError` → hook re-raises BadRequestError
  - Mock delegate raises `SpendGuardDegradeError(replacement_model="gpt-4o-mini")` → hook returns data with model swapped
  - Mock delegate fails with a non-SpendGuard exception → hook fail-closes (re-raises original; logged via tracing)

## Files touched

| File | Why |
|------|-----|
| `sdk/python/src/spendguard/integrations/litellm_guardrail.py` | Wire async_pre_call_hook body |
| `sdk/python/tests/integrations/test_litellm_guardrail_pre_call.py` | New tests for the 4 outcomes (ALLOW / DENY / DEGRADE / unknown error) |

## Test/verification plan

1. New tests pass: ≥ 8 unit tests covering ALLOW, DENY → BadRequestError, DEGRADE → data mutation, unknown error → re-raise, and edge cases (missing model, missing messages, empty messages).
2. Existing SLICE 1 tests (16) STILL pass.
3. Full `pytest sdk/python/tests/` shows zero regression.
4. `pytest.importorskip` for `litellm.exceptions.BadRequestError` so environments without LiteLLM cleanly skip.

## Anti-scope

- No `async_post_call_success_hook` body — SLICE 3.
- No `async_post_call_failure_hook` body — SLICE 3.
- No env-driven default factory — SLICE 4.
- No proxy_config.yaml entry — SLICE 5.
- No demo mode — SLICE 6.
- No docs page — SLICE 7.

## Backlinks

- Spec set: [`design.md`](../../specs/coverage/D11_litellm_proxy_plugin/design.md) §6 slice 2 row
- SLICE 1: [`COV_D11_S1_guardrail_class.md`](COV_D11_S1_guardrail_class.md)
