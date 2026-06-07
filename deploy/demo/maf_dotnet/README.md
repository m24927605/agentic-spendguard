# `DEMO_MODE=maf_dotnet_real`

End-to-end docker-compose demo for the `Spendguard.AgentFramework`
NuGet — the Microsoft Agent Framework (MAF) .NET middleware adapter
(the .NET half of D07).

```bash
make demo-up DEMO_MODE=maf_dotnet_real
```

## What it boots

Layered on top of the base `deploy/demo/compose.yaml` stack
(`postgres + ledger + canonical-ingest + sidecar + webhook-receiver +
outbox-forwarder`):

| Service | Role |
| ------- | ---- |
| `counting-stub` | In-network mock OpenAI `/v1/chat/completions` provider. Counts every call; serves `/_count` to readers. |
| `maf-dotnet-runner` | .NET 8 container — builds `examples/maf-dotnet` against the in-tree `Spendguard.AgentFramework` project and runs it against the sidecar UDS, drives `IChatClient.UseSpendGuard(sp)` against a counting-stub-backed inner client. |

## What it asserts

Three calls drive through the SpendGuard-wrapped `IChatClient`:

1. **ALLOW** — small message within budget. `UseSpendGuard(...)`'s
   `RequestDecision` returns `CONTINUE`, the inner
   `CountingStubChatClient.GetResponseAsync` POSTs to the counting-stub,
   counter `+1`, `EmitLlmCallPostStub` logs the SUCCESS commit.
2. **DENY** — message tagged `trigger-deny: please block me`. The
   counting-stub forwards the body's `spendguard_estimate_override=2000000000`
   to the sidecar's contract evaluator (via the proxy path); the
   evaluator emits `SPENDGUARD_DENY`; the middleware throws
   `SpendGuardDecisionDeniedException`; the inner HTTP **never fires**.
   Counter `+0` (proves the gate fires pre-call).
3. **ALLOW2** — second small message within budget. Replaces D04 /
   D06 / D08's STREAM step (streaming gating is v0.1.x non-goal —
   `design.md §3`). Counter `+1`.

Success line on a clean run (LOCKED — CI greps for the exact spelling):

```
[demo] maf_dotnet ALL 3 steps PASS (ALLOW + DENY + ALLOW2)
```

## Verify gates

After the runner exits, the Makefile runs:

```bash
make demo-verify-maf-dotnet
```

Which executes [`verify_step_maf_dotnet.sql`](../verify_step_maf_dotnet.sql)
against `spendguard_ledger` and a cross-DB `canonical_events` count
against `spendguard_canonical`. The gates:

- `ledger_transactions.reserve >= 2` (ALLOW + ALLOW2 each produce
  EXACTLY one reservation).
- `ledger_transactions.commit_estimated >= 2` (ALLOW + ALLOW2).
- `ledger_transactions.denied_decision >= 1` (DENY step).
- `audit_outbox` carries `>= 2` decision rows in the last 5 min.
- `canonical_events` carries `>= 2` decision + `>= 1` outcome rows.
- INV-2 strict-order: first reservation row precedes first outcome row.

## Anti-scope

This overlay does NOT redeclare `postgres`, `sidecar`, `ledger`,
`canonical-ingest`, `bundles-init`, `pki-init` or any other base
service — the Makefile target brings up the base stack first, then
this overlay.

## Known cross-slice gap

The `--real` end-to-end run currently surfaces the same D05
`UnitRef.unit_id empty` substrate-side validation error D04 / D06 /
D08 also surface against the same sidecar. The adapter's wire shape
and middleware contract are independently verified by the xUnit
suite (`sdk/dotnet-agent-framework/Spendguard.AgentFramework.Tests`);
the `--mock` mode exercises the bracket end-to-end without going
through the substrate's `ReserveSet` validator. A future SDK-side
`unit_id` broadening lands the `--real` demo green here and across
the sibling D04 / D06 / D08 modes simultaneously.

## Related

- [`examples/maf-dotnet/`](../../../examples/maf-dotnet/) — the demo's
  source.
- [`sdk/dotnet-agent-framework/`](../../../sdk/dotnet-agent-framework/)
  — the adapter package.
- [`deploy/demo/maf_python/`](../maf_python/) — Python sibling demo,
  same 3-step matrix.
- [`deploy/demo/maf_python_with_agt/`](../maf_python_with_agt/) —
  Python + AGT composite demo (MAF middleware wrapping the AGT policy
  engine).
