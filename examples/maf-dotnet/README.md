# `examples/maf-dotnet`

End-to-end .NET 8 console demo for the `Spendguard.AgentFramework`
NuGet — the Microsoft Agent Framework (MAF) middleware adapter (the
.NET half of D07).

```bash
# laptop iteration, no sidecar required
dotnet run --project examples/maf-dotnet -- --mock

# end-to-end against the sidecar + counting-stub
make demo-up DEMO_MODE=maf_dotnet_real
```

## What it shows

Wires an `IChatClient` through the SpendGuard
`SpendGuardChatMiddleware` via the `services.AddSpendGuard(...)` +
`inner.UseSpendGuard(sp)` DI extension and drives 3 chat-client calls:

| Step | Outcome | Inner call fires? | Notes |
| ---- | ------- | ----------------- | ----- |
| **1. ALLOW** | `CONTINUE` | YES | small message within budget. `UseSpendGuard(...)`'s `RequestDecision` returns `CONTINUE`, inner counting-stub receives one POST. |
| **2. DENY** | `STOP` | NO | message tagged `trigger-deny`. In `--mock` the in-process stub `ISidecarClient` returns `STOP`; in `--real` the contract evaluator emits `SPENDGUARD_DENY` via `spendguard_estimate_override`. Middleware throws `SpendGuardDecisionDeniedException`; the inner HTTP **never fires**. |
| **3. ALLOW2** | `CONTINUE` | YES | second small message — proves cross-call determinism. Replaces D04 / D06 / D08's STREAM step (streaming gating is v0.1.x non-goal — `design.md §3`). |

Success line on a clean run (LOCKED — CI grep depends on the exact
spelling, mirrors the `openai_agents_ts` / `inngest_agent_kit`
composite convention):

```
[demo] maf_dotnet ALL 3 steps PASS (ALLOW + DENY + ALLOW2)
```

## Wire shape

```csharp
using Microsoft.Extensions.AI;
using Microsoft.Extensions.DependencyInjection;
using Spendguard.AgentFramework.Extensions;
using Spendguard.AgentFramework.Options;

var services = new ServiceCollection();
services.AddLogging();
services.AddSpendGuard(o =>
{
    o.TenantId        = "00000000-0000-4000-8000-000000000001";
    o.BudgetId        = "44444444-4444-4444-8444-444444444444";
    o.WindowInstanceId = "55555555-5555-4555-8555-555555555555";
    o.SidecarSocketPath = "/var/run/spendguard/adapter.sock";
    o.OnSidecarUnavailable = OnSidecarUnavailable.Deny;
});

await using var sp = services.BuildServiceProvider();
IChatClient inner = /* your real OpenAI/Azure/etc. client */;
IChatClient gated = inner.UseSpendGuard(sp);

var resp = await gated.GetResponseAsync(new[]
{
    new ChatMessage(ChatRole.User, "hello"),
});
```

This sample uses an in-process HTTP-backed `IChatClient` pointing at
the demo's counting-stub instead of a real provider so the
end-to-end run is deterministic in the offline container. Swap
`CountingStubChatClient` for a `Microsoft.Extensions.AI.OpenAI`
`OpenAIChatClient` or any `IChatClient` implementation in production —
the middleware is contractually identical.

## Why an `IChatClient` and not a `ChatAgent`?

The Microsoft Agent Framework's `ChatAgent` drives the same
`IChatClient.GetResponseAsync(...)` boundary internally. The
`SpendGuardChatMiddleware` is a `DelegatingChatClient` — it sits on
that boundary regardless of whether the call originates from a hand-
written `IChatClient` consumer or a `ChatAgent`. We keep the sample
focused on the seam the middleware gates so the contract is obvious;
a future revision of this example can swap in a `ChatAgent` against
a real provider without rewriting the middleware-wiring story.

## Anti-scope

- **No real provider key required.** `--real` mode talks to the
  counting-stub via plain HTTP. Production wiring substitutes
  `Microsoft.Extensions.AI.OpenAI` or `Microsoft.Extensions.AI.Azure`.
- **No `ChatAgent` orchestration.** See above. The middleware is the
  load-bearing surface; agent orchestration is upstream of it.
- **Streaming-per-chunk gating is anti-scope for v0.1.x.** The
  `IAsyncEnumerable<ChatResponseUpdate>` boundary is left as-is for
  follow-up work.

## Known gap (cross-slice)

The `--real` demo run currently surfaces the same D05
`UnitRef.unit_id empty` substrate validation error D04 / D06 / D08
also surface against the same sidecar. The .NET adapter's middleware
+ wire shape are independently verified by the xUnit suite
(`sdk/dotnet-agent-framework/Spendguard.AgentFramework.Tests`); the
`--mock` mode exercises the bracket end-to-end without going through
the substrate's `ReserveSet` validator. A future SDK-side
`unit_id` broadening lands the `--real` demo green here and across
the sibling D04 / D06 / D08 modes simultaneously.

## Related

- [`sdk/dotnet-agent-framework/`](../../sdk/dotnet-agent-framework/) — the
  `Spendguard.AgentFramework` NuGet source.
- [`examples/maf-python/`](../maf-python/) — Python sibling demo, same
  3-step matrix.
- [`docs/site-v2/src/content/docs/docs/integrations/microsoft-agent-framework.mdx`](../../docs/site-v2/src/content/docs/docs/integrations/microsoft-agent-framework.mdx)
  — the published integration page.
