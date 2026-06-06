# D25 — SmolAgents `Model.generate` Wrap + `step_callbacks` Informational Adapter

**Status:** Spec — Tier 3, build plan `framework-coverage-build-plan-2026-06.md` §2.3.
**Owner:** AI Engineer. **Depends on:** D12 (LiteLLM SDK shim — transitive coverage for `LiteLLMModel`).
**Sibling reference:** [`D24_autogen_ag2`](../D24_autogen_ag2/) (Model subclass wrap pattern).
**Closest analog:** `spendguard.integrations.openai_agents` — `Model` subclass wrap with PRE/POST gating.

## 1. Problem

SmolAgents (HuggingFace, Apache-2.0, ~15k stars) exposes a pluggable `smolagents.Model` ABC. Vendor subclasses: `InferenceClientModel` (HF Inference API), `LiteLLMModel` (LiteLLM SDK), `OpenAIServerModel` (any OpenAI-compatible endpoint, including vLLM / Ollama), `TransformersModel` (in-process HF transformers). Every `CodeAgent` / `ToolCallingAgent` invokes the model through `Model.generate(messages, stop_sequences=None, response_format=None, tools_to_call_from=None, **kwargs) -> ChatMessage`. Legacy `smolagents<1.5` routes through `Model.__call__`, which `generate` aliases at `>=1.5`.

`MultiStepAgent` also accepts `step_callbacks: list[Callable[[ActionStep | PlanningStep], Any]]`. They fire **after** each step — they cannot deny a pending LLM call and they cannot reserve before provider HTTP. Useful for telemetry mirroring, not gating.

Two coverage paths:

1. **Transitive via D12.** `LiteLLMModel` users get the D12 monkey-patch shim for free; no SmolAgents code needed.
2. **Direct Model wrap.** `InferenceClientModel`, `OpenAIServerModel`, `TransformersModel` have no LiteLLM path. The only version-stable surface is subclassing `Model` and wrapping an inner instance, mirroring `SpendGuardAgentsModel` and `SpendGuardChatCompletionClient`.

D25 ships the Model wrap as the primary path; `step_callbacks` is documented as informational only.

## 2. Goals

1. Public class `SpendGuardSmolModel(Model)` in `spendguard.integrations.smolagents` wrapping any inner `smolagents.Model`.
2. `generate()` (and `__call__` alias for `<1.5`) does reserve-before / commit-after / release-on-exception with semantics identical to `SpendGuardAgentsModel.get_response`.
3. Extra `spendguard-sdk[smolagents]` resolves to `smolagents>=1.5`.
4. Helper `spendguard_step_callback(client, run_id)` returns a `Callable[[ActionStep | PlanningStep], None]` for `MultiStepAgent(step_callbacks=[...])`. Emits an informational `agent_step` audit event; does NOT gate.
5. Demo mode `agent_real_smolagents`: `CodeAgent(model=SpendGuardSmolModel(inner=OpenAIServerModel(...)))` with a budget that allows call #1 and denies call #2.
6. Tests parametrized over `InferenceClientModel` and `OpenAIServerModel` (mocked transport) — load-bearing acceptance gate.
7. Reuses `RunContext` + `run_context()` from `spendguard.integrations.openai_agents` so polyglot stacks share one trace.
8. Docs page `docs/site/docs/integrations/smolagents.md` with the two-path decision table (D12 transitive vs D25 direct).

## 3. Non-goals

- Per-chunk streaming gating. `generate()` is the bracket boundary; tool calls inherit the parent reservation.
- `TransformersModel` compute-cost (GPU-second) accounting. Token-count POST estimation only.
- `step_callbacks` as a gating surface.
- Re-implementing `Model._prepare_completion_kwargs`. Wrapper delegates verbatim.
- Wrapping `LiteLLMModel`. D12 shim is the canonical path; docs page redirects there.

## 4. Architecture

