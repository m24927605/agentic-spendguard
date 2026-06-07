# DEMO_MODE=botpress_real

D32 SLICE 5 demo overlay. Brings up:

- The base SpendGuard stack (postgres + ledger + canonical-ingest +
  sidecar + outbox-forwarder).
- An in-network counting stub on `http://counting-stub:8765` that
  emulates OpenAI's `/v1/chat/completions` and Anthropic's
  `/v1/messages` endpoints.
- A Node 20 runner (`run_botpress_demo.mjs`) that exercises the
  `@spendguard/botpress-integration` reservation lifecycle directly
  against the sidecar HTTP companion's `/v1/decision` and `/v1/trace`
  endpoints — the same wire path the integration's
  `beforeAiGeneration` / `afterAiGeneration` hooks would drive in a
  real Botpress v12 runtime.

## Why not boot Botpress v12?

The full self-hosted Botpress v12 image is ~800 MB and takes ~30s to
boot before the first conversation can fire. The integration's
SpendGuard contract is the reserve / commit / release path — the
Botpress runtime adds no new SpendGuard invariants over the focused
runner. The full v12 integration tier is exercised in CI via
`testcontainers-node` (see `.github/workflows/botpress-integration-ci.yml`).

## Invariants verified

- **INV-1**: DENY skips upstream — the counting stub's `/_count`
  endpoint shows zero increment across the DENY step.
- **INV-5**: ALLOW + STREAM commit real `inputTokens + outputTokens`
  from the upstream usage frame.

The SQL gate (`verify_step_botpress_real.sql`) verifies both
invariants at the ledger layer, plus the canonical-events outbox
forwarder drained.

## Run

```bash
make demo-up DEMO_MODE=botpress_real
```

Expected stdout includes `botpress_real ALL 3 steps PASS (ALLOW + DENY + STREAM)`
and `D32_BOTPRESS OK: decisions=N` for `N >= 2`.
