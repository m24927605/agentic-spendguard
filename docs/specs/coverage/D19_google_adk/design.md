# D19 — Google ADK `before_model_callback` Adapter

**Status:** Spec — Tier 3, build plan `framework-coverage-build-plan-2026-06.md` §2.3.
**Owner sub-agent:** AI Engineer.
**Closest priors:** [`integrations/langchain.py`](../../../../sdk/python/src/spendguard/integrations/langchain.py) (callback-as-class, RunContext, claim estimator dispatch); [`integrations/openai_agents.py`](../../../../sdk/python/src/spendguard/integrations/openai_agents.py) (PRE/POST symmetry, DENY short-circuit).

## 1. Problem

Google's Agent Development Kit (`google-adk` ≥ 1.0) exposes `LlmAgent` with `before_model_callback(callback_context, llm_request)` and `after_model_callback(callback_context, llm_response)`, called per LLM turn. Contract: a falsy return from `before_model_callback` continues; a non-None `LlmResponse` short-circuits. `after_model_callback` runs unconditionally after the model returns or after a short-circuit.

SpendGuard has no ADK adapter today. ADK users either route through the egress proxy (works for Gemini + LiteLlm-wrapped OpenAI but loses run/step semantics) or hand-roll a callback. ADK is the canonical-Google ecosystem entry point; the gap blocks the only first-party Google path.

## 2. Goals

1. New `spendguard.integrations.adk` module exporting `SpendGuardAdkCallback` — one instance usable for **both** callback slots.
2. PRE: `request_decision(trigger="LLM_CALL_PRE")`; on `DecisionDenied` return a synthetic `LlmResponse(error_code="SPENDGUARD_DENY", error_message=…)` (the ADK short-circuit channel) — do not raise.
3. POST: `emit_llm_call_post(outcome="SUCCESS")` extracting `usage_metadata` (Gemini: `total_token_count` or `prompt+candidates`; LiteLlm-wrapped: OpenAI `total_tokens`). On detected DENY, call `release_reservation` instead.
4. Vendor coverage: works against (a) `LlmAgent(model="gemini-2.0-flash")` direct, (b) Vertex-backed Gemini, (c) `LlmAgent(model=LiteLlm("openai/gpt-4o-mini"))`. Usage extraction is by response shape, not by model-string parsing.
5. New extras row `spendguard-sdk[adk]` requiring `google-adk>=1.0`. Import error names the install command.
6. `DEMO_MODE=agent_real_adk`: an `LlmAgent` end-to-end with `GOOGLE_API_KEY`, proving ALLOW commits and DENY short-circuits **before** the Gemini HTTP call.

## 3. Non-goals

- **TypeScript / Go / Java / Kotlin ADK ports.** Deferred (D19.5 covers the TS port).
- **Streaming intra-turn gating.** ADK `run_live` commits at turn boundary; we follow it (parity with LangChain / openai-agents priors).
- **`before_tool_callback` / `after_tool_callback`.** Tool calls don't drive spend directly; gating sits at the model boundary.
- **Replacing the egress proxy path.** D19 is additive.

## 4. Architecture

```
Runner.run_async → LlmAgent step
  → before_model_callback(ctx, llm_request)
      → SpendGuardAdkCallback._before
          → client.request_decision(LLM_CALL_PRE, claims=…)
          → ALLOW: store reservation_id in ctx.state, return None
          → DENY:  return LlmResponse(error_code="SPENDGUARD_DENY", …)
  → (if ALLOW) inner model.generate_content_async(…)
  → after_model_callback(ctx, llm_response)
      → SpendGuardAdkCallback._after
          → ALLOW path → emit_llm_call_post(SUCCESS, total_tokens=…)
          → DENY  path → release_reservation
          → return None
```

## 5. Key decisions

- **Single class, two callable slots.** `SpendGuardAdkCallback.__call__` dispatches by argument shape (PRE = `(ctx, LlmRequest)`; POST = `(ctx, LlmResponse)`). Operators register the same instance to both slots. Matches ADK idiom and keeps state co-located.
- **Reservation handoff via `callback_context.state["spendguard.reservation_id"]`.** ADK's `CallbackContext.state` is the documented per-invocation dict; avoids contextvar churn under ADK's task-per-turn model.
- **DENY returns `LlmResponse`, never raises.** Raising propagates as ADK runtime error; the synthetic response is the documented short-circuit channel and lets the user's own `after_model_callback` (if chained) still observe the deny path.
- **`run_id` defaults to `callback_context.invocation_id`** (ADK assigns one UUID per `Runner.run_async`). Override via `SpendGuardAdkCallback(run_id_fn=…)` for cross-framework correlation.
- **Usage extraction by shape, not string.** Try Gemini `usage_metadata.total_token_count`, then sum `prompt_token_count + candidates_token_count`, then OpenAI-shape `usage_metadata.total_tokens`; on miss, fall back to estimated value (commit still fires).
- **Default claim estimator** reuses `_default_estimator` dispatched off `llm_request.model` (e.g. `"gemini-2.0-flash"` → Gemini family; `LiteLlm("openai/gpt-4o-mini")` via prefix strip → OpenAI family). Selection logic mirrors the LangChain prior.

## 6. Slice plan

| Slice | Title | Size |
|-------|-------|------|
| `COV_D19_S1_module_skeleton` | `spendguard.integrations.adk` module + `[adk]` extra + import-error guard | S |
| `COV_D19_S2_callback_class` | `SpendGuardAdkCallback` class + `__call__` arity dispatch + state handoff | M |
| `COV_D19_S3_pre_post_wiring` | `_before` reserve + DENY short-circuit; `_after` commit / release; usage-by-shape extraction | M |
| `COV_D19_S4_tests` | Unit (mock ADK types) + integration (recorded Gemini fixtures + LiteLlm fixture) | M |
| `COV_D19_S5_demo_and_docs` | `DEMO_MODE=agent_real_adk` Makefile + driver + `docs/site/docs/integrations/adk.md` | M |

5 slices, all S/M, ~1100 LOC (~500 impl + 450 test + 150 docs/yaml). No proto changes.

## 7. Interfaces

```python
class SpendGuardAdkCallback:
    def __init__(
        self,
        *,
        client: SpendGuardClient,
        budget_id: str,
        window_instance_id: str,
        unit: common_pb2.UnitRef,
        pricing: common_pb2.PricingFreeze,
        claim_estimator: ClaimEstimator | None = None,
        run_id_fn: Callable[[CallbackContext], str] | None = None,
    ) -> None: ...

    async def __call__(self, callback_context, llm_request_or_response):
        ...  # arity / type dispatch → _before or _after
```

ADK registration:

```python
cb = SpendGuardAdkCallback(client=..., budget_id=..., window_instance_id=..., unit=..., pricing=...)
agent = LlmAgent(
    model="gemini-2.0-flash",
    before_model_callback=cb,
    after_model_callback=cb,
)
```

## 8. Open questions (locked at spec write)

1. **ADK 1.x callback signature stability.** `(callback_context, llm_request)` is stable across `google-adk` 1.0–1.4 sources. Locked: floor `google-adk>=1.0`.
2. **Sync vs async callbacks.** ADK accepts both. We ship `async def __call__` because `client.request_decision` is async; sync ADK callers get a clear error from ADK.
3. **`LlmResponse(error_code=…)` as deny channel.** Verified against ADK source — `error_code` + `error_message` cause ADK to skip the model invocation and treat the turn as terminal error. Locked.