```
CodeAgent.run("...")
  → SpendGuardSmolModel.generate(messages, stop_sequences, response_format, tools_to_call_from, **kwargs)
      ├─ ctx = current_run_context()                  [reused from openai_agents]
      ├─ signature = blake2b(messages | stop | tools | response_format | kwargs)
      ├─ llm_call_id / decision_id derived from signature
      ├─ sidecar.RequestDecision(LLM_CALL_PRE, projected_claims)
      │     ALLOW = continue · DENY/DEGRADE = raise (fail-closed)
      ├─ inner.generate(messages, ...)                [provider HTTP]
      └─ sidecar.emit_llm_call_post(SUCCESS|FAILURE|CANCELLED,
                                    estimated=ChatMessage.token_usage.total)
```

`ChatMessage` (>=1.5) exposes `token_usage: TokenUsage | None` with `input_tokens` + `output_tokens`. Sum mirrors `_extract_total_tokens` from `openai_agents.py:294-308`.

## 5. Key decisions

- **Composition over inheritance for the inner Model.** Constructor takes `inner: Model`; never instantiates `InferenceClient`, `OpenAI`, or `transformers.AutoModelForCausalLM`.
- **No `__init__` super call.** `smolagents.Model.__init__` sets attributes used only by direct vendor subclasses; the wrapper has no `model_id`. A super call would force a synthetic id and break inner introspection.
- **`__call__` alias.** `<1.5` agents call `model(messages, ...)`; `>=1.5` call `model.generate(...)`. The wrapper defines both, with `__call__` delegating to `generate`, so install-time version drift cannot bypass the gate.
- **`step_callbacks` helper is no-op-on-error.** Raising inside a step callback aborts the host agent run. The helper catches `Exception`, logs, and returns.
- **Reuse `RunContext` / `run_context()` from `openai_agents`** — polyglot stacks share one trace.
- **`CancelledError` → `outcome=CANCELLED`** in POST (matches D12 / D24). Other exceptions → `outcome=FAILURE` + re-raise.
- **No default `claim_estimator`.** `model_id` is set on `InferenceClientModel` / `OpenAIServerModel` but absent on `TransformersModel`; a uniform default is not safe.

## 6. Slice plan

| Slice | Title | Size |
|-------|-------|------|
| `COV_D25_S1_module_skeleton` | Module skeleton + `[smolagents]` extra + ImportError contract | S |
| `COV_D25_S2_wrap_generate` | `SpendGuardSmolModel.generate()` PRE/POST + `__call__` alias | M |
| `COV_D25_S3_step_callback_helper` | `spendguard_step_callback()` informational helper + safety wrap | S |
| `COV_D25_S4_tests_parametrized` | Unit + integration tests over `InferenceClientModel` + `OpenAIServerModel` (~20 tests) | M |
| `COV_D25_S5_demo_and_docs` | `agent_real_smolagents` demo mode + Makefile + verify SQL + docs page | M |

5 slices, S/M only, ~1100 LOC total (~350 impl + 500 test + 250 docs/yaml/demo).

## 7. Interfaces

```python
from spendguard.integrations.smolagents import SpendGuardSmolModel, spendguard_step_callback
from spendguard.integrations.openai_agents import RunContext, run_context

guarded = SpendGuardSmolModel(
    inner=OpenAIServerModel(model_id="gpt-4o-mini", api_base=..., api_key=...),
    client=spendguard_client, budget_id=..., window_instance_id=...,
    unit=..., pricing=...,
    claim_estimator=lambda messages: [common_pb2.BudgetClaim(...)],
)
agent = CodeAgent(model=guarded, tools=[...],
                  step_callbacks=[spendguard_step_callback(spendguard_client, run_id="...")])
async with run_context(RunContext(run_id="...")):
    result = await agent.arun("...")
```

Full operator code in `implementation.md` §2.

## 8. Open questions (locked)

1. **`<1.5` `__call__` shape:** locked — alias `__call__` to `generate`. Both versions route through one PRE/POST.
2. **`TransformersModel` cost model:** locked — token-count via `token_usage.total`. GPU-second tracked as D25.1.
3. **`step_callbacks` denying:** locked — informational only; reviewer rejects any callback that raises to deny.
