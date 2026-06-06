# D20 — AWS Strands `HookProvider.before_invocation` Adapter

**Status:** Spec — Tier 3, build plan `framework-coverage-build-plan-2026-06.md` §2.3.
**Owner:** AI Engineer. **Depends on:** none directly; D12 transitively covers the LiteLLM-routed sub-path. **Sibling:** [`D19_google_adk/`](../D19_google_adk/design.md).

## 1. Problem

AWS Strands Agents SDK (GA 2026-04) is Amazon's first-party Python+TS agent framework with a best-in-class typed-event-bus hook system: `HookProvider` exposes `before_invocation`, `after_invocation`, `before_tool`, `after_tool`, `on_message`. Strands is model-agnostic — Bedrock (default), Anthropic SDK, OpenAI SDK, Gemini, Ollama, or LiteLLM.

D12's LiteLLM shim transitively covers Strands only when LiteLLM is the backend. Native Bedrock / OpenAI / Anthropic paths bypass LiteLLM entirely. Strands' typical AWS-shop path is `BedrockModel(...)` — pure Bedrock InvokeModel HTTP. That population is uncovered today.

D20 ships `SpendGuardHookProvider` registered as `hooks=[SpendGuardHookProvider(...)]`. `before_invocation` reserves; `after_invocation` commits. Coverage is enforced **at the agent-runtime boundary**, not the model boundary — works for every backend with one provider instance.

## 2. Goals

1. Public surface: `class SpendGuardHookProvider(HookProvider)` with `register_hooks(registry)` per Strands' contract. Constructor: `(client, budget_id, window_instance_id, unit, pricing, claim_estimator=None, claim_reconciler, fail_closed=True)`.
2. `before_invocation(event)` resolves estimator claim from `event.invocation` (model + messages + tools) and calls `client.request_decision(trigger="LLM_CALL_PRE", ...)`. DENY raises `DecisionDenied`; runtime surfaces as `HookExecutionError`.
3. `after_invocation(event)` reads `event.result.usage` and calls `client.emit_llm_call_post(outcome=SUCCESS, ...)`. Failure path (`event.exception is not None`) emits `FAILURE` or `CANCELLED`.
4. Per-invocation stash keyed by `event.invocation.invocation_id` so before / after pair under interleaved `asyncio.gather`.
5. Demo modes `agent_real_strands` (ALLOW) + `agent_real_strands_deny` (zero provider HTTP on DENY).
6. Coverage proof against **three** backends (Bedrock + OpenAI + LiteLLM) via recorded fixtures. Bedrock is load-bearing.
7. Docs `docs/site/docs/integrations/aws-strands.md` with model-backend coverage matrix.

## 3. Non-goals

- `before_tool` / `after_tool` per-tool budget gating. Tool cost bundled into parent invocation. Deferred to D20.1.
- `on_message` streaming token gating. End-of-invocation commit only.
- Strands' TS SDK. Covered separately under D05 + D08 family.
- Pinning beyond `aws-strands-agents>=1.0,<2`.
- Auto-install via `default_hooks=`. Operator MUST construct the provider explicitly.

## 4. Architecture

```
agent = Agent(model=BedrockModel(...), hooks=[SpendGuardHookProvider(...)])
await agent.invoke_async(prompt="...")
  ↓ Strands runtime fires BeforeInvocationEvent(invocation_id, model, messages, tools)
  ↓ SpendGuardHookProvider.before_invocation
    ├─ estimator(invocation) → BudgetClaim
    ├─ sidecar.RequestDecision  ←── BEFORE Bedrock/OpenAI/LiteLLM HTTP
    │    ALLOW=stash by invocation_id · DENY=raise · DEGRADE=raise (fail-closed)
    └─ return
  ↓ Strands calls model.invoke(...) → provider HTTP
  ↓ AfterInvocationEvent(invocation_id, result, exception)
  ↓ SpendGuardHookProvider.after_invocation
    ├─ pop stash by invocation_id
    ├─ exception != None → emit_llm_call_post(FAILURE|CANCELLED), do NOT mask
    └─ else → reconciler(invocation, result) → emit_llm_call_post(SUCCESS)
```

