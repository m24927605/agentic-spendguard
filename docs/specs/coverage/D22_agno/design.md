# D22 — Agno `pre_hooks` / `post_hooks` adapter

**Status:** Spec — Tier 3 (`framework-coverage-build-plan-2026-06.md` §2.3).
**Owner sub-agent:** AI Engineer.
**Closest analogue:** `sdk/python/src/spendguard/integrations/langchain.py` (PRE/POST lifecycle + shared contextvar + idempotency derivation + default-estimator wiring). D22 reuses every primitive but ships **callable factories**, not a `BaseChatModel` subclass — Agno's per-provider `Model` classes wrap vendor SDKs directly, and the framework's first-class extension surface is `Agent(pre_hooks=[...], post_hooks=[...])`.

## 1. Problem

Agno (~40k stars, MPL-2.0, ex-Phidata) ships one `Model` per provider — no `BaseChatModel`-style trunk to subclass once. Wrapping each provider by subclass would track every vendor's release cadence and break for hand-rolled models. The framework's native extension surface is `pre_hooks` (after context prep, before LLM call) and `post_hooks` (after response), both stateless callables invoked via `inspect`-based dependency injection — closer to Pydantic-AI's `RunContext` injection than to LangChain's `BaseCallbackHandler`. A callable factory keeps SpendGuard model-agnostic and aligned with how Agno power-users extend the agent.

## 2. Goals

1. Ship `spendguard.integrations.agno` inside the `spendguard-sdk` wheel, opt-in via extra `[agno]`. Pairs with `agno >= 1.0`.
2. Public surface — TWO callable factories: `SpendGuardAgnoPreHook(client, budget_id, …) → Callable` (issues `request_decision(trigger=LLM_CALL_PRE)`, raises `DecisionDenied` to halt) and `SpendGuardAgnoPostHook(client, …) → Callable` (issues `emit_llm_call_post(…)` with real usage from `RunResponse.metrics`).
3. Behaviour parity with `SpendGuardChatModel`: PRE before vendor SDK fires; POST after with real usage; **PROVIDER_ERROR commit on RunError / missing metrics** so the projector releases the reservation.
4. Demo mode `agent_real_agno` proves (a) reserve lands before OpenAI is contacted and (b) DENY short-circuits the run without a vendor call.
5. Default estimator `agno_default_claim_estimator(...)` lives next to `langchain_default_claim_estimator` in `_default_estimator.py`; resolves the family from `agent.model.id` at call time.
6. Shared `_RUN_CONTEXT` contextvar across all four Python adapters so multi-framework agents reuse one `run_id`.

## 3. Non-goals

- Subclassing individual `Model` providers.
- Mid-stream chunk gating of `Agent.arun(stream=True)`. PRE before stream open, POST after stream end — POC parity with LangChain.
- Tool-call gating via Agno's `tool_hooks` — reserved for a follow-on; D22 covers `pre_hooks` + `post_hooks` only.
- DEGRADE mutation patch application. Surface as `MutationApplyFailed`; parity with `pydantic_ai`.
- Approval-resume UI. `ApprovalRequired` propagates out of the pre-hook; users handle resume.

## 4. Public surface — LOCKED

```python
from agno.agent import Agent
from agno.models.openai import OpenAIChat
from spendguard import SpendGuardClient
from spendguard.integrations.agno import (
    RunContext, SpendGuardAgnoPreHook, SpendGuardAgnoPostHook, run_context,
)

client = SpendGuardClient(socket_path=..., tenant_id=...)
await client.connect(); await client.handshake()

pre  = SpendGuardAgnoPreHook(client=client, budget_id=..., window_instance_id=..., unit=..., pricing=...)
post = SpendGuardAgnoPostHook(client=client, unit=..., pricing=...)

agent = Agent(model=OpenAIChat(id="gpt-4o-mini"),
              pre_hooks=[pre()], post_hooks=[post()])
async with run_context(RunContext(run_id="...")):
    response = await agent.arun("Hello")
```

## 5. Architecture

`Agent.arun()` walks `pre_hooks` after context prep, injecting whichever of `(agent, run_input, session, …)` the callable declares via `inspect`. The pre-hook declares `(agent, run_input)`, pulls `RunContext` from the shared contextvar, derives `llm_call_id` / `decision_id` / `idempotency_key` from a signature over `agent.model.id` + `run_input`, calls `request_decision`, and stores an `_InflightReservation` keyed by `(run_id, signature)` in a 10k-bounded FIFO map. STOP / DENY raises `DecisionDenied`, which Agno propagates out of `arun()`. The post-hook declares `(agent, run_response)`, pops the slot, extracts real usage from `RunResponse.metrics`, and calls `emit_llm_call_post`. On `RunError` / missing metrics → `PROVIDER_ERROR`; on missing slot → log once + no-op (no commit-without-reserve).

## 6. Locked design decisions

1. **Two callable factories, not a Model subclass.** Drop-in, provider-agnostic.
2. **Shared `_RUN_CONTEXT` contextvar name** (`spendguard_run_context`) across all four Python adapters.
3. **`llm_call_id` via blake2b-16** over `agent.model.id || run_input` (parity with `langchain.py:141`).
4. **Inflight map bounded** at 10k entries, FIFO eviction (parity with D04 §5).
5. **PROVIDER_ERROR commit on RunError / missing metrics.** Fail-closed release.
6. **`claim_estimator` optional** — auto-dispatched via new `agno_default_claim_estimator`.
7. **Streaming = PRE-before-open + POST-after-close.** Parity with LangChain POC.
8. **Extra `[agno]` declares `agno>=1.0,<2.0`** only; provider SDKs stay user-supplied.
9. **Hook parameter names are literal** `(agent, run_input)` / `(agent, run_response)` — Agno injects by name. `functools.wraps` NOT used.

## 7. Slice plan

| Slice | Title | Size |
|---|---|---|
| `COV_D22_01_module_skel_extra` | `integrations/agno.py` skeleton + `[agno]` extra + `RunContext` + shared contextvar + ImportError guard | S |
| `COV_D22_02_pre_post_hooks` | `SpendGuardAgnoPreHook` + `SpendGuardAgnoPostHook` factories + inflight map + signature derivation + bounded eviction + `agno_default_claim_estimator` | M |
| `COV_D22_03_tests_mock_sidecar` | unit + integration tests vs `MockSpendGuardClient` and real `Agent` with stubbed `OpenAIChat`; ≥ 22 cases incl. deny, PROVIDER_ERROR, retry, missing-metrics, signature, missing-context | M |
| `COV_D22_04_demo_docs` | `examples/agno-prehooks/run.py` + `agent_real_agno` demo-mode wiring + `docs/site/docs/integrations/agno.md` + README adapter row + memory-bank write-back | M |

## 8. Risks

- **Agno API drift** on `pre_hooks` signature injection — slice 1 pins `agno>=1.0,<2.0`; slice 3 asserts the injected keys remain valid via `inspect.signature(Agent.run)`.
- **`RunResponse.metrics` shape varies by provider** — `_extract_usage` helper has per-provider fall-through (parity with `langchain.py:339`).
- **Tool-hook coverage deferred** — called out in `docs/site/docs/integrations/agno.md`.
