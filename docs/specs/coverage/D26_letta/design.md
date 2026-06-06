# D26 — Letta (ex-MemGPT) `LLMClient` Subclass Adapter

**Status:** Spec — Tier 3, build plan `framework-coverage-build-plan-2026-06.md` §2.3.
**Owner:** AI Engineer.
**Closest analog:** `spendguard.integrations.openai_agents` — provider-abstraction subclass with PRE/POST gating. **Sibling reference:** [`D24_autogen_ag2`](../D24_autogen_ag2/).

## 1. Problem

Letta (formerly MemGPT, ~22k stars, Apache-2.0) is a stateful agent platform with persistent memory. Every LLM call inside `Agent.step()` flows through a per-provider concrete subclass of `letta.llm_api.llm_client_base.LLMClientBase`: `OpenAIClient`, `AnthropicClient`, `GoogleAIClient`, `DeepSeekClient`. Each implements `async send_llm_request(request_data, llm_config, tools, force_tool_use, ...) -> ChatCompletionResponse` plus a sync sibling.

There is **no formal pre-LLM middleware bus**. `step_callback` fires once per agent turn — turns frequently fan out into 3-4 internal LLM calls (reasoning → tool select → reflection), so step-level gating over-grants reservations.

The only safe surface that observes every LLM call is **subclassing `LLMClientBase` and overriding `send_llm_request` / `send_llm_request_sync`**. Letta is also widely self-hosted as `letta server` behind an OpenAI-compatible REST surface; for that shape, the egress-proxy drop-in (D02/D03) is canonical. D26 covers the embedded-library shape only.

## 2. Goals

1. Public class `SpendGuardLettaClient(LLMClientBase)` in `spendguard.integrations.letta` that wraps an inner `LLMClientBase` instance — provider-agnostic.
2. Wraps both `send_llm_request` (async) and `send_llm_request_sync` with reserve / call / commit, identical semantics to `SpendGuardAgentsModel`.
3. `wrap_llm_client(inner, *, client, ...)` factory operators call once and hand the result back to Letta's `Agent`.
4. Extras: `spendguard-sdk[letta]` resolves to `letta>=0.8,<1.0`.
5. Demo mode `agent_real_letta` exercising a Letta `Agent` with persistent memory turned on; deny path asserts zero provider HTTP.
6. Public docs page distinguishing **library mode** (D26 wrap) vs **server mode** (D02/D03), with a decision table that leads with server mode so operators don't waste a slice on the wrong path.
7. Reuses shared `RunContext` + `run_context()` from `spendguard.integrations.openai_agents` for polyglot trace sharing.

## 3. Non-goals

- Wrapping `step_callback` for coarse gating (documented inferior).
- Wrapping `letta.embeddings.*` — separate deliverable, different `BudgetClaim` shape.
- Patching Letta's `Message` table / DB schema.
- A Letta-side PR upstream — D26 is SDK-internal only.
- `letta server` mode — D02/D03 cover it.

## 4. Architecture

```
Agent.step()
  → (internal reasoning loop)
    → SpendGuardLettaClient.send_llm_request(request_data, llm_config, tools, ...)
        ├─ ctx = current_run_context()           [reused from openai_agents]
        ├─ signature = blake2b(request_data | llm_config | tools | force_tool_use)
        ├─ llm_call_id / decision_id derived from signature
        ├─ sidecar.RequestDecision(LLM_CALL_PRE, projected_claims)
        │     ALLOW = continue · DENY/DEGRADE = raise (fail-closed before HTTP)
        ├─ inner.send_llm_request(...)             [provider HTTP]
        └─ sidecar.emit_llm_call_post(SUCCESS|FAILURE|CANCELLED,
                                      estimated=usage.total_tokens)
```

`ChatCompletionResponse.usage` carries OpenAI-style `prompt_tokens` / `completion_tokens` / `total_tokens` regardless of inner provider — Letta normalizes via `convert_response_to_chat_completion` before returning. Validated against `letta 0.8.0` source.

## 5. Key decisions

- **Wrap the base class, not each provider.** One `SpendGuardLettaClient(LLMClientBase)` covers all Letta providers via composition. Per-provider matrix is rejected.
- **Composition over inheritance for inner.** Constructor takes `inner: LLMClientBase`. No provider SDK instantiation inside the wrapper.
- **Both async and sync overrides.** Older Letta code still uses `send_llm_request_sync`. Sync wrapper inside an active asyncio loop raises with a pointer to the async variant.
- **No `super().__init__()` call** — base init takes provider config the wrapper doesn't own. `__getattr__` delegates `llm_config` / `provider` / `build_request_data` / `convert_response_to_chat_completion` to inner.
- **Reuse `RunContext` / `run_context()` from `openai_agents`.** Polyglot stacks share one trace.
- **`step_callback` path documented as inadequate, not shipped.**
- **`force_tool_use` and `tools` are signature inputs.**
- **No default `claim_estimator`.** Operator MUST pass one — per-provider tokenizer mismatch makes a single default fragile.
- **Server-mode redirect is load-bearing.** Docs page leads with `letta server` → D02/D03.

## 6. Slice plan

| Slice | Title | Size |
|-------|-------|------|
| `COV_D26_S1_module_skeleton` | Module skeleton + `[letta]` extra + ImportError contract + `wrap_llm_client` factory stub | S |
| `COV_D26_S2_wrap_send_llm_request` | `send_llm_request()` PRE/POST with full signature passthrough | M |
| `COV_D26_S3_sync_and_passthrough` | `send_llm_request_sync()` bracket + `__getattr__` delegation | S |
| `COV_D26_S4_tests` | Unit + integration tests (real Letta `Agent`) + deny-path zero-HTTP assertion | M |
| `COV_D26_S5_demo_and_docs` | `agent_real_letta` demo mode + Makefile + verify SQL + integration docs page (library vs server decision table) | M |

5 slices, S/M only, ~1100 LOC total (~350 impl + 500 test + 250 docs/yaml/demo).

## 7. Interfaces

```python
from letta.llm_api.openai_client import OpenAIClient

from spendguard import SpendGuardClient
from spendguard.integrations.letta import wrap_llm_client
from spendguard.integrations.openai_agents import RunContext, run_context

inner = OpenAIClient(...)
guarded = wrap_llm_client(
    inner=inner,
    client=spendguard_client,
    budget_id=..., window_instance_id=...,
    unit=..., pricing=...,
    claim_estimator=lambda req: [common_pb2.BudgetClaim(...)],
)
# Hand `guarded` to Letta Agent per its documented LLMClient injection.
async with run_context(RunContext(run_id="...")):
    response = await agent.step(message)
```

Full operator sample in `implementation.md` §2.

## 8. Open questions (locked)

1. **Letta API churn:** `LLMClientBase.send_llm_request` stable since 0.6.x. Extras pin `letta>=0.8,<1.0`.
2. **`step_callback` gap:** out-of-scope; customers layer a callback on top.
3. **Server-mode redirect:** docs page leads with that decision. ~70% of Letta deployments are server-mode per Trend Researcher 2026-06.
4. **Sync API in async loop:** raises with an async-path pointer. No silent `asyncio.run()`.