Stash lives on `self._stash: dict[str, _PendingInvocation]`. Strands stamps `invocation_id` per GA contract — pinned via runtime assertion. No mutation of `event` (bus contract is read-only).

## 5. Key decisions

- **Provider, not Model wrap.** Strands' Model abstraction is plural (Bedrock/OpenAI/Anthropic/Gemini/Ollama/LiteLLM); wrapping each is unscalable. Hook provider sits above all backends.
- **Stash keyed by `invocation_id`**, NOT `(run_id, message_count)`. Strands fans out parallel tool calls; run-scoped keys collide.
- **No `_RUN_CONTEXT` required.** Strands' event carries context end-to-end. Optional `run_context()` provided for parity with LangChain/LiteLLM users mixing frameworks.
- **DENY-raise type.** Raise `DecisionDenied` directly; Strands wraps to `HookExecutionError`; caller catches via `__cause__` chain.
- **Estimator dispatch by model backend.** Default `strands_default_claim_estimator(model_id=event.invocation.model.model_id)` dispatches per-vendor (Bedrock IDs / OpenAI names / LiteLLM names). Operator override wins.
- **Backend coverage matrix asserted in CI.** Three integration tests assert identical adapter behaviour on `BedrockModel` + `OpenAIModel` + `LiteLLMModel`. Adding a backend = adding a 4th row.
- **Reconciler receives `event.result` directly.** Strands normalizes usage across providers into `result.usage` (GA contract). Reconciler reads `input_tokens + output_tokens`.

## 6. Slice plan

| Slice | Title | Size |
|-------|-------|------|
| `COV_D20_S1_module_skeleton` | Module skeleton + `[strands]` extra + dataclasses | S |
| `COV_D20_S2_hook_provider_reserve` | `before_invocation` reserve + DENY/DEGRADE fail-closed + stash | M |
| `COV_D20_S3_hook_provider_commit` | `after_invocation` commit/release + exception classification | M |
| `COV_D20_S4_multi_backend_tests` | Unit + recorded-fixture tests for Bedrock + OpenAI + LiteLLM | M |
| `COV_D20_S5_demo_and_docs` | `agent_real_strands` + `_deny` demo modes + verify SQL + docs | M |

5 slices, S/M only, ~1500 LOC (~450 impl + 750 test + 300 docs/yaml).

## 7. Interfaces

```python
class SpendGuardHookProvider(HookProvider):
    def __init__(self, *, client, budget_id, window_instance_id, unit, pricing,
                 claim_estimator=None, claim_reconciler, fail_closed=True): ...
    def register_hooks(self, registry: HookRegistry) -> None: ...
    async def before_invocation(self, event: BeforeInvocationEvent) -> None: ...
    async def after_invocation(self, event: AfterInvocationEvent) -> None: ...

@dataclass(frozen=True, slots=True)
class StrandsRunContext:
    run_id: str
    step_id: str | None = None
```

Full code in `implementation.md` §2.

## 8. Open questions (locked)

1. **`invocation_id` stability across retries:** locked — fresh ID per attempt (verified against 1.0 source). Each gets its own reserve+commit pair. Matches LiteLLM ADR-002.
2. **Tool-call cost during invocation:** locked — bundled into parent estimator+reconciler. Per-tool is D20.1.
3. **Streaming `on_message`:** locked — adapter ignores; commit only at `after_invocation`. Estimator-fallback if `result.usage` is None.
4. **Multi-provider in one Agent:** Strands allows mid-run `agent.model = new_model`. Hook sees every invocation; stash keyed by `invocation_id` handles it. Documented as supported.
5. **`HookExecutionError` wrapping:** locked — caller catches `DecisionDenied` directly via `exc.__cause__` chain.
