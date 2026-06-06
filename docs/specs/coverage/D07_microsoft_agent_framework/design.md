# D07 — Microsoft Agent Framework (MAF) middleware — design.md

> Status: Proposed (doc-first; no code lands until all 5 specs accepted).
> Owner: Backend Architect.
> Related: `sdk/python/src/spendguard/integrations/agt.py` (different product; complements this deliverable), `sdk/python/src/spendguard/integrations/openai_agents.py` (closest Python shape precedent), `docs/specs/litellm-integration/*` (spec set precedent), `proto/spendguard/sidecar_adapter/v1/adapter.proto` (sidecar IPC).

## 1. Problem & users

### 1.1 Who adopts this

Microsoft Agent Framework (MAF) v1.0 GA'd 2026-04 as the unified successor to Semantic Kernel (SK) + AutoGen; both upstream lines are now maintenance-only. Adopter shapes:

- **.NET enterprise teams.** ASP.NET Core / .NET 8+ services, DI-hosted `IChatClient` (Microsoft.Extensions.AI) wrapped by `AgentBuilder`.
- **Python platform teams.** `agent_framework` package (Python ≥3.11), same contract via `AgentBuilder().use_middleware(...)` + `@function_middleware`.

### 1.2 The gap MAF leaves

MAF's middleware pipeline (inherited from SK's `IPromptRenderFilter` + `IFunctionInvocationFilter`) is observational + transform only. Nothing in the framework gates on dollar spend, ledger reservations, or signed audit. Customers bolt on Azure OpenAI quota or homegrown counters — both reactive and race-prone. No predictive reservation, no cross-tenant audit chain.

| Gap | MAF today | SpendGuard adds |
|---|---|---|
| Pre-dispatch dollar gate | None (only Azure quota post-call) | `LLM_CALL_PRE` reserve via sidecar |
| Multi-instance correctness | Per-process; no ledger | Single-writer-per-budget Postgres ledger |
| Audit integrity | OTel spans only | `canonical_events` hash-chain |
| Approval workflow | Not a primitive | `REQUIRE_APPROVAL` outcome |

This is **not** competing with MAF — it slots into MAF's own middleware contract.

### 1.3 Coordination with existing AGT integration

