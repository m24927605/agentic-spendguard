# D07 — Microsoft Agent Framework (MAF) middleware — implementation.md

> Status: Proposed. Sibling: `design.md`, `tests.md`, `acceptance.md`, `review-standards.md`.
> Reader audience: implementer sub-agents (Backend Architect + supporting AI Engineer for .NET).

## 1. Slice plan

8 slices, total ≈ 6 medium + 2 small. Two languages, but slices are interleaved so each slice has a self-contained acceptance gate.

| # | Slice ID | Size | Language | Title |
|---|---------|------|----------|-------|
| 1 | `COV_d07_01_dotnet_skeleton` | M | .NET | Create `Spendguard.AgentFramework` csproj + NuGet metadata + CI build |
| 2 | `COV_d07_02_dotnet_uds_client` | M | .NET | gRPC-over-UDS sidecar client + Handshake + RequestDecision |
| 3 | `COV_d07_03_dotnet_middleware` | M | .NET | `SpendGuardChatMiddleware` delegate + DI extension + token estimator |
| 4 | `COV_d07_04_dotnet_tests` | M | .NET | xUnit unit + sidecar-stub integration tests + golden snapshots |
| 5 | `COV_d07_05_python_module` | S | Python | Add `agent-framework` extra + `spendguard.integrations.agent_framework` module |
| 6 | `COV_d07_06_python_middleware` | M | Python | `SpendGuardMiddleware` + `run_context()` + tool-scope middleware |
| 7 | `COV_d07_07_python_tests` | M | Python | pytest unit + sidecar-stub integration + replay-safety |
| 8 | `COV_d07_08_demos_docs` | M | Both | `DEMO_MODE=maf_dotnet_real` + `DEMO_MODE=maf_python_real` + `maf_python_with_agt` + Makefile targets + docs page + README integrations row |

Slice 1-4 are .NET-only; 5-7 Python-only; 8 stitches both. Slices 1 and 5 can start in parallel.

## 2. Code structure

### 2.1 .NET package layout

```
sdk/dotnet/
├── Spendguard.AgentFramework.sln
├── src/Spendguard.AgentFramework/
│   ├── Spendguard.AgentFramework.csproj
│   ├── SpendGuardChatMiddleware.cs       ← IChatClientMiddleware impl
│   ├── SpendGuardOptions.cs              ← DI options bag
│   ├── DependencyInjection/
│   │   └── ServiceCollectionExtensions.cs
│   ├── Sidecar/
│   │   ├── SidecarChannelFactory.cs      ← UDS Grpc.Net.Client connect
│   │   ├── SidecarClient.cs              ← thin wrapper over generated stub
│   │   └── HandshakeManager.cs
│   ├── Tokens/
│   │   ├── ITokenEstimator.cs
│   │   ├── SharpTokenEstimator.cs        ← OpenAI provider tokenizer
│   │   └── SidecarTokenEstimator.cs      ← non-OpenAI fallback (UDS)
│   ├── Generated/                        ← protoc-gen-csharp output
│   │   └── ...sidecar_adapter.cs
│   └── Ids/
│       └── IdempotencyKeyDerivation.cs   ← mirrors sdk/python/spendguard/ids.py
└── tests/Spendguard.AgentFramework.Tests/
    └── ...
```

`Spendguard.AgentFramework.csproj` targets `<TargetFrameworks>netstandard2.1;net8.0</TargetFrameworks>`, sets `<IsPackable>true</IsPackable>`, embeds SourceLink, signs deterministically. Proto generation hooked into `<Target Name="GenerateProtos" BeforeTargets="BeforeCompile">` using `Grpc.Tools` 2.65+.

### 2.2 Python package layout

```
sdk/python/src/spendguard/integrations/
├── agent_framework.py                ← new (mirrors openai_agents.py)
└── _agent_framework_estimator.py     ← reuses default tokenizer estimator
```

`pyproject.toml` gains:

```toml
[project.optional-dependencies]
agent-framework = [
  "agent-framework>=1.0,<2",
]
```

### 2.3 Key types — .NET

```csharp
namespace Spendguard.AgentFramework;

public sealed class SpendGuardOptions
{
    public string SocketPath { get; set; } = "/var/run/spendguard/sidecar.sock";
    public Guid TenantId { get; set; }
    public string BudgetId { get; set; } = "";
    public string WindowInstanceId { get; set; } = "";
    public OnSidecarUnavailable OnSidecarUnavailable { get; set; } = OnSidecarUnavailable.Deny;
    public Func<ChatRequest, IReadOnlyList<BudgetClaim>>? ClaimEstimator { get; set; }
}

public enum OnSidecarUnavailable { Deny, Allow }

public sealed class SpendGuardChatMiddleware : IChatClientMiddleware
{
    public async Task<ChatResponse> InvokeAsync(
        ChatRequest request,
        ChatMiddlewareDelegate next,
        CancellationToken ct)
    {
        var decision = await _sidecar.RequestDecisionAsync(...);
        if (decision.Outcome == DecisionOutcome.Deny)
            throw new SpendGuardDecisionDeniedException(decision);

        ChatResponse response;
        try { response = await next(request, ct); }
        catch { await _sidecar.ReleaseAsync(...); throw; }

        await _sidecar.EmitLlmCallPostAsync(response.Usage, ...);
        return response;
    }
}
```

