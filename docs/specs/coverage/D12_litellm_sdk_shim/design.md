# D12 — LiteLLM SDK Monkey-Patch Shim (`spendguard-litellm-shim`)

**Status:** Spec — Tier 2, build plan `framework-coverage-build-plan-2026-06.md` §2.2.
**Owner:** Backend Architect. **Depends on:** D11. **Sibling:** [`D11_litellm_proxy_plugin/`](../D11_litellm_proxy_plugin/design.md).

## 1. Problem

LiteLLM Issue #8842 (open 14 months): `async_pre_call_hook` fires only on the proxy path. Direct `litellm.acompletion()` callers bypass every gate. `CustomLogger` post-call hooks fire AFTER provider HTTP, cannot deny.

`SpendGuardDirectAcompletion` (litellm.py:896-1126) closes this for callers that explicitly instantiate the wrapper. That is NOT the shape of CrewAI, DSPy, SmolAgents, Strands, BeeAI, AutoGen, Atomic Agents — they import `litellm` and call `acompletion(...)` directly. SpendGuard cannot retrofit constructors into upstream framework code.

D12 ships a monkey-patch shim. `spendguard_litellm_shim.install()` globally replaces LiteLLM SDK entry points (and `Router` equivalents) with wrappers that reserve before / commit after / release on exception. The caller's `await litellm.acompletion(...)` is unchanged in signature, return, exceptions.

D12 is additive to `SpendGuardLiteLLMCallback` (proxy) and `SpendGuardDirectAcompletion` (explicit wrapper). D12 serves as transitive coverage for D20-D28 — each routes through LiteLLM, so `install()` alone enforces budgets.

## 2. Goals

1. Public API: `install(*, client, budget_resolver, claim_estimator=None, claim_reconciler, patch_router=True, patch_sync=True)` + `uninstall()` + `is_installed()`. Idempotent: same config = no-op; different config = `SpendGuardShimAlreadyInstalled`.
2. Patch surfaces (all reuse `SpendGuardDirectAcompletion` core via composition): `litellm.acompletion`, `litellm.completion`, `litellm.atext_completion`, `litellm.text_completion`, `litellm.Router.acompletion`, `litellm.Router.completion`.
3. Reserve fires BEFORE provider HTTP. Verified by `pytest-httpx` wire-level ordering.
4. Commit reads `response.usage.completion_tokens` (OpenAI shape; LiteLLM normalizes Anthropic/Bedrock/Gemini).
5. Exception → `emit_llm_call_post(outcome=FAILURE)` + re-raise. `CancelledError` → `outcome=CANCELLED`.
6. Demo modes: `litellm_sdk_real` (ALLOW+COMMIT) + `litellm_sdk_deny` (DENY zero provider hits).
7. Transitive proof: `test_crewai_via_shim.py` — `crew.kickoff()` triggers SpendGuard reserves with no CrewAI code changes. Load-bearing.
8. Docs `docs/site/docs/integrations/litellm-sdk-shim.md` with 3-path decision matrix (D02 egress / D11 guardrail / D12 shim).

## 3. Non-goals

- Token-by-token streaming gating. End-of-stream commit only.
- Sync `completion()` from inside a running loop. Raises `SpendGuardShimSyncInAsyncContext`.
- `litellm.embedding` / `aembedding` / `aimage_generation` / `atranscription`. Reserved for D12.1.
- LiteLLM Issue #8842 upstream fix. Separate workstream.
- Auto-install via import hook. Operator MUST call `install()` explicitly so monkey-patching is observable in stack traces and reversible.

## 4. Architecture

```
user → await litellm.acompletion(model=..., messages=...)  [after install()]
       ↓
shim wrapper:
  if _IN_FLIGHT.get(): return await ORIGINAL(**kwargs)   [recursion guard]
  token = _IN_FLIGHT.set(True)
  try: return await _DirectCore(_original_acompletion=ORIGINAL, **kwargs)
       ├─ resolver→binding · estimator→claim
       ├─ sidecar.RequestDecision  ←── BEFORE provider HTTP
       │    ALLOW=continue · DENY=raise · DEGRADE=raise (fail-closed)
       ├─ ORIGINAL(**kwargs) → ModelResponse  [provider HTTP]
       ├─ reconciler→real claim
       └─ sidecar.emit_llm_call_post(SUCCESS)
         exception → emit_llm_call_post(FAILURE|CANCELLED) + re-raise
  finally: _IN_FLIGHT.reset(token)
```

