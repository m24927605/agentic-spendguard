# Agent Spend Guard — Framework Coverage Strategy

**Date:** 2026-06-06
**Owner:** Michael Chen
**Status:** Internal strategy memo — informs roadmap planning, not yet committed scope.

## TL;DR

The LLM observability / gateway layer was reshuffled in the last 90 days:

- **Cloudflare AI Gateway Spend Limits** — GA 2026-06-05 (one day ago).
- **Databricks Unity AI Gateway AI Spend Controls** — GA 2026-05-19.
- **Portkey** — acquired by Palo Alto Networks 2026-04-30.
- **Helicone** — acquired by Mintlify 2026-03, now maintenance mode.
- **Langfuse** — acquired by ClickHouse 2026-01.
- **Microsoft Agent Framework 1.0** — GA 2026-04.
- **AWS Strands Agents SDK** — GA 2026-04.
- **GitHub Copilot CLI BYOK** (`COPILOT_PROVIDER_BASE_URL`) — GA 2026-04-07.

"Spend cap" as a headline feature is now mainstream commodity. SpendGuard's wedge must be these three:

1. **Predictive, not reactive cumulative.** Every incumbent gates "after $X spent." SpendGuard reserves against budget *before* dispatch using a token-cost projection, and the reservation can be released if the call fails.
2. **Cross-provider, cross-platform.** Cloudflare can only see traffic through Cloudflare. Databricks can only see Databricks endpoints. SpendGuard is the only product that crosses every boundary — agent framework SDK calls, gateway proxy traffic, closed CLI binaries, no-code platforms.
3. **ASP standard wire format.** The Agent Spend Protocol Draft-01 + the aeoess crosswalk PRs #99 / #105 (merged) give us a public wire format no incumbent has. This is the long-term moat.

"Full framework support" is achievable, but not as an N×1 SDK-per-framework matrix. It is the Cartesian product of **3 integration patterns × 4 ecosystem categories**.

---

## The four ecosystem categories

| Category | Examples | Integration difficulty | SpendGuard customer? |
|----------|----------|------------------------|----------------------|
| **A. Programmatic agent frameworks** | LangChain, LangGraph, CrewAI, AutoGen/AG2, Microsoft Agent Framework (MAF), OpenAI Agents SDK, Anthropic claude-agent-sdk, Pydantic AI, Vercel AI SDK + Mastra, LlamaIndex, SmolAgents, Agno, DSPy, Letta, AWS Strands, Google ADK, BeeAI, Inngest AgentKit | Medium — most expose a `Model` abstraction or callback bus | Yes — primary ICP |
| **B. LLM gateways / proxies** | LiteLLM, OpenRouter, Portkey, Cloudflare AI Gateway, Envoy AI Gateway, Kong AI Gateway, Apigee, Traefik Hub, Bedrock, Vertex AI, Azure AI Foundry, Databricks Unity AI Gateway | High — competitor + integrator mixed | Depends — channels vs. competitors |
| **C. Closed coding CLI / IDE agents** | Claude Code, Codex, Gemini CLI, Aider, Cursor, Continue, Cline, Roo Code, OpenHands, Goose, Amazon Q Developer, GitHub Copilot CLI, Devin, Trae, Manus, Genspark, Cody, Tabnine, Windsurf, Augment, Zed | High — end user does not write code, no SDK hook point | Yes — primary source of enterprise spend leak |
| **D. No-code / visual agent builders** | Dify, Coze Studio, Botpress, AnythingLLM, LobeChat, Flowise, Langflow, n8n, Make, Zapier Agents, Stack AI, Lyzr, Voiceflow, Retool Agents, BuildShip, Relevance AI | Medium — OSS self-host = high ROI, SaaS-only = locked out | Partial — some unreachable |

---

## The three integration patterns

The ecosystem reduces to three hook points. Every "support framework X" task collapses into one of these:

### Pattern 1 — Model-abstraction middleware (in-process)

**How it works:** The framework already has a `Model` / `LLM` / `ChatClient` abstraction. SpendGuard registers a callback, middleware, or wrapper that intercepts `messages → response`.

**Coverage:**

