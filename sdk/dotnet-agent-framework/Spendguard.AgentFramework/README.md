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
    // HARDEN_D05_UR: thread canonical-truth ledger unit_id UUID. Sourced
    // typically from the SPENDGUARD_UNIT_ID env var. Omit for recipe-style
    // integrations that do not issue ledger-backed reserve calls.
    options.UnitId = Guid.Parse(Environment.GetEnvironmentVariable("SPENDGUARD_UNIT_ID")
        ?? "00000000-0000-0000-0000-000000000000");
});
```

## Unreleased

- `SpendGuardOptions.UnitId` (Guid?) — optional canonical-truth UUID of the
  ledger unit row. When set, the middleware threads it through to
  `DecisionRequest.Inputs.ProjectedUnit.UnitId` on the wire so the sidecar
  ledger can resolve the budget claim (closes the HARDEN_D05_UR substrate
  gap). Additive only; null preserves the pre-HARDEN_D05_UR wire shape.
  Closes HARDEN_D05_UR_S02 for D07.

## Pick AGT or MAF middleware?

- **AGT integration** (`spendguard.integrations.agt`) — policy-engine framing
  ("is this tool action allowed?"). Layered above MAF.
- **MAF middleware** (this package) — framework-native hook ("did the LLM call
  reserve budget?"). Inside MAF's own delegating chat client pipeline.

Both can be combined: AGT composite evaluator running inside an MAF middleware
delegate.

## License

Apache-2.0