`_DirectCore` reuses `SpendGuardDirectAcompletion` verbatim (composition) but receives the saved original via a new `_original_acompletion` kwarg. Without it, the core's internal `await litellm.acompletion(...)` (litellm.py:1067) re-enters the wrapper → infinite recursion. The contextvar guard also catches it, but explicit injection is cleaner and observable in stack traces.

## 5. Key decisions

- **Composition over inheritance.** Shim holds `_DirectCore` instances; never subclasses LiteLLM types.
- **Idempotent install** via stable `config_signature` hash. Same config = no-op. Different config = `SpendGuardShimAlreadyInstalled`.
- **uninstall()** walks `state.originals: list[(owner, attr, original)]` in reverse.
- **Sync wrapper raises in async context.** `litellm.completion()` inside a running loop would deadlock under `asyncio.run`; shim detects via `asyncio.get_running_loop()` + `RuntimeError` and raises `SpendGuardShimSyncInAsyncContext`. Outside a loop, bridges via `asyncio.run(_async_dispatch(...))`.
- **Router patching at class level.** `install()` records original `Router.acompletion`, walks `Router.__subclasses__()` and re-patches subclasses that overrode the method. Post-install subclasses inherit via MRO.
- **Recursion guard via `contextvars.ContextVar[bool]`** — per-task, never thread-local. Token-based set+reset.
- **Inherits `SPENDGUARD_LITELLM_FAIL_OPEN`** from `SpendGuardDirectAcompletion`. No new env vars.

## 6. Slice plan

| Slice | Title | Size |
|-------|-------|------|
| `COV_D12_S1_shim_skeleton` | Module skeleton + install/uninstall state machine + recursion guard | M |
| `COV_D12_S2_patch_acompletion` | Patch `acompletion` + `atext_completion`; reuse `_DirectCore` via `_original_acompletion` | M |
| `COV_D12_S3_patch_sync` | Patch `completion` + `text_completion`; async-context guard | S |
| `COV_D12_S4_patch_router` | Patch `Router.acompletion` + subclass walk | S |
| `COV_D12_S5_unit_tests_mock_litellm` | 22+ unit tests + pytest-httpx ordering assertion | M |
| `COV_D12_S6_integration_real_litellm` | 6 integration tests with real litellm + 3 CrewAI/DSPy transitive smokes | M |
| `COV_D12_S7_demo_modes_and_docs` | `litellm_sdk_real` + `litellm_sdk_deny` + Makefile + verify SQL + docs | M |

7 slices, S/M only, ~1700 LOC (~600 impl + 800 test + 300 docs/yaml). Mirrors D11's 7-slice rhythm.

## 7. Interfaces

```python
def install(*, client, budget_resolver, claim_estimator=None,
            claim_reconciler, patch_router=True, patch_sync=True) -> None
def uninstall() -> None
def is_installed() -> bool
class SpendGuardShimAlreadyInstalled(SpendGuardConfigError)
class SpendGuardShimSyncInAsyncContext(SpendGuardConfigError)
```

Full operator code sample in `implementation.md` §2.

## 8. Open questions (locked)

1. **Router subclasses overriding `acompletion`:** locked — install patches both `Router` AND walks live `Router.__subclasses__()`. Post-install subclasses inherit via MRO. Documented as limitation.
2. **Streaming `usage` frame:** locked — wrap returned `AsyncIterator`; commit at `StopAsyncIteration` OR `aclose()`. Estimator-fallback path mirrors `SpendGuardLiteLLMCallback._async_log_success_streaming`.
3. **`completion()` sync inside async test context:** locked — raise `SpendGuardShimSyncInAsyncContext` with hint pointing at `acompletion`. Never silently bridge (would deadlock).
