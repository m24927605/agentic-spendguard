# Spendguard.AgentFramework

SpendGuard middleware for Microsoft Agent Framework (`Microsoft.Agents.AI`).
Gates LLM calls against the SpendGuard sidecar via Unix Domain Socket
gRPC. Default behavior is fail-closed: when the sidecar is unreachable, the
LLM call is short-circuited rather than allowed unaudited.

> Pre-release (`0.1.0-pre`). Not yet wired into the published examples.

## Install

```bash
dotnet add package Spendguard.AgentFramework --prerelease
```

## Quick start

```csharp
using Spendguard.AgentFramework.Extensions;

builder.Services.AddSpendGuard(options =>
{
    options.TenantId = "tenant-uuid";
    options.BudgetId = "team-llm-budget";
    options.SidecarSocketPath = "/var/run/spendguard/adapter.sock";
});
```

## Pick AGT or MAF middleware?

- **AGT integration** (`spendguard.integrations.agt`) — policy-engine framing
  ("is this tool action allowed?"). Layered above MAF.
- **MAF middleware** (this package) — framework-native hook ("did the LLM call
  reserve budget?"). Inside MAF's own delegating chat client pipeline.

Both can be combined: AGT composite evaluator running inside an MAF middleware
delegate.

## License

Apache-2.0
