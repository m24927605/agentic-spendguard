# COV_D11_S3 — D11 LiteLLM proxy plugin: commit/release hook wiring

> **Deliverable**: D11 LiteLLM `async_pre_call_hook` proxy guardrail plugin
> **Slice**: 3 of 7 (S)
> **Spec set**: [`docs/specs/coverage/D11_litellm_proxy_plugin/`](../../specs/coverage/D11_litellm_proxy_plugin/)

## Scope

Wire the two remaining hook stubs: `async_post_call_success_hook` (commit) and `async_post_call_failure_hook` (release). Replace SLICE 1's `NotImplementedError("COV_D11_S3")` bodies with pure 3-LOC delegations to the underlying `_LoopBoundCallback`'s commit/release paths.

Concretely:
- `sdk/python/src/spendguard/integrations/litellm_guardrail.py`:
  - `async def async_post_call_success_hook(self, data, user_api_key_dict, response)`:
    - Translate LiteLLM's `data + response` shape into the kwargs `_LoopBoundCallback.async_log_success_event` expects (per implementation.md §2.5 + litellm.py:478+)
    - Specifically: populate `kwargs["litellm_call_id"]` from `data["litellm_call_id"]` so `_get_stash` finds the SLICE 2 reserve stash
    - `await self._delegate.async_log_success_event(kwargs, response, start_time, end_time)`
    - Return None (LiteLLM expects None from success hook)
  - `async def async_post_call_failure_hook(self, request_data, original_exception, user_api_key_dict, traceback_str=None)`:
    - Translate LiteLLM's failure shape into delegate's `async_log_failure_event` expected kwargs
    - Populate `kwargs["litellm_call_id"]` similarly
    - `await self._delegate.async_log_failure_event(kwargs, original_exception, start_time, end_time)`
    - Re-raise the original exception (LiteLLM expects failure hooks to propagate)
- `sdk/python/tests/integrations/test_litellm_guardrail_post_call.py` — NEW:
  - ≥ 8 tests: success commit path, failure release path, kwargs translation, missing litellm_call_id (fail-closed warning), unknown exception type in failure, timing fields propagation (start_time/end_time)
- Update `test_litellm_guardrail_skeleton.py::test_hook_methods_raise_not_implemented` parametrize — remove both post-call rows (all 3 hooks now wired)

## Files touched

| File | Why |
|------|-----|
| `sdk/python/src/spendguard/integrations/litellm_guardrail.py` | Wire success + failure hook bodies |
| `sdk/python/tests/integrations/test_litellm_guardrail_post_call.py` | New tests for the 2 paths + edge cases |
| `sdk/python/tests/integrations/test_litellm_guardrail_skeleton.py` | Remove parametrize rows for now-wired hooks |

## Test/verification plan

1. New tests pass: ≥ 8 covering success commit, failure release, kwargs translation, edge cases.
2. SLICE 1 tests (15) + SLICE 2 tests (15) still pass.
3. Full pytest sdk/python/tests/ = 873+ ~8 new = 881+ passing.
4. test_hook_methods_raise_not_implemented now has empty parametrize (or skipped) since all 3 hooks are wired.

## Anti-scope

- No env-driven default factory — SLICE 4.
- No proxy_config.yaml entry — SLICE 5.
- No demo mode — SLICE 6.
- No docs page — SLICE 7.

## Backlinks

- Spec set: [`design.md`](../../specs/coverage/D11_litellm_proxy_plugin/design.md) §6 slice 3 row, §7 interfaces
- review-standards: §3.1 commit/release Blocker checks; §3.2 kwargs translation
- SLICE 1: [`COV_D11_S1_guardrail_class.md`](COV_D11_S1_guardrail_class.md)
- SLICE 2: [`COV_D11_S2_pre_call.md`](COV_D11_S2_pre_call.md)