| Framework | Hook |
|-----------|------|
| LangChain | `BaseCallbackHandler.on_chat_model_start` |
| LangGraph | Inherits LangChain |
| Vercel AI SDK | `wrapLanguageModel({ middleware })` |
| Mastra | Inherits Vercel AI SDK |
| OpenAI Agents SDK | `Model` subclass + `TracingProcessor` |
| Microsoft Agent Framework | Middleware pipeline (GA 2026-04) |
| Semantic Kernel | `IPromptRenderFilter` |
| Google ADK | `before_model_callback` |
| AWS Strands | `HookProvider.before_invocation` (GA 2026-04) |
| Pydantic AI | OTel `SpanProcessor` (until #4561 lands) |
| DSPy | `BaseCallback.on_lm_start` |
| Agno | `pre_hooks` |
| BeeAI | `Emitter` subscribe |
| SmolAgents | `step_callbacks` (coarse-grained) |
| Inngest AgentKit | `step.ai.wrap()` |

**SDK package design:**

- `spendguard-langchain` (Python + TS)
- `spendguard-vercel-ai` (TS) — also covers Mastra
- `spendguard-openai-agents` (Python + TS)
- `spendguard-maf` (Python + .NET) — wedge via existing PR #2398
- `spendguard-google-adk`
- `spendguard-strands`
- `spendguard-pydantic-ai`
- `spendguard-dspy`

Each package is a thin shim. Core reservation logic (`reserve → call → commit_estimated / release`) lives in `spendguard-core`. Each adapter only translates the framework's `messages` into a `prompt_hash` and the framework's `response` into a commit payload.

### Pattern 2 — Proxy redirect via OpenAI-compatible base URL (zero-code-change)

**How it works:** User sets one environment variable. SpendGuard runs an OpenAI-compatible (or Anthropic-compatible) endpoint, forwards to the real provider.

**Coverage:**

| Tool | Setting |
|------|---------|
| LiteLLM (proxy mode) | `OPENAI_API_BASE` |
| Aider | `OPENAI_API_BASE` |
| Continue | `apiBase` in `config.yaml` |
| Cline / Roo Code (BYOK) | Custom OpenAI provider |
| OpenHands (BYOK) | LLM custom endpoint |
| Goose | `OPENAI_HOST` (native) |
| Zed AI | OpenAI-compatible `api_url` (native) |
| GitHub Copilot CLI (BYOK, 2026-04-07) | `COPILOT_PROVIDER_BASE_URL` |
| Tabnine Enterprise | BYO LLM endpoint |
| AnythingLLM | Custom OpenAI-compatible base URL |
| LobeChat | Custom base URL (native) |
| Cody self-hosted Enterprise | Sourcegraph relay endpoint |
| Augment (BYOK) | LLM custom endpoint |
| Dify (Model Provider Plugin) | Custom provider plugin |

This is the lowest-friction lane. Should be packaged as a one-page **"drop-in install"** landing page covering all 14 tools.

### Pattern 3 — Egress proxy + self-signed CA install (forward proxy)

**How it works:** SpendGuard installs a root CA into the OS keychain. SpendGuard sidecar runs an HTTPS proxy (or is set as `HTTPS_PROXY`). All traffic to `api.openai.com`, `api.anthropic.com`, `generativelanguage.googleapis.com` flows through it.

**Coverage:**

| Tool | Mechanism |
|------|-----------|
| Claude Code CLI (BYOK) | `HTTPS_PROXY` + `NODE_EXTRA_CA_CERTS` |
| Codex CLI (BYOK) | `CODEX_CA_CERTIFICATE` (native) |
| Gemini CLI (API key / Vertex) | Standard proxy env |
| Anthropic claude-agent-sdk | Spawns `claude` CLI subprocess — egress proxy is the only LLM-scope gate |
| Any framework + `HTTPS_PROXY` | Standard |

This is the workhorse for closed CLIs. Pattern 3 is the only path that gates the `claude-agent-sdk` because its `PreToolUse` hook is tool-scope, not LLM-scope.

---

## Closed CLI deep dive

21 tools fall into 5 archetypes:

### Archetype I — BYOK + standard `HTTPS_PROXY` (green light, mainstream)

12–14 tools: Claude Code BYOK, Codex BYOK, Gemini API-key / Vertex, Aider, Continue, Cline / Roo BYOK, OpenHands, Goose, Amazon Q (v1.8+), Copilot BYOK, Cody self-hosted, Tabnine Enterprise, Augment BYOK, Zed.

**Support mechanism:** Pattern 3 (egress proxy + CA) or Pattern 2 (base URL swap), depending on which the tool supports.

**MVP shape:** A single `spendguard install` script does two things: (a) install root CA into the OS keychain; (b) write a shell rc that sets `HTTPS_PROXY=http://localhost:8443`. All BYOK CLIs are protected with one install.

### Archetype II — Subscription quota (yellow light, meter-only)

Claude Code Pro/Max ($20–$200/month), Codex CLI on ChatGPT Plus/Pro, Cody Cloud, GitHub Copilot CLI managed.

Technically the traffic is visible at the proxy — it still goes to `api.anthropic.com` or `chatgpt.com/backend-api/codex/responses` — but the vendor settles the quota internally. SpendGuard sees tokens, never sees the dollar equivalent.

**Support mechanisms:**

- **Meter mode:** Use the tokenizer service + the price table to estimate cost. Produce audit events, alerts, and soft caps. Slots cleanly into the existing 17 prediction columns.
- **Hard cap (degraded UX):** When the budget is exhausted, SpendGuard proxy returns synthetic 429 / `unavailable` errors and the CLI treats them as provider errors and stops. Works, but coarse.
- **Out-of-band:** When vendor APIs open (Anthropic Console usage API, ChatGPT admin API), import billing for post-hoc reconciliation.

### Archetype III — Proprietary on-device protocol (red-yellow, requires reverse engineering)

Cursor Agent, Windsurf managed Cascade.

These do not call `api.openai.com`. They call their own `api.cursor.sh` / `windsurf-server` over a private Connect-RPC or proprietary wire format.

**Support mechanism:** MITM CA + protocol translator. Community PoCs prove it works (`cursor-byok`, `windsurf-proxy`), but **someone has to maintain a reverse-engineered codec**. Vendor protocol changes break it.

**Recommendation:** Tier 3. Enterprise SOW only, not a GA feature. Customer accepts codec-break risk.

### Archetype IV — Fully managed cloud agent (unreachable)

Devin (Cognition), Manus (Butterfly Effect / Meta), Genspark Super Agent.

Execution happens inside the vendor's cloud VM. The client sees a task result and an ACU / credit usage number, never a per-LLM-call payload.

**Support mechanism:** Post-hoc billing import. SpendGuard runs `spendguard-importer-{devin,manus,genspark}` collectors against vendor admin APIs, converts usage records into synthetic audit events, surfaces them on the dashboard alongside everything else. **Reconciliation only, no gating.**

Required for CIO / CFO "single pane of glass" use case. Not for hard-gate use cases.

### Archetype V — Legal red line

**Gemini CLI free OAuth tier:** Google publicly banned OAuth-token proxying in 2026-02 (enforcement from 2026-03-25). SpendGuard cannot support this path. Documentation must explicitly steer customers to Vertex AI or Gemini API key mode, not the free Code Assist OAuth flow.

---

## Competitive map

### Direct competitors on spend gating

| Competitor | Coverage | Differentiation we ship against |
|------------|----------|--------------------------------|
| Cloudflare AI Gateway Spend Limits (GA 2026-06-05) | Any LLM call routed through Cloudflare | Reactive cumulative; cannot see non-Cloudflare traffic; no ASP wire format |
| Databricks Unity AI Gateway AI Spend Controls (GA 2026-05-19) | All model serving endpoints in Databricks | Locked to Databricks estate |
| Google Vertex AI Spend Caps (preview, Next '26) | Vertex AI only | Google-only |
| Portkey (PANW acquisition 2026-04-30) | Cross-provider gateway + virtual keys + budgets | Security framing now dominant under PANW; integration velocity will slow during merger — 6–12 month window |
| AWS Bedrock Guardrails + Cost Mgmt | Bedrock customers | Not predictive; not cross-cloud |
| Stack AI / Lyzr Agent Studio | Built-in "LLM Provider Governance" inside their no-code builder | They compete inside their builder; nobody covers BYOK CLI + agent framework |

The slot that is **architecturally empty**: cross-provider, cross-platform, pre-dispatch reservation-based budget gating, with a public wire format. No competitor is in it today. 6–12 months is the window before Cloudflare or Portkey can reach across walled gardens.

### Should integrate, not compete

| Target | Why |
|--------|-----|
| **Envoy AI Gateway** (CNCF, v0.6 prod-ready 2026-05) | **#1 priority.** ExtProc sidecar pattern maps 1:1 to existing SpendGuard UDS+mTLS architecture. Their docs already explain token counting via ExtProc. Ship before Cloudflare extends Spend Limits to cover ExtProc callouts. |
| Kong AI Gateway | First-class Lua/Go plugin SDK. Enterprise API governance distribution channel. |
| Apigee Extension Processor (GA) | Callout-to-sidecar pattern fits SpendGuard architecture. Google distribution into Vertex customers. |
| Traefik Hub AI Gateway | Middleware plugin model is clean. They cover LLM Guard + token rate limit, lack predictive $ budget — we complete the surface. |
| LiteLLM proxy mode | Existing work. Ship the `async_pre_call_hook` guardrail plugin. |
| Langfuse / ClickHouse | Different slot (observability). Export SpendGuard CloudEvents to it for historical analysis. |

### Watch but don't invest

- **Portkey** — competitor now under PANW, but their `/plugins` model is open. Skip unless co-marketing opportunity surfaces.
- **Helicone** — maintenance mode. Skip.
- **Vellum / Martian / NotDiamond** — different architectural slot.
- **Pangea (content security) / Pomerium (identity)** — different layers. Can pair via documentation "chain-with" recipes.

### LiteLLM SDK gap

Confirmed status as of 2026-06: LiteLLM Issue #8842 is **still open from 14 months ago**. `async_pre_call_hook` remains "proxy only." `CustomLogger` is post-fact on the SDK path. The DESIGN.md §3.4 blocker on `feat/litellm-100` Slice 1 is real and persistent. PR #15636 is narrow scope and does not close the gap.

**Recommended approach — parallel tracks:**

- **(a) Proxy guardrail plugin ships now** via `async_pre_call_hook`. Covers LiteLLM proxy customers.
- **(b) SDK monkey-patch shim ships in parallel** — `spendguard-litellm-shim` patches `litellm.acompletion` so direct SDK callers are also covered.
- **(c) Upstream PR re-opened** with ASP wire format as the value proposition.

---

## Priority tiers

### Tier 1 — Ship now (next 30 days)

| Item | Rationale |
|------|-----------|
| Envoy AI Gateway ExtProc sidecar | v0.6 prod-ready one month, CNCF distribution; Cloudflare will catch up — window is now |
| LangChain `BaseCallbackHandler` adapter (Python + TS) | 130k stars, biggest reach per engineering hour |
| Closed CLI Pattern 3 install script + CA bootstrap | One script protects Claude Code / Codex / Aider / Goose / Zed / Continue / Cline (all BYOK) |
| `OPENAI_BASE_URL` drop-in landing page | 14 tools listed on one page, banner marketing feature |

### Tier 2 — 90 days

| Item | Rationale |
|------|-----------|
| Vercel AI SDK `wrapLanguageModel` middleware | TS standard, covers Mastra in the same package |
| Microsoft Agent Framework middleware | MAF 1.0 GA two months old; enterprise .NET channel |
| OpenAI Agents SDK `Model` wrap | OpenAI distribution + existing cookbook PR #2722 wedge |
| Kong AI Gateway plugin | Enterprise API gateway customers |
| Dify Model Provider Plugin | Largest no-code SaaS alternative |
| LiteLLM `async_pre_call_hook` proxy plugin + SDK monkey-patch shim | Unblocks Slice 1; unlocks CrewAI / DSPy / SmolAgents / Strands / BeeAI long tail |

### Tier 3 — Backlog

| Item | Rationale |
|------|-----------|
| Subscription-tier meter mode (Claude Code Pro, Codex ChatGPT-OAuth) | Yellow light, but enterprises ask for it |
| Devin / Manus / Genspark billing importer | Black-hole platforms, but CIO / CFO pane-of-glass needs them |
| Cursor / Windsurf MITM codec | SOW only, not GA |
| Google ADK, AWS Strands, Pydantic AI, DSPy, Agno, BeeAI adapters | Most covered transitively via LiteLLM; per-SDK adapters as customer demand warrants |
| Coze, Botpress, AnythingLLM, LobeChat, Flowise, Langflow, n8n custom nodes | No-code completion |

### Don't do

- Make / Zapier Agents / Voiceflow non-Enterprise — SaaS locked
- Manus / Genspark agent intercept — architecturally unreachable
- Gemini OAuth proxy — legal red line
- Stack AI / Lyzr — direct competitors in adjacent space, not integration targets

---

## Strategic readiness items

### ASP Draft-02 scope expansion

Add a "Closed Agent / CLI Binding" section to ASP Draft-02. Specifies the `Authorization-Forwarded-By: spendguard` header and the audit event shape for Pattern 2 / Pattern 3 customers. This is a layer Cloudflare / Portkey cannot easily reach into — the wire format becomes part of the moat. Coordinate with aeoess so the crosswalk also covers it.

### Public comparison table

Ship a public landing page comparing SpendGuard to Cloudflare AI Gateway Spend Limits (GA'd one day before this memo). Explicit framing: **predictive reservation, cross-platform, ASP-standard** vs. **Cloudflare-only, cumulative**. Cloudflare's GA is one day fresh; the framing window closes quickly as their messaging settles.

### LiteLLM upstream re-engagement

Open a new PR against BerriAI/litellm with the ASP wire format as the value proposition. Frame it as "join a public spec" rather than "fix our blocker." Lower social cost on the maintainer.

---

## Background research

This memo synthesizes parallel landscape research run 2026-06-06 across four ecosystem axes (programmatic agent frameworks, LLM gateways, closed CLIs, no-code builders). Source citations and per-framework hook details live in the research transcripts (not committed — large JSONL, retained out-of-tree).
