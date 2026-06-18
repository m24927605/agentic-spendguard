# COV_D11_S1 — D11 LiteLLM proxy plugin: guardrail class skeleton

> **Deliverable**: D11 LiteLLM `async_pre_call_hook` proxy guardrail plugin
> **Slice**: 1 of 7 (S)
> **Spec set**: [`docs/specs/coverage/D11_litellm_proxy_plugin/`](../../specs/coverage/D11_litellm_proxy_plugin/)

## Scope

Ship the bare `SpendGuardGuardrail(CustomGuardrail)` class skeleton in a new `spendguard.integrations.litellm_guardrail` module. Composition (NOT inheritance) wires it to the existing `_LoopBoundCallback` in `litellm.py` so the legacy callback path stays intact and all identity/idempotency code remains single-sourced.

Concretely:
- New module: `sdk/python/src/spendguard/integrations/litellm_guardrail.py`.
- Class `SpendGuardGuardrail` extends `litellm.integrations.custom_guardrail.CustomGuardrail`.
- `__init__(self, *, guardrail_name="spendguard", **kwargs)`: store name, hold a `_LoopBoundCallback` instance (composition) — but do NOT initialize the loop-bound state yet (that comes in S2). For now, the three hook methods exist with `raise NotImplementedError("wired in COV_D11_S2/S3")` bodies.
- `async def async_pre_call_hook(self, user_api_key_dict, cache, data, call_type)`: stub.
- `async def async_post_call_success_hook(self, data, user_api_key_dict, response)`: stub.
- `async def async_post_call_failure_hook(self, request_data, original_exception, user_api_key_dict)`: stub.
- Export the class in `sdk/python/src/spendguard/integrations/__init__.py` (additive — must not break existing exports).
- New test file: `sdk/python/tests/integrations/test_litellm_guardrail_skeleton.py` that:
  - Imports the class.
  - Instantiates it with `guardrail_name="test"`.
  - Verifies the three hook methods exist + are coroutines (don't actually call them — they raise).
  - Verifies the existing `_LoopBoundCallback` is held internally.

## Files touched

| File | Why |
|------|-----|
| `sdk/python/src/spendguard/integrations/litellm_guardrail.py` | New module |
| `sdk/python/src/spendguard/integrations/__init__.py` | Additive export |
| `sdk/python/tests/integrations/test_litellm_guardrail_skeleton.py` | Skeleton sanity test |

## Test/verification plan

1. `pytest sdk/python/tests/integrations/test_litellm_guardrail_skeleton.py -v` passes (4-5 tests).
2. `pytest sdk/python/tests/` — all existing tests STILL pass. Zero regressions.
3. `python -c "from spendguard.integrations import SpendGuardGuardrail"` succeeds.
4. `python -c "from spendguard.integrations.litellm import SpendGuardLiteLLMCallback"` STILL succeeds (legacy callback untouched).

## Anti-scope

- No real pre-call wiring — SLICE 2.
- No commit/release wiring — SLICE 3.
- No env-driven default factory — SLICE 4.
- No `proxy_config.yaml` snippet or PyPI extras — SLICE 5.
- No demo mode — SLICE 6.
- No docs page — SLICE 7.

## Backlinks

- Spec set: [`design.md`](../../specs/coverage/D11_litellm_proxy_plugin/design.md) §6 slice plan, §7 interfaces
- Build plan: [`framework-coverage-build-plan-2026-06.md`](../../strategy/framework-coverage-build-plan-2026-06.md) §1.5
- Review standards: [`review-standards.md`](../../specs/coverage/D11_litellm_proxy_plugin/review-standards.md)
