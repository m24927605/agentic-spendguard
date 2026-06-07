# `DEMO_MODE=n8n_real` — D37 SLICE 5 (n8n community node)

This overlay mirrors `botpress_real` and `inngest_agent_kit`. Run from
the repo root:

```bash
make -C deploy/demo demo-up DEMO_MODE=n8n_real
```

It boots the SpendGuard base stack + a `counting-stub` mock provider +
a Node 20 `n8n-integration-runner` that exercises the n8n integration's
`reserve` / `commit` / `release` lifecycle directly against the sidecar
HTTP companion (`/v1/decision` + `/v1/trace`).

The runner drives a 3-step matrix:

1. **ALLOW** — reserve + upstream + commit; counting stub +1.
2. **DENY** — reserve returns DENY; INV-1 enforces zero upstream HTTP.
3. **STREAM** — same as ALLOW but with `stream=true` on the decision
   context (the streaming-mode SQL gate consumes this).

## Why the focused runner instead of a full n8n self-host

n8n v1.50 self-host is ~600 MB and ~30 s of boot just to dispatch two
HTTP calls. The integration's own conformance against the v1.50 runtime
lives in CI via testcontainers-node; the demo here focuses on the
reserve/commit/release wire shape so the audit-chain invariants
(INV-1 / INV-2 / INV-5) verify in <30 s.

The Botpress and Inngest AgentKit demos follow the same pattern — see
the deviation note in this overlay's `docker-compose.yaml` header.
