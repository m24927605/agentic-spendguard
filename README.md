<div align="center">

# 🛡️ Agentic SpendGuard

**The spend firewall for LLM agents.**

Stops runaway agents *before* the provider is called — not after the
invoice arrives the next morning. Budget reserved per-call, signed
audit trail, p50 ≤10ms decision overhead ([Contract §14 SLO](docs/specs/contract-dsl-spec-v1alpha1.md)).
Works with **LiteLLM proxy**, **OpenAI Agents SDK**, **LangGraph**,
**LangChain**, **Pydantic-AI**, and **Microsoft Agent Governance
Toolkit** ([community integration merged upstream](https://github.com/microsoft/agent-governance-toolkit/pull/2398)).

[![License: Apache 2.0](https://img.shields.io/badge/license-Apache%202.0-blue)](LICENSE)
[![PyPI: spendguard-sdk](https://img.shields.io/pypi/v/spendguard-sdk?label=pypi)](https://pypi.org/project/spendguard-sdk/)
[![Built with Rust 1.91](https://img.shields.io/badge/rust-1.91-orange)](deploy/demo/runtime/Dockerfile.ledger)
[![Postgres 15+ ledger](https://img.shields.io/badge/postgres-15%2B-336791)](services/ledger/migrations/)
[![mTLS gRPC](https://img.shields.io/badge/wire-mTLS%20gRPC-purple)](proto/)
[![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)](https://github.com/m24927605/agentic-spendguard/issues)

</div>

```bash
pip install 'spendguard-sdk[litellm]'
```

→ [90-second demo](#-quick-start-30-seconds) · [Microsoft AGT integration](https://github.com/microsoft/agent-governance-toolkit/blob/main/docs/integrations/spendguard-integration.md) · [Architecture](#%EF%B8%8F-how-it-works)

---

## 💡 Why this exists

Picture the failure mode SpendGuard is built to stop:

A customer-support agent hits a rate-limited tool at 2:47am. The retry
policy kicks in. The agent loop re-plans, re-prompts, re-tries — each
retry a fresh `gpt-4o` call with the full conversation in context.
Forty minutes later, one stuck conversation has consumed ~$380 in
tokens. Multiply across the other tenants doing the same during the
incident.

The post-mortem starts with *"we didn't know until the OpenAI dashboard
updated the next morning."*

**SpendGuard moves detection from tomorrow to the 11th call.** Every
request reserves tokens against a per-tenant budget before the provider
is called. Budget exhausted → the call is refused with a signed audit
row of why (HTTP 429 from the egress proxy; HTTP 403 from the LiteLLM
callback — see [adapter integrations](#-adapter-integrations) for which path your
client takes). The provider is never hit.

The standard answer — *"track usage, send alerts"* — is reconciliation,
not control. You see the bill **after** it lands. SpendGuard inverts
this: if the agent isn't allowed to spend that much on that model under
that tenant right now, the LLM call never happens.

---

## 🚀 Quick start (30 seconds)

```bash
git clone git@github.com:m24927605/agentic-spendguard.git
cd agentic-spendguard
export OPENAI_API_KEY=sk-...
make demo-up DEMO_MODE=proxy
```

That spins up Postgres + ledger + sidecar + the egress proxy, then runs a real `gpt-4o-mini` call through it. **Your application code stays unchanged:**

```python
from openai import OpenAI

client = OpenAI(
    base_url="http://localhost:9000/v1",   # ← only change
    api_key=os.environ["OPENAI_API_KEY"],
)
client.chat.completions.create(model="gpt-4o-mini", messages=[...])
```

| Decision | HTTP | Body |
|---|---|---|
| **CONTINUE** (budget available) | 200 | OpenAI's response byte-identical; ledger writes a `commit_estimated` audit row |
| **STOP** (over hard-cap) | 429 + `Retry-After: 86400` | `{"error":{"code":"spendguard_blocked","details":{"reason_codes":["BUDGET_EXHAUSTED"],...}}}` — **the HTTP request never reaches OpenAI** |

If you've integrated Stripe before: this is **auth/capture, applied to LLM tokens**. PRE the call, the proxy reserves the cost against a configured budget; POST the call, it captures the real `usage.total_tokens`. Idempotent, atomic, fail-closed.

---

## 📊 Head-to-head benchmark

Identical fixture — 100 attempted calls, $1.00 budget, $0.18 per call — through three drop-in budget tools, reporting ground-truth `$` spent against a centralized pricing table.

```text
$ make benchmark
```

| Runner | Budget | Wire calls | $ spent | Overshoot |
|---|---|---:|---:|---:|
| **Agentic SpendGuard** | $1.00 | 5 | $0.90 | **−10.0%** ✅ |
| `agentbudget` | $1.00 | 6 | $1.08 | +8.0% |
| `agent-guard` | $1.00 | 100 | $18.00 | **+1700%** ❌ |

- **`agentbudget`** overshoots by one call because enforcement is **post-call** (the 6th call completes on the wire, *then* it raises `BudgetExhausted`).
- **`agent-guard`** doesn't enforce at all because its HTTP-level interception is hardcoded to `openai.com` / `anthropic.com` and silently no-ops the moment you point an OpenAI client at a self-hosted base URL.
- **Agentic SpendGuard** does **pre-call reservation** against a ledger and refuses call #6 before it leaves the runner.

Reproducible benchmark in [`benchmarks/runaway-loop/`](benchmarks/runaway-loop/). Full results in [RESULTS.md](benchmarks/runaway-loop/RESULTS.md).

### Predictor-upgrade benchmark (SLICE_15)

The predictor upgrade adds a concurrent-burst benchmark comparing decision-time latency + overshoot against LiteLLM proxy at 1 / 10 / 100 concurrent calls. SpendGuard with the SLICE_06 output_predictor + SLICE_09 run_cost_projector tracks p99 < 50ms (Contract DSL §14 SLO) and overshoot below LiteLLM at every burst level.

```bash
# Bring up demo + run the burst harness:
bash tests/e2e/predictor_upgrade.sh
cd benchmarks/predictor-upgrade && cargo build --release
./target/release/predictor-upgrade-bench --bursts 1,10,100 --output ./out
```

| Burst | SpendGuard p99 | LiteLLM p99 | SpendGuard overshoot | LiteLLM overshoot |
|---:|---:|---:|---:|---:|
| 1   | _populated by run_ | _populated by run_ | _populated by run_ | _populated by run_ |
| 10  | _populated by run_ | _populated by run_ | _populated by run_ | _populated by run_ |
| 100 | _populated by run_ | _populated by run_ | _populated by run_ | _populated by run_ |

Latest CI numbers + reproduction details: [`benchmarks/predictor-upgrade/RESULTS.md`](benchmarks/predictor-upgrade/RESULTS.md). Calibration accuracy on a synthetic 1000-call workload: [`benchmarks/predictor-upgrade/calibration_synthetic.py`](benchmarks/predictor-upgrade/calibration_synthetic.py) (slice §8.3 asserts SpendGuard P95 |predicted − actual| / actual ≤ 5%). Portkey: documented N/A — closed-source proxy not benchmark-able from the open repo. Spec set locked on the SLICE_15 merge.

---

## 🧰 What works today

The 1-env-var claim is verified **end-to-end against real OpenAI** for:

| Client | Status | What you change |
|---|:---:|---|
| 🐍 `openai-python` (`from openai import OpenAI`) | ✅ | `base_url=...` |
| 🦜 LangChain `ChatOpenAI` | ✅ | `base_url=...` |
| 🕸️ LangGraph (via `ChatOpenAI`) | ✅ | `base_url=...` |
| 🤖 openai-agents shorthand `Agent(model="...")` | ✅ | `OPENAI_BASE_URL=...` |
| 🌊 Streaming (`stream:true`) on both endpoints | ✅ | (transparent) |

For approval workflows, model-tier degradation, and multi-budget claims that the proxy doesn't yet cover, there's an [SDK wrapper-mode path](#-sdk-advanced-wrapper-mode) below.

Specs:
- Auto-instrument proxy: [`docs/specs/auto-instrument-egress-proxy-spec.md`](docs/specs/auto-instrument-egress-proxy-spec.md) (v7 LOCKED)
- v0.2 streaming SSE: [`docs/specs/egress-proxy-v0.2-streaming-sse.md`](docs/specs/egress-proxy-v0.2-streaming-sse.md)
- v0.3 `/v1/responses` (openai-agents default): [`docs/specs/egress-proxy-v0.3-responses-api.md`](docs/specs/egress-proxy-v0.3-responses-api.md)

---

## 🛡️ How it works

Three layers. The proxy is the thing your client talks to. The other two are infrastructure.

### 1. Egress proxy (Rust + axum)
- Forwards `POST /v1/chat/completions` and `POST /v1/responses` to OpenAI byte-identically on the success path.
- On budget breach: returns **HTTP 429** with a structured `spendguard_blocked` body the client can branch on. **The upstream OpenAI request never fires.**
- Streaming variant: tees the SSE stream to the client byte-identical while side-parsing usage for the commit lane.

### 2. Sidecar (Rust + tonic over UDS)
- Per-pod. Holds a contract DSL evaluator + the gRPC client to the ledger.
- Decides `Continue` / `Stop` / `RequireApproval` / `Degrade` for every LLM call.
- Signs every decision with Ed25519 or AWS KMS ECDSA P-256.

### 3. Audit chain (Postgres + signed CloudEvents)
- Every reservation, commit, release, and denied decision is an immutable row in `audit_outbox`.
- DB-enforced triggers refuse `UPDATE` / `DELETE`. The chain is **tamper-evident**.
- An outbox forwarder closes the loop into `canonical_events`, downstream ETL / SIEM consumers can subscribe.

```
agent  ──HTTP──▶  egress-proxy  ──UDS gRPC──▶  sidecar  ──TLS gRPC──▶  ledger
                       │                                                  │
                       └── byte-identical forward to OpenAI on Continue   │
                                                                          ▼
                                                       audit_outbox (signed, immutable)
                                                                          │
                                                                          ▼
                                                       outbox-forwarder ─▶ canonical_events
                                                                          │
                                                                          ▼
                                                              your SIEM / data lake
```

---

## 🎚️ Capability levels (L0–L3)

Pick the trust model that fits how much your agent's code can be trusted not to bypass the gate.

| Level | What it does | Where the agent can cheat |
|---|---|---|
| **L0** advisory_sdk | SDK logs decisions to sidecar; never blocks | Agent code that bypasses the SDK |
| **L1** semantic_adapter | SDK refuses the upstream call on STOP | Agent that imports the LLM client directly |
| **L2** egress_proxy_hard_block | Network egress proxy rejects un-gated traffic | (none — agent must use the proxy) |
| **L3** provider_key_gateway | Provider API keys live in a gateway; agent never sees them | (none — provider rotates keys) |

POC default is **L3** (recommended for any pilot that runs against a real provider key); lower levels exist for backward-compat with older adapters.

---

## 📦 SDK (advanced wrapper-mode)

For agents that need `REQUIRE_APPROVAL` / `DEGRADE` decisions, multi-budget claims, or custom claim estimators, install the Python SDK:

```bash
pip install --pre spendguard-sdk

# or with a framework integration:
pip install --pre 'spendguard-sdk[pydantic-ai]'
pip install --pre 'spendguard-sdk[langchain]'
pip install --pre 'spendguard-sdk[langgraph]'
pip install --pre 'spendguard-sdk[openai-agents]'
pip install --pre 'spendguard-sdk[agt]'
```

```python
from spendguard import SpendGuardClient, ApprovalRequired, DecisionStopped

async with SpendGuardClient(
    socket_path="/var/run/spendguard/adapter.sock",
    tenant_id=TENANT,
) as sg:
    await sg.handshake()
    try:
        outcome = await sg.request_decision(
            trigger="LLM_CALL_PRE",
            run_id=run_id, step_id=step_id, llm_call_id=call_id,
            decision_id=decision_id, route="llm.call",
            projected_claims=[claim],
            idempotency_key=derive_idempotency_key(...),
        )
        # OK to make the LLM call. outcome.reservation_ids holds the auth.
    except DecisionStopped as e:
        raise
    except ApprovalRequired as e:
        resume_outcome = await e.resume(sg)  # waits for operator
```

| Framework | Module | What gets gated | Runnable example |
|---|---|---|---|
| **Pydantic-AI** | `spendguard.integrations.pydantic_ai` | Every `Model.request()` | — |
| **LangChain** | `spendguard.integrations.langchain` | Every `BaseChatModel` invocation | — |
| **LangGraph** | same module | Same wrapper (LangGraph builds on `BaseChatModel`) | — |
| **LangChain.js (TS/JS)** | `@spendguard/langchain` (`SpendGuardCallbackHandler`) | Every LangChain.js `BaseChatModel` / `BaseLLM` invocation; covers LangGraph because it builds on `BaseChatModel` | [`examples/langchain-ts/`](examples/langchain-ts/) |
| **Vercel AI SDK + Mastra (TS/JS)** | `@spendguard/vercel-ai` (`createSpendGuardMiddleware`) — `/mastra` subpath alias re-exports as `createSpendGuardLanguageMiddleware` | Every `wrapLanguageModel({ middleware })` call drives `generateText` / `streamText` through the SpendGuard middleware; covers Mastra Agents because Mastra resolves to the same `LanguageModelV1` boundary | [`examples/vercel-ai-mastra/`](examples/vercel-ai-mastra/) |
| **OpenAI Agents SDK (Python)** | `spendguard.integrations.openai_agents` | Every model call inside an `Agent` run | [`examples/openai-agents-composite/`](examples/openai-agents-composite/) |
| **OpenAI Agents SDK (TS/JS)** | `@spendguard/openai-agents` (`withSpendGuard(model, opts)` factory + `SpendGuardAgentsModel` class) | Every `Runner.run(agent, ...)` call drives `getResponse(request)` through the SpendGuard bracket; multi-framework runs share one trace via the shared `runContext()` AsyncLocalStorage | [`examples/openai-agents-ts-composite/`](examples/openai-agents-ts-composite/) |
| **Inngest AgentKit (TS/JS)** | `@spendguard/inngest-agent-kit` (`wrapWithSpendGuard(step.ai, client, opts)` factory) | Every `step.ai.infer()` / `step.ai.wrap()` call drives `reserve` → provider → `commitEstimated` through the SpendGuard bracket; **N retry attempts of the same step body collapse to ONE logical reservation** via Inngest's step identity reuse — the headline retry-dedup contract | [`examples/inngest-agent-kit/`](examples/inngest-agent-kit/) |
| **Microsoft Agent Framework (.NET + Python)** | `Spendguard.AgentFramework` NuGet (`services.AddSpendGuard(...)` + `inner.UseSpendGuard(sp)` DI extension) **and** `spendguard.integrations.agent_framework` (`SpendGuardMiddleware(client=..., ...)` subclass of `agent_framework.ChatMiddleware`) | Every MAF `IChatClient.GetResponseAsync(...)` (.NET) / `ChatClient.get_response(...)` (Python) call drives `reserve` → provider → `commitEstimated` through the SpendGuard bracket at the chat-client boundary `Microsoft.Agents.AI.ChatAgent` itself uses; both languages share one design and one demo matrix | [`examples/maf-dotnet/`](examples/maf-dotnet/) · [`examples/maf-python/`](examples/maf-python/) |
| **Microsoft AGT** | `spendguard.integrations.agt` | AGT's PolicyEngine + SpendGuard as a policy plugin | [`microsoft/agent-governance-toolkit#2398`](https://github.com/microsoft/agent-governance-toolkit/pull/2398) |
| **Google ADK (Python)** | `spendguard.integrations.adk` (`SpendGuardAdkCallback` — register the **same instance** to both `before_model_callback` and `after_model_callback`) | Every `LlmAgent` model turn: PRE reserves via `RequestDecision(LLM_CALL_PRE)`, on DENY returns a synthetic `LlmResponse(error_code="SPENDGUARD_DENY")` so ADK short-circuits the model call. POST commits with real `usage_metadata.total_token_count`. Multi-vendor (Gemini direct, Vertex Gemini, LiteLlm-wrapped OpenAI / Anthropic) — usage extraction is by response shape, not by model-string parsing | [`deploy/demo/agent_real_adk/`](deploy/demo/agent_real_adk/) |
| **AWS Strands (Python)** | `spendguard.integrations.strands` (`SpendGuardStrandsHookProvider` — register on the `Agent` via `hooks=[provider]`) | Every `Agent.invoke_async()` turn: `before_invocation` reserves via `RequestDecision(LLM_CALL_PRE)`, on DENY raises `DecisionDenied` BEFORE Strands dispatches the model HTTP. `after_invocation` commits with real `result.usage` tokens. Multi-vendor by shape, not by string: Bedrock + OpenAI + Anthropic + Gemini + Ollama + LiteLLM gated identically with one provider instance — gating sits at the agent-runtime boundary, not the model boundary | [`deploy/demo/agent_real_strands/`](deploy/demo/agent_real_strands/) |
| **DSPy (Python)** | `spendguard.integrations.dspy` (`pip install 'spendguard-sdk[dspy]'` — `SpendGuardDSPyCallback` registers via `dspy.configure(callbacks=[callback])`, MUST be FIRST in the list) | Every `dspy.LM` invocation (inside `dspy.Predict` / `dspy.ChainOfThought` / `dspy.ReAct` / custom modules): `on_lm_start` reserves via `RequestDecision(LLM_CALL_PRE)`, on DENY raises `DecisionDenied` BEFORE DSPy dispatches the LM HTTP. `on_lm_end` commits with real `outputs[0].usage` tokens. D12 + D21 coexist safely: a shared `_SHIM_IN_FLIGHT` contextvar ensures exactly ONE reserve when both adapters are installed. Custom `dspy.LM` subclasses that bypass LiteLLM are gated identically — load-bearing direct-path coverage proof | [`deploy/demo/agent_real_dspy/`](deploy/demo/agent_real_dspy/) |
| **Agno (Python)** | `spendguard.integrations.agno` (`pip install 'spendguard-sdk[agno]'` — `SpendGuardAgnoPreHook` + `SpendGuardAgnoPostHook` factories register via `Agent(pre_hooks=[pre()], post_hooks=[post()])`) | Every `Agent.arun()` turn: pre-hook reserves via `RequestDecision(LLM_CALL_PRE)`, on DENY wraps `DecisionDenied` into Agno's `InputCheckError` (DEVIATION-1 — Agno 2.x hook loop only re-raises `Input/OutputCheckError`) so the model HTTP NEVER fires; post-hook commits with real `run_output.metrics.total_tokens`. Multi-vendor by shape, not by string: OpenAIChat + Claude + Gemini + Groq + xAI + DeepSeek gated identically with one hook pair — gating sits at the agent-runtime boundary, not the model boundary. Pin `agno >= 2.0,< 3` (the spec's `>=1.0,<2.0` cap was widened because `pre_hooks` / `post_hooks` only ship in the 2.x line) | [`deploy/demo/agent_real_agno/`](deploy/demo/agent_real_agno/) |
| **BeeAI Framework (Python)** | `spendguard.integrations.beeai` (`pip install 'spendguard-sdk[beeai]'` — `subscribe_spendguard(agent, client, ...)` returns an unsubscribe callable, install once per `BaseAgent`) | Every BeeAI LLM step: `*.start` handler reserves via `RequestDecision(LLM_CALL_PRE)`, on DENY `DecisionDenied` propagates (BeeAI's `Emitter._invoke` wraps as `EmitterError` preserving `__cause__`) so the model HTTP NEVER fires; `*.success` handler commits with real `usage.total_tokens`. One subscriber covers every `ChatModel` provider (OpenAIChatModel + WatsonxChatModel + OllamaChatModel + GroqChatModel + custom) because gating sits at the agent-runtime `Emitter`, not the model boundary. Pin `beeai-framework >= 0.1.81,< 0.2` (DEVIATION-A — spec's `>=0.3,<1.0` cap predates the first PyPI publish under the `beeai-framework` name; reality is the 0.1.x line) | [`deploy/demo/agent_real_beeai/`](deploy/demo/agent_real_beeai/) |
| **AutoGen / AG2 (Python)** | `spendguard.integrations.autogen` (`pip install 'spendguard-sdk[autogen]'` — `SpendGuardChatCompletionClient(inner=..., claim_estimator=...)` subclasses `autogen_core.models.ChatCompletionClient` and passes as `AssistantAgent(model_client=...)`) | Every `AssistantAgent.on_messages(...)` turn (also `MagenticOneGroupChat`, `Swarm`): wrapper's `create()` reserves via `RequestDecision(LLM_CALL_PRE)` BEFORE inner HTTP, on DENY `DecisionDenied` propagates straight out of `create()` (no framework-side catch — verified against autogen-core 0.4.0 + ag2 0.7.0) so the model HTTP NEVER fires; commits with real `CreateResult.usage.prompt_tokens + completion_tokens`. ONE wrapper covers BOTH lineages — AutoGen 0.4+ (Microsoft, maintenance mode) AND AG2 (community fork, ~48k stars) — because they share `autogen_core.models.ChatCompletionClient` unchanged through at least AG2 0.7.x. The `LINEAGE` constant tells operators which lineage is loaded (`autogen` / `ag2` / `both` / `core-only`) but is telemetry-only — business logic never branches on it (review-standards §1.1). Multi-vendor by ABC composition: OpenAIChatCompletionClient / AnthropicChatCompletionClient / AzureAIChatCompletionClient / LiteLLM-routed gated identically with one wrapper instance | [`deploy/demo/agent_real_autogen/`](deploy/demo/agent_real_autogen/) |
| **SmolAgents (Python)** | `spendguard.integrations.smolagents` (`pip install 'spendguard-sdk[smolagents]'` — `SpendGuardSmolModel(inner=..., claim_estimator=...)` subclasses `smolagents.Model` and passes as `CodeAgent(model=...)` / `ToolCallingAgent(model=...)`) | Every `Model.generate(messages, ...)` call: wrapper's sync `generate()` reserves via `RequestDecision(LLM_CALL_PRE)` BEFORE inner HTTP, on DENY `DecisionDenied` propagates straight out (no framework-side catch in `MultiStepAgent.step` — verified against smolagents 1.26) so the model HTTP NEVER fires; commits with real `ChatMessage.token_usage.input_tokens + output_tokens`. ONE wrapper covers `InferenceClientModel` (HF Inference API) / `OpenAIServerModel` (vLLM / Ollama / Together / Groq / OpenAI-compatible) / `TransformersModel` (in-process HF transformers) because gating sits at the ABC layer. `__call__` aliased to `generate` so `smolagents<1.5` legacy agents that invoke `model(messages, ...)` route through the same gate. **DEVIATION-1 vs spec**: `Model.generate` is SYNCHRONOUS (not async) in smolagents 1.5+; the wrapper bridges to the async sidecar RPCs via `asyncio.run` with a sticky `SyncInAsyncContext` guard (mirrors the DSPy 2.6 sync-callback precedent). LiteLLMModel inner is REFUSED at construction (would double-gate via D12) — point operators to the [LiteLLM SDK shim](https://agenticspendguard.dev/docs/integrations/litellm-sdk-shim/) instead. `step_callbacks=[spendguard_step_callback(...)]` is informational telemetry only — does NOT gate. Pin `smolagents >= 1.5,< 2` | [`deploy/demo/agent_real_smolagents/`](deploy/demo/agent_real_smolagents/) |
| **Letta 0.8+ (library mode)** | `spendguard.integrations.letta` (`pip install 'spendguard-sdk[letta]'` — `SpendGuardLettaClient` / `wrap_llm_client(inner=OpenAIClient(...), ...)` subclasses `letta.llm_api.llm_client_base.LLMClientBase` and is handed to Letta's `Agent` per its documented LLMClient injection point) | Every `Agent.step()` internal LLM call (3-4 per turn — reasoning → tool select → reflection): wrapper's `send_llm_request()` reserves via `RequestDecision(LLM_CALL_PRE)` BEFORE inner HTTP, on DENY `DecisionDenied` propagates straight out of `send_llm_request` (no framework-side catch — verified against letta 0.8.0) so the provider HTTP NEVER fires; commits with real `ChatCompletionResponse.usage.total_tokens` (Letta normalizes every provider response to OpenAI-shaped usage). ONE wrapper covers every Letta provider — OpenAIClient / AnthropicClient / GoogleAIClient / DeepSeekClient — because gating sits at the ABC layer (composition, not inheritance). `__getattr__` delegates `llm_config` / `provider` / `build_request_data` / `convert_response_to_chat_completion` to the inner client without side effects. `send_llm_request_sync` raises if called inside an active asyncio loop (silent `asyncio.run()` re-entry is a release-blocking defect). D26 is **library-mode only** — `letta server` REST deployments (~70% of installs) use D02 + D03 egress proxy instead; LiteLLM-routed Letta uses D12 transitively. D26 cap: `letta>=0.8,<1.0` (D25 SmolAgents precedent for upstream-locked floor) | [`deploy/demo/agent_real_letta/`](deploy/demo/agent_real_letta/) |
| **LlamaIndex (Python)** | `spendguard.integrations.llamaindex` (`pip install 'spendguard-sdk[llamaindex]'` — `Settings.callback_manager = CallbackManager([SpendGuardLlamaIndexHandler(client=...)])` subclasses `llama_index.core.callbacks.base_handler.BaseCallbackHandler` and registers via `Settings.callback_manager`, propagating to every `LLM` in the query graph) | Every `CBEventType.LLM` event published by LlamaIndex provider integrations: handler's `on_event_start` reserves via `RequestDecision(LLM_CALL_PRE)` BEFORE the inner provider HTTP, on DENY raises `SpendGuardLlamaIndexDenied` (LlamaIndex's `CallbackManager.event(...)` context manager propagates it out before the LLM dispatches) so the provider HTTP NEVER fires; `on_event_end` commits with extracted total tokens. ONE handler covers `llama-index-llms-openai` (`raw["usage"]["total_tokens"]`), `-anthropic` (`raw["usage"]["input_tokens"] + ["output_tokens"]`), `-gemini` (`raw["usage_metadata"]["total_token_count"]`), and `-bedrock-converse` (`raw["usage"]["inputTokens"] + ["outputTokens"]`) — vendor detection is by response shape, not class-name parsing. Non-LLM events (`CBEventType.EMBEDDING` / `RETRIEVE` / `CHUNK` / `QUERY` / `NODE_PARSING`) are filtered with a single enum compare at handler entry (zero sidecar calls). The handler owns a per-instance daemon thread + asyncio loop and bridges sync LlamaIndex callbacks to the async sidecar client via `asyncio.run_coroutine_threadsafe` (no `nest_asyncio` required). Two-path coverage matrix: `llama-index-llms-litellm` is covered transitively by D12 (the LiteLLM SDK shim) — operators install BOTH D12 + D27 for mixed setups, D12's contextvar recursion guard prevents double-reservation on the LiteLLM-routed inner call. D27 cap: `llama-index-core>=0.12` (callback surface stable across the 0.12.x line) | [`deploy/demo/agent_real_llamaindex/`](deploy/demo/agent_real_llamaindex/) |
| **Atomic Agents (Python) + Instructor** | `spendguard.integrations.atomic_agents` (`pip install 'spendguard-sdk[atomic-agents]'` — `wrap_instructor_client(inner=instructor.from_openai(OpenAI()), claim_estimator=...)` returns a `SpendGuardInstructorProxy` / `SpendGuardAsyncInstructorProxy` you hand to `BaseAgent(BaseAgentConfig(client=guarded, ...))`) | Every `BaseAgent.run({...})` call AND **every Instructor validation-retry attempt**: gated raw `inner.client.chat.completions.create` reserves via `RequestDecision(LLM_CALL_PRE)` BEFORE the provider HTTP, on DENY raises `DecisionDenied` straight out of the gated raw (Atomic Agents 2.x `BaseAgent` has no framework-side catch — verified against `atomic-agents==2.8.0`) so the provider HTTP NEVER fires; commits with real `ChatCompletion.usage.total_tokens` per attempt. **DEVIATION-C from spec**: the spec described wrapping `chat.completions.create_with_completion`, but in Instructor 1.14+ that's a single outer call — the retry loop calls the raw provider method per attempt. The proxy intercepts the raw method via `inner.client.chat.completions.create` (or `inner.create_fn.__wrapped__` fallback), wraps it with PRE/POST, then re-runs `instructor.patch(create=gated_raw, mode=inner.mode)` so Instructor's retry loop drives our gated raw per attempt → each retry gets its own reservation. One wrapper covers every Instructor backend (OpenAI / Anthropic / Gemini / Cohere). DEVIATION-A: `atomic-agents>=2.0,<3` (spec said `>=1.0,<2.0`; reality is 2.x on PyPI) | [`deploy/demo/agent_real_atomic_agents/`](deploy/demo/agent_real_atomic_agents/) |
| **LiteLLM proxy** (legacy `CustomLogger` callback) | `spendguard.integrations.litellm` | Every `/v1/chat/completions` through the LiteLLM proxy | [`docs/specs/litellm-integration/PROXY_RECIPE.md`](docs/specs/litellm-integration/PROXY_RECIPE.md) |
| **LiteLLM proxy guardrail** (new `CustomGuardrail` registry) | `spendguard.integrations.litellm_guardrail` | Every `/v1/chat/completions` through the LiteLLM proxy, registered via `guardrails:` (zero-Python install for single-tenant) | [LiteLLM proxy guardrail docs](https://agenticspendguard.dev/docs/integrations/litellm-proxy/) |
| **LiteLLM SDK monkey-patch shim** (new in 0.5.1) | `spendguard.integrations.litellm_sdk_shim` (`install_shim(SpendGuardShimOptions(...))`) | Every `litellm.acompletion` / `completion` / `Router.acompletion` call in the running interpreter — **including transitive callers**: CrewAI, DSPy, SmolAgents, Strands, BeeAI, AutoGen, Atomic Agents. One `install_shim()` at boot gates every framework that uses LiteLLM as its LLM transport, with no framework-side changes. Closes [LiteLLM Issue #8842](https://github.com/BerriAI/litellm/issues/8842) | [LiteLLM SDK shim docs](https://agenticspendguard.dev/docs/integrations/litellm-sdk-shim/) |
| **Kong AI Gateway** | `plugins/kong/spendguard-go/` (Go plugin) **and** `plugins/kong/spendguard-lua/` (experimental Lua) | Kong DataPlane `access` (reserve) + `body_filter` (commit). Bind via `KongPlugin` CRD or `kong.conf`; plugin dials a SpendGuard companion service over HTTPS+mTLS; covers OpenAI `/v1/chat/completions` + Anthropic `/v1/messages` upstreams | [`examples/kong-gateway-composite/`](examples/kong-gateway-composite/) |
| **Dify Model Provider Plugin** | `plugins/dify/spendguard/` (Python plugin, packaged as `.difypkg`) | Every Dify `chat-message` / agent step / workflow LLM node routes through `SpendGuardLLM._invoke()` → sidecar reserve → upstream (OpenAI or Anthropic) → end-of-stream commit; install via `dify plugin install` (Dify Cloud) or sideload `.difypkg` (self-host); covers SSE streaming + `get_num_tokens` via sidecar `/v1/tokenize` HTTP companion | [`plugins/dify/spendguard/`](plugins/dify/spendguard/) |
| **Botpress (v12 self-host + Cloud)** | `integrations/botpress/` (TypeScript Integration SDK package — `npm i @spendguard/botpress-integration && botpress integrations push`) | Botpress 0.7 `beforeAiGeneration` hook reserves via the SpendGuard sidecar HTTP companion `/v1/decision` BEFORE Botpress dispatches upstream; `afterAiGeneration` commits real `inputTokens + outputTokens` via `/v1/trace`; DENY / DEGRADE throw Botpress `RuntimeError(BUDGET_DENIED \| BUDGET_DEGRADED)` (fail-closed default; dev escape `SPENDGUARD_BOTPRESS_FAIL_OPEN=1`). `validateConfiguration` issues a 1-token reserve+release probe at integration register time so sidecar misconfig surfaces in the operator form save. Covers OpenAI / Anthropic / Bedrock upstreams. | [`integrations/botpress/`](integrations/botpress/) · [`deploy/demo/botpress_real/`](deploy/demo/botpress_real/) |
| **Flowise (no-code visual builder, self-host)** | `integrations/flowise/` (TypeScript Flowise custom node — `npm i @spendguard/flowise-nodes` into a Flowise source install, or copy the package's `dist/` to `~/.flowise/nodes/spendguard/`, or layer the wrapper onto a custom `flowiseai/flowise:2.x` Docker image) | One canvas node — **SpendGuard ChatModel Wrapper** under category `Spend Guard` — drops between any Flowise `BaseChatModel` (ChatOpenAI / ChatAnthropic / ChatBedrock / ChatGoogleVertexAI / ...) and the downstream Chain / Agent. Reuses D04's `SpendGuardCallbackHandler` verbatim: the wrapper's `init()` appends the handler to the inner ChatModel's `callbacks` array IN PLACE and returns the SAME reference so downstream nodes see a normal `BaseChatModel`. Module-level `SpendGuardClient` cache keyed by `(tenantId, sidecarUds)` avoids per-invocation UDS reconnect (Flowise instantiates an INode per chatflow execution). No-code `claimEstimatorJson` input gives a conservative `$1` USD-micros default per call (so a builder dragging the wrapper without writing TypeScript still gets a working install) with a JSON override for per-route tuning. Self-hosted Flowise only; Flowise Cloud out of scope. ESM-only, Node 20.10+; peer-deps `@spendguard/sdk` + `@spendguard/langchain` + `flowise-components>=2.0.0`. | [`integrations/flowise/`](integrations/flowise/) · [`deploy/demo/flowise_real/`](deploy/demo/flowise_real/) · [`examples/flowise/`](examples/flowise/) |
| **Langflow custom component** (DataStax visual builder, self-host) | `plugins/langflow/` (Python Langflow custom component — `pip install spendguard-langflow-component && spendguard-langflow-install --target $LANGFLOW_COMPONENTS_PATH`) | One canvas card — **SpendGuard Budget Gate** under category `Models` — wraps any LangChain `BaseChatModel` (ChatOpenAI / ChatAnthropic / ChatVertexAI / ...) connected through its `Inner Model` HandleInput. Downstream Langflow nodes see a `LanguageModel` handle indistinguishable from a raw `ChatOpenAI`. Reuses `spendguard.integrations.langchain.SpendGuardChatModel` verbatim — zero re-implementation of the reservation lifecycle. Auto-binds `run_context(...)` per canvas invocation using `self.graph.flow_id`; caller-bound contexts always win (INV-3). Tags every decision in the audit chain as `integration=langchain, source=langflow` so SpendGuard operators can distinguish Langflow-driven calls from raw LangChain SDK callers. Fail-closed default (DEGRADE → `DecisionSkipped` surfaces as canvas error node). Vendor-drop install script refuses system paths (`/usr`, `/etc`, ...) per INV-8. Self-hosted Langflow 1.8+ only; Langflow Cloud (DataStax-hosted) marketplace push deferred. Python 3.10+; peer-deps `spendguard-sdk[langchain]>=0.5.1` + `langflow>=1.8.0,<2.0.0`. | [`plugins/langflow/`](plugins/langflow/) · [`deploy/demo/langflow_real/`](deploy/demo/langflow_real/) |
| **n8n (workflow automation, self-host)** | `sdk/typescript-n8n/` (n8n community node — `n8n npm install n8n-nodes-spendguard` with `N8N_COMMUNITY_PACKAGES_ENABLED=true`) | One canvas sub-node — **SpendGuard Chat Model** under codex category `AI → Language Models` — drops between any `ai_languageModel`-producing node (`lmChatAnthropic` / `lmChatOpenAi` / ...) and the **AI Agent**. `supplyData` resolves the upstream `BaseChatModel`, attaches D04's `SpendGuardCallbackHandler` to its `callbacks` array IN PLACE, and returns the SAME reference verbatim — no Proxy, no clone, no spread. The AI Agent's RunManager fires `handleChatModelStart`, the handler reserves via the sidecar UDS BEFORE the provider HTTP, on DENY throws `DecisionDenied` and the n8n loader surfaces it as `NodeApiError(httpCode: "403")` (428 for approval, 503 for sidecar unavailable, 502 for handshake failure); `handleLLMEnd` commits real `inputTokens + outputTokens`. Process-wide `SpendGuardClient` singleton cache keyed by `(tenantId, socketPath)`, bounded 16 with FIFO eviction + concurrent-call dedup + `beforeExit` close hook. `executionId` is the SpendGuard `sessionId`; default `runId` is `${executionId}:${nodeName}`; node-name and custom-expression overrides supported. Self-hosted n8n ≥ 1.50 only (CJS-only loader; community-node packages enabled flag); n8n Cloud blocked by UDS / FS policy. Pinned `@spendguard/sdk@0.1.0` + `@spendguard/langchain@0.1.0` so the `deriveIdempotencyKey` output is byte-deterministic per node install. | [`sdk/typescript-n8n/`](sdk/typescript-n8n/) · [`deploy/demo/n8n_real/`](deploy/demo/n8n_real/) · [`examples/n8n/`](examples/n8n/) |
| **Coze Studio (ByteDance, no-code)** | `examples/coze-studio/` (recipe — no SDK; Pattern 2 base-URL redirect) | Every Coze workspace chat-flow / agent / workflow LLM call routes through the SpendGuard sidecar HTTP companion (`/v1/openai/chat/completions`). Workspace **Model Provider → OpenAI** is wired to `base_url=https://<sidecar>:8443/v1/openai` with three custom headers — `X-SpendGuard-Tenant-Id` (workspace ID), `X-SpendGuard-Budget-Id`, `X-SpendGuard-Window-Instance-Id` — and mTLS material. Companion reserves pre-call, forwards to upstream OpenAI, commits real usage end-of-stream. Fail-closed by default (DENY → 502 surface in Coze workflow trace; no escape-hatch flag in v1 snippet). Self-hosted Coze only (Coze Cloud's gated config is v1.1 scope); OpenAI-compatible models only (Anthropic / Gemini / Bedrock slots v1.1). Native Coze plugin SDK route (Pattern 3) is v1.1 — Pattern 2 already covers 100% of Coze model calls. | [`examples/coze-studio`](examples/coze-studio/README.md) · [`deploy/demo/coze_studio`](deploy/demo/coze_studio/README.md) |
| **Cursor IDE MITM codec** **(EXPERIMENTAL — SOW only)** | `services/cursor_codec/` (Rust crate, gated by workspace feature `cursor-mitm-experimental`) | Reverse-engineered Connect-RPC codec for the Cursor IDE Agent's `api.cursor.sh` traffic. Every Cursor Agent chat call routes through the SpendGuard egress proxy → codec → translate to OpenAI canonical → sidecar reserve → upstream `api.cursor.sh` (re-encoded) → commit / release. Codec breaks whenever Cursor changes their wire protocol; Customer accepts break-window risk via signed [SOW addendum](services/cursor_codec/SOW.md). **DO NOT SHIP IN GA CONFIG.** | [`services/cursor_codec/`](services/cursor_codec/) · [SOW.md](services/cursor_codec/SOW.md) · [PROTOCOL.md](services/cursor_codec/PROTOCOL.md) |
| **Windsurf IDE MITM codec** **(EXPERIMENTAL — SOW only)** | `services/windsurf_codec/` (Rust crate, gated by workspace feature `windsurf-mitm-experimental` + env var `SPENDGUARD_EXPERIMENTAL_CODECS=1`) | Reverse-engineered gRPC-Web codec for the Windsurf IDE Cascade runtime's `server.codeium.com` / `windsurf-server.codeium.com` traffic. Every managed Cascade chat call routes through the SpendGuard egress proxy → codec → version gate → translate to OpenAI canonical → sidecar reserve → upstream Codeium (byte-perfect tee) → commit / release. Unknown wire versions fail closed (`windsurf_wire_version_unsupported`); known-version body failures degrade to `decoder_skipped` pass-through. Codec breaks whenever Codeium changes their wire protocol; Customer accepts break-window risk via signed [SOW addendum](services/windsurf_codec/SOW.md). **DO NOT SHIP IN GA CONFIG.** | [`services/windsurf_codec/`](services/windsurf_codec/) · [SOW.md](services/windsurf_codec/SOW.md) · [PROTOCOL.md](services/windsurf_codec/PROTOCOL.md) |
| **Subscription-tier meter** (Claude Code Pro + Codex on ChatGPT Plus) | `services/sidecar/src/subscription_meter/` (Rust module, additive proto `ReservationSource::SUBSCRIPTION_METER`) | Best-effort retail-USD meter for flat-fee subscription plans where SpendGuard cannot enforce the dollar (the vendor settles internally). Classifier requires BOTH `Authorization` prefix AND `User-Agent` to match (UA-only forgery rejected); meter-only path NEVER writes `ledger_entries` or `reservations`. Three modes: `meter` (default, audit-only), `soft_cap` (alert + CONTINUE), `hard_cap` (synthetic 429 with vendor-matched body shape — `code = spendguard_subscription_cap`). `Retry-After` clamped to ≤ 24 h. Day-2 reconciliation contract locked via [`subscription_importer`](services/ledger/src/subscription_importer/) stubs. | [Subscription meter docs](https://agenticspendguard.dev/docs/integrations/subscription-meter/) |
| **Devin billing importer** **(Cognition Labs — reconciliation only)** | `services/importer_devin/` (Rust crate, `publish=false`, `live` Cargo feature for HTTP client) | Post-hoc Devin Team API importer for [Archetype IV](docs/strategy/framework-coverage-2026-06.md) — Devin runs the agent loop entirely inside Cognition's cloud VM, so SpendGuard cannot gate Devin sessions. The importer pulls **ACU (Agent Compute Unit, ≈ $2.25/ACU)** consumption from `/api/v1/teams/{id}/usage`, converts via a vendored price table to estimated micro-USD (saturating arithmetic; rejects NaN / Inf / negative inputs), and emits signed `spendguard.audit.import.devin_acu` CloudEvents tagged `reservation_source=subscription_meter` / `import_source=devin_team_api`. Enterprise plans with negotiated rates emit `amount_micro_usd=NULL` + `reason_code=devin_enterprise_negotiated_rate`. `pricing_version` stamped per row; rate back-revisions never rewrite history. Default build is HTTPS-client-free (acceptance A2.4); `live` feature pulls rustls-only `reqwest` with typed 401/403/429/5xx errors that never log the bearer token. Idempotent via deterministic UUIDv5 `(team, session, window_end)`. | [`services/importer_devin/`](services/importer_devin/) · [Devin importer docs](https://agenticspendguard.dev/docs/integrations/devin-importer/) · [CloudEvent schema](docs/specs/coverage/D14_devin_importer/cloudevent-schema.md) |
| **Manus importer** **(Butterfly Effect, Meta-acquired 2026 — reconciliation only)** | `services/importer_manus/` (Rust crate, `publish=false`, `live` Cargo feature for HTTP client) | Post-hoc Manus admin REST importer for [Archetype IV](docs/strategy/framework-coverage-2026-06.md) — Manus runs each agent task entirely inside a vendor-managed VM, so SpendGuard cannot gate Manus sessions. The importer pulls **credit** consumption from `/v1/usage`, converts via a vendored TOML price table (`team_plan` $39/mo @ 1900 credits → 20_526 micro-USD/credit; `enterprise` operator-override default 0; `enterprise_byok` LOAD-BEARING 0 because BYOK customers pay LLM provider direct) to estimated micro-USD (integer saturating multiply; rejects negative credits; unknown tier → WARN+skip, never fabricated), and emits signed `spendguard.audit.import.manus_credit` CloudEvents tagged `reservation_source=subscription_meter` / `import_source=manus_team_api`. `pricing_version` stamped per row; rate back-revisions never rewrite history. Synthetic `model=manus.session/credit`, `input_tokens=output_tokens=0` (honest zero beats guessed tokens — Manus exposes no per-LLM-call detail). In-progress sessions filtered from the demo path per E3. Default build is HTTPS-client-free (acceptance A2.4); `live` feature pulls rustls-only `reqwest` with typed 401/403/429/5xx errors, bounded cursor pagination (10_000 page cap), and 30s per-request timeout — bearer token never logged in Display path. Idempotent via deterministic UUIDv5 `(workspace, session, window_end)` + vendor-prefixed `dedupe_key="manus:<session_id>"`. | [`services/importer_manus/`](services/importer_manus/) · [Manus importer docs](https://agenticspendguard.dev/docs/integrations/manus-importer/) · [D15 design](docs/specs/coverage/D15_manus_importer/design.md) |
| **Genspark billing importer** **(Super Agent — reconciliation only)** | `services/importer_genspark/` (Rust crate, `publish=false`, `live` Cargo feature for HTTP client) | Post-hoc Genspark admin-usage importer for [Archetype IV](docs/strategy/framework-coverage-2026-06.md) — Genspark Super Agent runs the agent loop entirely inside Genspark's cloud VM, so SpendGuard cannot gate Genspark sessions. The importer pulls **credit** consumption from `/v1/admin/usage`, converts via a vendored price table (Plus $19.99 @ 10k credits → $0.001999/credit; Pro $24.99 @ 12.5k → $0.0019992/credit; Premium $249.99 @ 125k → $0.00199992/credit) to estimated micro-USD (saturating arithmetic; rejects NaN / Inf / negative inputs), and emits signed `spendguard.audit.import.genspark_credit` CloudEvents tagged `reservation_source=subscription_meter` / `import_source=genspark_team_api`. Unknown plan slugs emit `amount_micro_usd=0` + `reason_code=genspark_plan_unknown` (BOTH fields set; dashboard distinguishes "unknown rate" from "zero spend"). `pricing_version` stamped per row; rate back-revisions never rewrite history. Default build is HTTPS-client-free; `live` feature pulls rustls-only `reqwest` with typed 401/403/429/5xx errors plus a three-way token gate (missing / empty / too-short < 32 chars) that never logs the bearer token. Idempotent via deterministic UUIDv5 `(workspace, task, window_end)`. | [`services/importer_genspark/`](services/importer_genspark/) · [Genspark importer docs](https://agenticspendguard.dev/docs/integrations/genspark-importer/) · [D16 design](docs/specs/coverage/D16_genspark_importer/design.md) |
| **Anthropic `claude-agent-sdk` (Python + TS)** | _no SDK adapter; egress-proxy recipe via D02 root-CA install_ | Anthropic's first-party agent SDK subprocesses the `claude` CLI; LLM calls leave the process from inside the CLI, not from the SDK. `PreToolUse` is **tool-scope, not LLM-scope** — registering a budget cap there is a lie. The honest gate is the SpendGuard egress proxy: D02 `spendguard install` trusts the SpendGuard root CA on the host and sets `HTTPS_PROXY` + `NODE_EXTRA_CA_CERTS`, the CLI inherits them, and `api.anthropic.com/v1/messages` flows through the proxy's existing Anthropic route → one `RESERVE_RESPONSE` + one matching `COMMIT_OUTCOME` per call in `audit_outbox`. BYOK only — Claude Code Pro / Max sits behind the [subscription meter](https://agenticspendguard.dev/docs/integrations/subscription-meter/) | [`examples/claude-agent-sdk/`](examples/claude-agent-sdk/) · [claude-agent-sdk docs](https://agenticspendguard.dev/docs/integrations/claude-agent-sdk/) |
| **AnythingLLM** (Mintplex Labs — Generic OpenAI provider) | _no SDK; Pattern 2 — Generic OpenAI provider Base URL_ | AnythingLLM 1.8+ ships a **Generic OpenAI** provider tile under Settings → LLM Preference; setting its Base URL to a SpendGuard egress proxy routes every Workspace chat through the SpendGuard pre-call budget gate and KMS-signed audit chain, with no AnythingLLM code change, no fork, and no plugin install. Verified end-to-end via `DEMO_MODE=anythingllm_real` (boots pinned `mintplexlabs/anythingllm:1.8.4` + smoke driver that configures the provider via `/api/v1/system/update-env`, sends one Workspace chat, asserts one `reserve` row and one `commit_estimated` row in the audit chain). Streaming SSE supported (egress proxy injects `stream_options.include_usage=true` and commits on the terminating `[DONE]` event); the inbound `Authorization` header is substituted server-side so the AnythingLLM-side API Key can be any non-empty string. Desktop is GUI-only (no admin API; manual configuration only); Cloud requires a non-`localhost` SpendGuard URL (Helm chart). | [`examples/anythingllm/`](examples/anythingllm/) · [AnythingLLM recipe](https://agenticspendguard.dev/docs/drop-in/anythingllm/) |
| **LobeChat** (LobeHub — `OPENAI_PROXY_URL` env var) | _no SDK; Pattern 2 — `OPENAI_PROXY_URL` container env var_ | LobeChat 1.40+ honours `OPENAI_PROXY_URL` at container boot and rewrites the OpenAI upstream for every server-side chat at `/api/chat/openai`. Setting `OPENAI_PROXY_URL=http://egress-proxy:9000/v1` puts every server-mode LobeChat chat in SpendGuard's pre-call budget gate and KMS-signed audit chain, with no LobeChat code change, no fork, and no plugin install. Verified end-to-end via `DEMO_MODE=lobechat_real` (boots pinned `lobehub/lobe-chat:1.40.0` + smoke driver that sends one chat through `/api/chat/openai`, asserts one `reserve` row and one `commit_estimated` row in the audit chain). Streaming SSE supported (egress proxy injects `stream_options.include_usage=true` and commits on the terminating `[DONE]` event); the inbound `Authorization` header is substituted server-side so the LobeChat-side `OPENAI_API_KEY` can be any non-empty string. Client mode (browser keys) bypasses the env var — use the per-session UI override at **Settings → Language Model → OpenAI → API Proxy Address**; Cloud (lobechat.com) requires a non-`localhost` SpendGuard URL (Helm chart). | [`examples/lobechat/`](examples/lobechat/) · [LobeChat recipe](https://agenticspendguard.dev/docs/drop-in/lobechat/) |
| **Drop-in (14 tools)** | _no SDK; Pattern 2 env-var redirect_ | Every OpenAI-compatible base URL tool — drop in SpendGuard in 30 seconds, one env var | [Drop-in landing](https://agenticspendguard.dev/docs/drop-in/) |

---

## 🌐 Other demo modes

```bash
make demo-up DEMO_MODE=decision               # CONTINUE flow
make demo-up DEMO_MODE=deny                   # hard-cap → STOP
make demo-up DEMO_MODE=approval               # REQUIRE_APPROVAL → resume()
make demo-up DEMO_MODE=ttl_sweep              # reservation TTL release
make demo-up DEMO_MODE=agent_real             # real OpenAI via Pydantic-AI
make demo-up DEMO_MODE=agent_real_anthropic   # real Anthropic
make demo-up DEMO_MODE=agent_real_langgraph   # LangGraph
make demo-up DEMO_MODE=agent_real_openai_agents          # OpenAI Agents SDK (wrapper)
make demo-up DEMO_MODE=agent_real_openai_agents_proxy    # openai-agents via proxy ⭐
make demo-up DEMO_MODE=agent_real_adk                    # Google ADK (Python) ⭐ D19
make demo-up DEMO_MODE=agent_real_strands                # AWS Strands (Python) ⭐ D20
make demo-up DEMO_MODE=agent_real_strands_deny           # AWS Strands DENY proof
make demo-up DEMO_MODE=agent_real_dspy                   # DSPy (Python) ⭐ D21
make demo-up DEMO_MODE=agent_real_agno                   # Agno (Python) ⭐ D22
make demo-up DEMO_MODE=agent_real_beeai                  # BeeAI Framework (Python) ⭐ D23
make demo-up DEMO_MODE=agent_real_autogen                # AutoGen 0.4+ (Python) ⭐ D24
make demo-up DEMO_MODE=agent_real_ag2                    # AG2 lineage (same wrapper) ⭐ D24
make demo-up DEMO_MODE=agent_real_smolagents             # SmolAgents (Python) ⭐ D25
make demo-up DEMO_MODE=agent_real_letta                  # Letta (Python — library mode) ⭐ D26
make demo-up DEMO_MODE=agent_real_llamaindex             # LlamaIndex (Python) ⭐ D27
make demo-up DEMO_MODE=agent_real_llamaindex_stub        # LlamaIndex (Python) — MockLLM CI gate ⭐ D27
make demo-up DEMO_MODE=agent_real_atomic_agents          # Atomic Agents + Instructor (Python) ⭐ D28
make demo-up DEMO_MODE=litellm_real           # LiteLLM proxy: ALLOW+DENY+STREAM+MULTI-TEAM ⭐
make demo-up DEMO_MODE=litellm_deny           # LiteLLM proxy: 3 fail-closed sub-steps
make demo-up DEMO_MODE=langchain_ts           # LangChain.js: ALLOW+DENY+STREAM
make demo-up DEMO_MODE=vercel_ai_mastra       # Vercel AI SDK (covers Mastra): ALLOW+DENY+STREAM ⭐
make demo-up DEMO_MODE=inngest_agent_kit      # Inngest AgentKit: ALLOW+DENY+RETRY_DEDUP ⭐
make demo-up DEMO_MODE=maf_dotnet_real        # Microsoft Agent Framework .NET: ALLOW+DENY+ALLOW2 ⭐
make demo-up DEMO_MODE=maf_python_real        # Microsoft Agent Framework Python: ALLOW+DENY+ALLOW2 ⭐
make demo-up DEMO_MODE=dify_plugin_real       # Dify Model Provider Plugin: ALLOW+DENY+STREAM ⭐
make demo-up DEMO_MODE=botpress_real          # Botpress Integration SDK: ALLOW+DENY+STREAM ⭐ D32
make demo-up DEMO_MODE=flowise_real           # Flowise custom node: ALLOW+DENY+STREAM ⭐ D35
make demo-up DEMO_MODE=langflow_real          # Langflow custom component: ALLOW+DENY+STREAM ⭐ D36
make demo-up DEMO_MODE=n8n_real               # n8n community node: ALLOW+DENY+STREAM ⭐ D37
make demo-up DEMO_MODE=coze_studio_real       # Coze Studio (ByteDance, no-code): ALLOW+DENY+STREAM ⭐ D31
make demo-up DEMO_MODE=anythingllm_real       # AnythingLLM Generic OpenAI provider: ALLOW round-trip ⭐ D33
make demo-up DEMO_MODE=lobechat_real          # LobeChat OPENAI_PROXY_URL: ALLOW round-trip ⭐ D34
make demo-up DEMO_MODE=litellm_sdk_real       # LiteLLM SDK shim: ALLOW+STREAM+TRANSITIVE/CrewAI ⭐
make demo-up DEMO_MODE=litellm_sdk_deny       # LiteLLM SDK shim: 3 fail-closed sub-steps
make demo-up DEMO_MODE=cursor_mitm_fixture    # Cursor MITM codec fixture replay (EXPERIMENTAL — SOW only)
make demo-up DEMO_MODE=subscription_meter     # Claude Code Pro + Codex on ChatGPT Plus — meter / soft_cap / hard_cap (synthetic 429) ⭐ D13
make demo-up DEMO_MODE=maf_python_with_agt    # MAF + AGT coexistence smoke
make demo-up DEMO_MODE=approval_hot_reload    # frozen-pricing regression
make demo-up DEMO_MODE=multi_provider_usd     # multi-provider USD normalization
```

`make demo-up` (no flag) spins up the full wrapper-mode stack including the dashboard at `http://localhost:8090`.

---

## ❓ FAQ

<details>
<summary><b>How does this compare to Helicone / Portkey / LiteLLM?</b></summary>

Those proxy your traffic too, but their decision model is **observability**: log the call, then alert / route / retry. SpendGuard's decision model is **auth/capture**: reserve PRE the call, fail-closed on overrun, commit POST. The audit chain isn't a log — it's a tamper-evident ledger backed by Postgres immutability triggers + KMS-signed CloudEvents.

If you only need a per-key dollar cap on a gateway, Portkey or LiteLLM is simpler. SpendGuard is for anyone who needs the LLM call **refused** the moment the budget is gone — whether that's a 1-person SaaS protecting a free tier, or a platform team that also has to hand evidence to compliance after the bill lands.
</details>

<details>
<summary><b>What about latency?</b></summary>

The proxy adds one UDS gRPC roundtrip to the sidecar PRE the call (~1–3ms on the same pod) + one async EmitTraceEvents POST the call (doesn't block the response). The audit-chain write is async via outbox.
</details>

<details>
<summary><b>Does the agent's code need to change?</b></summary>

For the proxy path (Chat Completions + Responses API): **no**. One environment variable. The verified clients listed above all work without any code changes.

For the SDK wrapper-mode (approval workflows / model degradation): yes — but it's typically one line of "wrap the model object" inside your framework. See the integrations table above.
</details>

<details>
<summary><b>What about agents that import the OpenAI client directly and skip the proxy?</b></summary>

That's the L1 → L2 → L3 trust model. L1 (SDK wrapper) blocks via the framework. L2 (`egress_proxy_hard_block`) blocks at the HTTP layer + a Kubernetes NetworkPolicy that forbids egress except via the proxy. L3 (`provider_key_gateway`, future) keeps the provider API key entirely server-side so the agent process can't make calls at all without the gateway.
</details>

<details>
<summary><b>How does the audit chain prevent tampering?</b></summary>

Three layers: (1) `audit_outbox` table has a Postgres trigger refusing any `UPDATE` or `DELETE`; (2) every row carries an Ed25519 or KMS-ECDSA-P256 signature over a canonical hash; (3) `canonical_ingest` verifies signatures at ingest time and quarantines failed verifications. Any tampering fails at the DB layer, the signature layer, or the ingest layer.
</details>

<details>
<summary><b>What's the Phase 1 ledger constraint?</b></summary>

`single_writer_per_budget` only. A given budget can be written by exactly one workload instance at a time, enforced via fencing leases. Multi-region writers come in Phase 2.
</details>

<details>
<summary><b>Why Rust?</b></summary>

Zero-GC in the hot path (the sidecar is in the request-path for every LLM call). `tonic` + `axum` compose cleanly. The team had ~6 months of existing Rust ledger code when the proxy work started.
</details>

---

## 🔌 Service map

| Service | What it does | Port |
|---|---|---:|
| [`ledger`](services/ledger/) | Postgres-backed double-entry ledger + audit transactional outbox | 50051 |
| [`sidecar`](services/sidecar/) | Per-pod UDS gRPC server; contract evaluator; mTLS clients | (UDS) |
| [`canonical_ingest`](services/canonical_ingest/) | Per-decision_id canonical ordering + 3 storage classes | 50052 |
| [`egress_proxy`](services/egress_proxy/) | HTTP proxy for `/v1/chat/completions` + `/v1/responses` (1-env-var) | 9000 |
| [`control_plane`](services/control_plane/) | REST API for tenants / budgets / approvals | 8091 |
| [`dashboard`](services/dashboard/) | Read-only operator UI (budgets / decisions / audit export) | 8090 |
| [`outbox_forwarder`](services/outbox_forwarder/) | Closes the audit-chain loop (ledger → canonical_ingest) | — |
| [`ttl_sweeper`](services/ttl_sweeper/) | Releases expired reservations | — |
| [`webhook_receiver`](services/webhook_receiver/) | Provider HTTPS webhooks → Ledger gRPC ops (HMAC-verified) | 8443 |
| [`usage_poller`](services/usage_poller/) | OpenAI / Anthropic admin-usage API → `provider_usage_records` | — |
| [`signing`](services/signing/) | Producer signing trait (Local Ed25519 + KMS verifier) | — |

Every external surface is mTLS. Every service exposes `/metrics` (Prometheus, per-handler ok/err counters). Every audit row is signed.

---

## 🚀 Deploy

**Docker Compose (demo / local dev):** [`deploy/demo/compose.yaml`](deploy/demo/compose.yaml) — full stack with PKI bootstrap, manifest signing, mTLS internal, all on one network.

**Kubernetes (Helm):** [`charts/spendguard/`](charts/spendguard/) — DaemonSet sidecar + Deployments for ledger / canonical_ingest / control_plane / dashboard / webhook_receiver. `chart.profile=production` enforces required-input gates (bundle hashes, trust-root SPKI, real Postgres URL) at template render time. Validated end-to-end on `kind` via [`scripts/helm-validate-kind.sh`](scripts/helm-validate-kind.sh) (CI: [`.github/workflows/helm-validate.yml`](.github/workflows/helm-validate.yml)).

**Signing modes:**
- `local` — Ed25519 PKCS8 PEM mounted from K8s Secret (demo / on-prem)
- `kms` — AWS KMS-backed ECDSA P-256 via IRSA (production)
- `disabled` — empty signatures (refuses to construct outside `SPENDGUARD_PROFILE=demo`)

---

## 📚 Specs (source of truth)

Read before changing wire format or invariants:

- [`docs/agent-runtime-spend-guardrails-complete.md`](docs/agent-runtime-spend-guardrails-complete.md) — full design doc
- [`docs/trace-schema-spec-v1alpha1.md`](docs/trace-schema-spec-v1alpha1.md) — CloudEvent / audit chain
- [`docs/ledger-storage-spec-v1alpha1.md`](docs/ledger-storage-spec-v1alpha1.md) — double-entry model, idempotency, replay
- [`docs/contract-dsl-spec-v1alpha1.md`](docs/contract-dsl-spec-v1alpha1.md) — Contract DSL + decision boundary semantics
- [`docs/sidecar-architecture-spec-v1alpha1.md`](docs/sidecar-architecture-spec-v1alpha1.md) — fencing, drain, capability handshake
- [`docs/stage2-poc-topology-spec-v1alpha1.md`](docs/stage2-poc-topology-spec-v1alpha1.md) — Phase 1 SaaS topology + durability invariants

All locked at v1alpha1; schema bumps land via additive proto changes (backwards-compatible).

---

## 🤝 Contributing

**Honest status:** Dev Status 4-Beta. Single-maintainer open-source project (Apache 2.0). Solid demo coverage (8+ demo modes, all green) and a signed audit chain — but zero production users yet. PyPI 0.3.0 + Microsoft AGT integration merged 2026-05-19 are the only third-party validation signals. PRs welcome; the wire spec + audit invariants are append-only — open an issue first if you're about to touch `proto/` or `migrations/`.

---

## 📄 License

[Apache 2.0](LICENSE)

## Third-Party Tokenizer Notices

SpendGuard vendors tokenizer assets for predictor validation. The Llama
tokenizer path uses Meta Llama 3.1-derived tokenizer files and is
`Built with Llama`; review
[`crates/spendguard-tokenizer/LICENSE_NOTICES.md`](crates/spendguard-tokenizer/LICENSE_NOTICES.md)
for attribution, the 700 million monthly active users threshold measured
in the calendar month before the Llama 3.1 release date (2024-07-23),
and Meta Llama 3.1 Acceptable Use Policy obligations before
redistributing or enabling that path in a product.