### 2.4 Key types — Python

Mirrors `sdk/python/src/spendguard/integrations/openai_agents.py`:

```python
class SpendGuardMiddleware:
    def __init__(self, *, client, budget_id, window_instance_id, unit, pricing,
                 claim_estimator=None, on_sidecar_unavailable="deny",
                 default_model=""): ...

    async def __call__(self, ctx: "ChatMiddlewareContext", call_next):
        # 1) Derive idempotency_key from RunContext.
        # 2) await client.request_decision(trigger=LLM_CALL_PRE, ...)
        # 3) Deny → raise DecisionDenied (matches agt.py + openai_agents.py).
        # 4) On call_next exception → release the reservation.
        # 5) After call_next → emit LLM_CALL_POST trace event with real usage.
```

### 2.5 MAF API contract pin

- **.NET:** package reference `Microsoft.Agents.Framework >= 1.0.0, < 2.0.0` (NuGet). The middleware contract type pinned by name: `Microsoft.Agents.Framework.Middleware.IChatClientMiddleware`. If MAF GA renamed this between writing and slice 1, the implementer adds an indirection layer; the slice is allowed to ship if and only if it tests against the publicly-released NuGet.
- **Python:** `agent-framework >= 1.0, < 2`. The middleware base class pinned by import path: `agent_framework.middleware.ChatMiddleware`. The exact class is verified at slice-start by `python -c "from agent_framework.middleware import ChatMiddleware"`.

## 3. Sidecar protocol use

All slices reuse `proto/spendguard/sidecar_adapter/v1/adapter.proto`. No proto changes.

| Phase | RPC | Trigger | Idempotency |
|------|-----|---------|-------------|
| Connect | `Handshake` | n/a | One-shot, per-process |
| LLM pre-gate | `RequestDecision` | `LLM_CALL_PRE` | key = blake2b(tenant_id, session_id, run_id, step_id, llm_call_id, trigger) |
| LLM post-event | `EmitTraceEvents` (server stream) | `LLM_CALL_POST` | event_id derived from llm_call_id |
| Tool pre-gate (opt-in) | `RequestDecision` | `TOOL_CALL_PRE` | key includes tool_call_id |

## 4. Build + CI hooks

- **`make sdk-dotnet-build`** — new Makefile target: `dotnet build sdk/dotnet/Spendguard.AgentFramework.sln -c Release`.
- **`make sdk-dotnet-test`** — `dotnet test ... --logger trx`.
- **`make sdk-dotnet-pack`** — `dotnet pack -c Release -o sdk/dotnet/dist`.
- **`.github/workflows/dotnet.yml`** — new workflow restricted to `paths: ['sdk/dotnet/**', 'proto/**']`.
- Python slice extends existing pyproject `[project.optional-dependencies]` and reuses existing `make sdk-python-test`.

## 5. Demo modes (slice 8)

| Mode | File | What it proves |
|------|------|----------------|
| `maf_dotnet_real` | `deploy/demo/Makefile` + `examples/maf-dotnet/` | .NET console app, real provider, sidecar reserve → commit cycle, audit row produced |
| `maf_python_real` | `examples/maf-python/` | Python `agent_framework` app, real provider, same lifecycle |
| `maf_python_with_agt` | `examples/maf-python-agt/` | AGT composite evaluator wrapped inside MAF middleware; proves the two integrations compose without double-counting |
| `maf_dotnet_deny` | `examples/maf-dotnet/` (deny seed) | Budget exhaustion → `SpendGuardDecisionDeniedException` → no provider call |
| `maf_python_deny` | `examples/maf-python/` (deny seed) | Same on Python (typed `DecisionDenied`) |

## 6. Docs deliverables (slice 8)

- `docs/site/docs/integrations/microsoft-agent-framework.md` — user guide, both languages.
- `README.md` adapter table row: "Microsoft Agent Framework — `Spendguard.AgentFramework` (NuGet) + `spendguard-sdk[agent-framework]` (PyPI)." With "complements AGT integration; pick AGT for policy framing, MAF middleware for framework-native hook" callout.
- `CHANGELOG.md` two entries (one per package, semver-bumped).

## 7. Memory write-back

Per build plan §8, when D07 is fully merged, write `~/.claude/projects/-Users-michael-chen-products-agentic-spendguard/memory/project_coverage_D07_shipped.md` with the canonical pattern (merge commit + round count + arbitration y/n + closed issues + .NET package version + Python extra version).