The shipped `spendguard.integrations.agt` module wraps Microsoft Agent Governance Toolkit (AGT) — a policy-engine **above** MAF. MAF middleware is a **lower** hook (the framework's own pipeline). Both ship; the README integrations table lists them as siblings:

> *Pick **AGT** for policy-engine framing ("is this tool action allowed?"); pick **MAF middleware** for framework-native hook ("did the LLM call reserve budget?"). Both can be combined: AGT composite evaluator running inside an MAF middleware delegate.*

## 2. Goals & non-goals

### 2.1 Goals (v1)

- **G1.** Drop-in MAF middleware in both languages with ≤3 config changes per app.
- **G2.** Fail-closed: sidecar unreachable → middleware short-circuits the LLM call (configurable, default deny).
- **G3.** Cover both **.NET** (`Spendguard.AgentFramework` NuGet, netstandard2.1 + net8.0) and **Python** (`spendguard.integrations.agent_framework`, ≥3.11) via one shared design.
- **G4.** Audit chain: every gated MAF `IChatClient` / `ChatClient` call produces a `canonical_events` row keyed by MAF `messageId` + `responseId`.
- **G5.** Real end-to-end demo: `DEMO_MODE=maf_dotnet_real` and `DEMO_MODE=maf_python_real` both run against a real provider via sidecar.

### 2.2 Non-goals (v1)

- **NG1.** Do not replace MAF's orchestration, tool routing, or memory.
- **NG2.** Do not ship a SpendGuard fork of MAF.
- **NG3.** No SK-only or AutoGen-only adapter (both upstreams are maintenance; MAF middleware covers the migration path).
- **NG4.** No Azure OpenAI quota integration — orthogonal axis.

## 3. Architecture

### 3.1 Shared shape

Both languages register a SpendGuard delegate at the MAF `AgentBuilder` middleware pipeline. The delegate runs at the LLM-invocation boundary (MAF: `ChatClient.GetResponseAsync` / `chat_client.get_response`) and sits **after** the prompt-render stage so token estimates use the final rendered prompt.

```
AgentBuilder.Build()
  ├─ message-build
  ├─ prompt-render
  ├─ ─── SpendGuardMiddleware ───      ← reserve(LLM_CALL_PRE)
  ├─ chat-client.get_response          ← actual provider call
  ├─ ─── SpendGuardMiddleware ───      ← commit_estimated(LLM_CALL_POST)
  ├─ response-postprocess
  └─ function-invocation (tools)
```

### 3.2 Sidecar contract reuse

Same UDS+mTLS sidecar contract used by all other adapters. The middleware calls `SidecarAdapter.RequestDecision` with `trigger = LLM_CALL_PRE`, awaits `ALLOW | DENY | DEGRADE | REQUIRE_APPROVAL`, then emits a `LLM_CALL_POST` trace event with real token usage from the provider's response object. No new proto.

### 3.3 .NET surface

- **Package:** `Spendguard.AgentFramework` on NuGet (signed, deterministic build, SourceLink, `<TargetFrameworks>netstandard2.1;net8.0</TargetFrameworks>`).
- **DI extension:** `services.AddSpendGuardMiddleware(options => { options.SocketPath = ...; options.BudgetId = ...; })`.
- **Middleware delegate:** `SpendGuardChatMiddleware : IChatClientMiddleware` (or whatever MAF GA names the contract; verified in implementation.md §2).
- **Transport:** `Grpc.Net.Client` over Unix Domain Socket via `UnixDomainSocketConnectionFactory` (System.Net.Sockets ≥6.0). Windows fallback uses named pipes per sidecar proto contract.

### 3.4 Python surface

- **Package:** add `agent-framework` extra to existing `spendguard-sdk` pyproject (`pip install 'spendguard-sdk[agent-framework]'`). Module: `spendguard.integrations.agent_framework`.
- **Public API:** `SpendGuardMiddleware` class + `run_context()` async context manager, matching the existing `openai_agents.py` shape (`RunContext`, `current_run_context`, `ClaimEstimator`).
- **Registration:** `AgentBuilder().use_middleware(SpendGuardMiddleware(client=..., budget_id=..., ...))` and a function-scope `@spendguard_function_middleware` decorator for tool-level gating.

## 4. Key design decisions

- **ADR-001.** Two languages, one design doc. Slice plan splits .NET and Python; both reference this single design.
- **ADR-002.** Middleware operates at the LLM boundary. Function-level (tool-call) gating is an opt-in second middleware (`SpendGuardToolMiddleware`) so token budget is separable from tool-cost budget.
- **ADR-003.** No new proto. Reuse `SidecarAdapter.RequestDecision` with existing `LLM_CALL_PRE` / `TOOL_CALL_PRE` triggers and existing `EmitTraceEvents` stream.
- **ADR-004.** Default `claim_estimator` reuses Python SDK's `tiktoken` / `tokenizers` core deps. .NET uses `SharpToken` (MIT) for OpenAI tokenizers and falls back to sidecar `count_tokens` UDS RPC for non-OpenAI providers.
- **ADR-005.** Fail-closed default. `OnSidecarUnavailable = Deny` / `on_sidecar_unavailable="deny"`. `Allow` is explicit opt-in with deprecation warning.
- **ADR-006.** Coexists with AGT integration (see §1.3). AGT composite evaluator may be wrapped inside an MAF middleware delegate; tested in `maf_python_with_agt`.
- **ADR-007.** Replay safety. `idempotency_key` per call derives from `(tenant_id, session_id, run_id, message_id, llm_call_id)` so MAF retry middleware cannot double-reserve.
- **ADR-008.** .NET versioning. `Spendguard.AgentFramework` follows the SDK semver line (0.5.x alongside `spendguard-sdk` 0.5.x) until both reach 1.0; NuGet metadata pins `<MinVerTagPrefix>spendguard-sdk-v</MinVerTagPrefix>`.

## 5. Out of scope (for v1)

- Semantic Kernel-only or AutoGen-only filters (upstream maintenance).
- Azure OpenAI deployment quota merge.
- .NET 7 / Framework 4.x backports.
- Multi-region sidecar failover (single sidecar per pod, per existing arch).

## 6. Risks

- **R1.** MAF GA middleware contract is two months old (2026-04); implementation.md §2.5 pins exact assembly + import paths.
- **R2.** .NET UDS on Windows < 10/2019 unsupported by MAF; named-pipe fallback is Windows-only.
- **R3.** AGT + MAF middleware running concurrently could double-count if mis-configured; tested in `maf_python_with_agt` demo.
