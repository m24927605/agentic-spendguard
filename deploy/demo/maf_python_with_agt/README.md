# `DEMO_MODE=maf_python_with_agt`

Composite docker-compose demo: MAF middleware wrapping the AGT policy
engine. Proves the design.md ADR-006 coexistence story — a single
`SpendGuardClient` shared between
`spendguard.integrations.agent_framework.SpendGuardMiddleware`
(the MAF chat-client hook) and the shipped
`spendguard.integrations.agt` module (the AGT policy plugin).

```bash
make demo-up DEMO_MODE=maf_python_with_agt
```

## Status: smoke-only

This overlay is **not** part of the D07 closer's verify gates — the
.NET + Python sibling overlays (`deploy/demo/maf_dotnet/` +
`deploy/demo/maf_python/`) carry the load-bearing ALLOW + DENY +
ALLOW2 assertions. This overlay exists to smoke-test that:

- A single `SpendGuardClient` can be reused between the two
  integration surfaces without spurious double-handshake or
  double-reservation behaviour.
- The AGT + MAF middleware ordering does not produce conflicting
  decisions on the same call.

## What it boots

The same `counting-stub` + `maf-python-with-agt-runner` topology as
`deploy/demo/maf_python/docker-compose.yaml`, with the runner's
`SPENDGUARD_DEMO_WITH_AGT=1` env var signalling an AGT path that
future revisions of `examples/maf-python/run.py` can branch on.
The current run.py treats this flag as informational only; the
overlay runs the same 3-step matrix as the `maf_python_real` mode.

## Anti-scope

This overlay does NOT:

- Re-declare base services (postgres, sidecar, ledger, etc.).
- Modify the run.py driver semantics — the env var is wired through
  for future use.
- Promote AGT-side gates to verify-step status. The shipped
  `spendguard.integrations.agt` module has its own dedicated demo
  mode (`DEMO_MODE=agent_real_agt`); this composite overlay is a
  coexistence smoke, not a replacement for either dedicated demo.

## Related

- [`deploy/demo/maf_python/`](../maf_python/) — load-bearing Python
  half of D07.
- [`deploy/demo/maf_dotnet/`](../maf_dotnet/) — load-bearing .NET
  half of D07.
- AGT dedicated demo: `make demo-up DEMO_MODE=agent_real_agt`.
- design.md §1.3 and ADR-006 — the AGT + MAF middleware coexistence
  story this overlay smokes.
