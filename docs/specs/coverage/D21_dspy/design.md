# D21 — DSPy `BaseCallback` Adapter (`spendguard.integrations.dspy`)

**Status:** Spec — Tier 3, build plan `framework-coverage-build-plan-2026-06.md` §2.3.
**Owner:** AI Engineer. **Depends on:** none. **Sibling:** [`D12_litellm_sdk_shim/`](../D12_litellm_sdk_shim/design.md).

## 1. Problem

DSPy (`dspy-ai >= 2.6`) ships `dspy.LM` which routes through LiteLLM by default — D12 covers that transitively. Two paths bypass D12:

1. Custom `dspy.LM` subclasses overriding `__call__` to hit provider SDKs directly.
2. Users who have NOT installed D12 but call `dspy.Predict` / `dspy.ChainOfThought` directly.

D21 closes both via `dspy.utils.callback.BaseCallback`. `on_lm_start(call_id, instance, inputs)` fires before EVERY `dspy.LM` call regardless of routing; `on_lm_end` fires after. Both are first-class in DSPy ≥ 2.6 with no per-provider drift.

D21 is additive to D12. When both are installed, a shared `_IN_FLIGHT` contextvar prevents double-reserve: D21 reserves first; D12's wrapper short-circuits to the original. See §5.

## 2. Goals

1. Public API: `SpendGuardDSPyCallback(*, client, budget_resolver, claim_estimator=None, claim_reconciler, run_context_factory=None)`. Operator passes the instance to `dspy.configure(callbacks=[...])`. No global install.
2. `on_lm_start` reserves BEFORE the LM's provider call. Verified by ordering test with mock `dspy.LM`.
3. `on_lm_end` commits with real usage from `outputs[0].usage` (DSPy normalizes OpenAI/Anthropic/Gemini/Cohere). Exception → `outcome=FAILURE`; `asyncio.CancelledError` → `outcome=CANCELLED`.
4. Demo mode `agent_real_dspy`: `dspy.ChainOfThought("question -> answer")` end-to-end with ALLOW + DENY substeps. Stub counts hits; DENY registers zero.
5. Per-call run-context auto-generated; pluggable via `run_context_factory` to share `run_id` with adjacent LangChain / pydantic-ai contextvars.
6. PyPI extra `dspy = ["dspy-ai>=2.6"]` in `sdk/python/pyproject.toml`.
7. Docs page `docs/site/docs/integrations/dspy.md` with 2-path decision matrix (D12 transitive / D21 direct).

## 3. Non-goals

- Token-by-token streaming gating. `on_lm_end` reports end-of-call usage only.
- `on_tool_start` / `on_tool_end` gating; tool spend rolls into the parent LM reservation. Reserved for D21.1.
- `on_module_start` / `on_module_end` gating; LM-boundary gating subsumes it.
- Async DSPy callbacks. DSPy ≥ 2.6 hooks are sync — we dispatch via `asyncio.run` when no loop is running, raise `SyncInAsyncContext` when one is.
- Module-level retry-aware idempotency. DSPy's retry layer re-invokes `dspy.LM`; each invocation is a fresh reservation (parity with LangChain).

## 4. Architecture

```
user → dspy.ChainOfThought("q -> a")(question="...")
       ↓
dspy.LM("openai/gpt-4o-mini").__call__(prompt=..., messages=...)
       ↓
DSPy iterates callbacks list:
  SpendGuardDSPyCallback.on_lm_start(call_id, instance, inputs)
       ├─ resolver(instance.model) → BudgetBinding
       ├─ estimator(inputs) → projected claims
       ├─ sidecar.RequestDecision  ←── BEFORE provider HTTP
       │    ALLOW → stash (call_id → state) in module dict; continue
       │    DENY  → raise DecisionDenied
       └─ DEGRADE → raise SidecarUnavailable (fail-closed)
       ↓
dspy.LM invokes provider (LiteLLM or direct SDK)
       ↓
SpendGuardDSPyCallback.on_lm_end(call_id, outputs, exception)
       ├─ pop state for call_id
       ├─ exception None → reconciler(outputs) → real claim
       │                 → sidecar.emit_llm_call_post(SUCCESS)
       └─ exception → outcome = CANCELLED if CancelledError else FAILURE
                    → sidecar.emit_llm_call_post(outcome=...)
                    → DSPy re-raises original exception
```

Per-call state lives in `_PENDING: dict[call_id, _CallState]` keyed by DSPy's UUID. `on_lm_end` pops; a TTL sweep drops entries older than 5 min + WARN.

## 5. Key decisions

- **Composition, not monkey-patch.** Subclasses `BaseCallback` (isinstance required); holds the SpendGuard client by composition. Never patches `dspy.LM`.
- **Per-call state via `call_id`.** DSPy issues stable UUIDs per LM invocation; key our dict on it. No threading / contextvar ownership across the start/end pair.
- **Fail-closed default.** DEGRADE raises `SidecarUnavailable`. `SPENDGUARD_DSPY_FAIL_OPEN=1` permits otherwise (parity with `litellm.py`).
- **D12 coexistence via shared contextvar.** `on_lm_start` sets `spendguard._litellm_shim._IN_FLIGHT=True`. D12's wrapper short-circuits; D21 owns the reserve. If D12 is absent, no-op.
- **Sync callback → async dispatch.** Outside a loop: `asyncio.run`. Inside a running loop: raise `SyncInAsyncContext`. Never bridge silently.
- **No DSPy global install.** Operator wires `dspy.configure(callbacks=[...])` explicitly so the callback is observable in `dspy.settings`.

## 6. Slice plan

| Slice | Title | Size |
|-------|-------|------|
| `COV_D21_S1_skeleton_extras` | Module skeleton + `[dspy]` extra + `_PENDING` registry + run-context plumbing | S |
| `COV_D21_S2_callback_class` | `SpendGuardDSPyCallback` `on_lm_start` + `on_lm_end` + reconciler / commit / release wiring | M |
| `COV_D21_S3_tests_demo` | Unit tests (mock dspy.LM) + integration test (real dspy + pytest-httpx) + `agent_real_dspy` demo mode | M |
| `COV_D21_S4_docs_readme` | `dspy.md` docs page + 2-path decision matrix + README adapter row | S |

4 slices, S/M only, ~700 LOC (250 impl + 350 test + 100 docs).

## 7. Interfaces

```python
class SpendGuardDSPyCallback(BaseCallback):
    class SyncInAsyncContext(SpendGuardConfigError): ...

    def __init__(self, *, client: SpendGuardClient,
                 budget_resolver: BudgetResolver,
                 claim_estimator: ClaimEstimator | None = None,
                 claim_reconciler: ClaimReconciler,
                 run_context_factory: Callable[[], RunContext] | None = None,
                 ) -> None: ...

    def on_lm_start(self, call_id: str, instance: Any,
                    inputs: dict[str, Any]) -> None: ...
    def on_lm_end(self, call_id: str, outputs: Any,
                  exception: BaseException | None) -> None: ...
```

Full code in `implementation.md` §2.

## 8. Open questions (locked)

1. **Multi-callback ordering:** locked — DSPy invokes callbacks in list order; place `SpendGuardDSPyCallback` FIRST so reserve precedes user callbacks. Documented in `dspy.md`.
2. **Retry loops re-firing callbacks:** locked — each retry IS a new `call_id` and a new reservation. Idempotency via `(run_id, call_id, attempt)` ID derivation.
3. **`outputs` shape:** locked — DSPy normalizes `LMResponse.usage = {"prompt_tokens", "completion_tokens", "total_tokens"}` across providers. Subclasses without `usage` fall back to estimator claim + WARN.
