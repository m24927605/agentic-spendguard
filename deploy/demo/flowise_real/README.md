# `DEMO_MODE=flowise_real` — D35 SLICE 5

Runs the focused integration-tier demo for
`@spendguard/flowise-nodes` — the SpendGuard Flowise custom node
package.

## What runs

A compose overlay that layers on top of `deploy/demo/compose.yaml`:

1. **counting-stub** — in-network mock OpenAI provider that counts
   incoming requests (mirrors the D32 Botpress / D31 Coze / D10 Dify
   demo stubs).
2. **flowise-integration-runner** — Node 20 container that drives the
   sidecar HTTP companion through a 3-step matrix (ALLOW + DENY +
   STREAM) using the same wire shape the
   `SpendGuardChatModelWrapper` node exercises in production.

## Why focused, not full Flowise

Spinning up `flowiseai/flowise:2.x` (~1.2 GB image, ~30s boot, UI-driven
chatflow config) just to dispatch the same two sidecar HTTP calls that
the wrapper's runtime drives in production is wasted CI minutes. The
focused runner exercises the EXACT lifecycle:

- `POST /v1/decision` — pre-call reserve, identical body shape to the
  one D04's `SpendGuardCallbackHandler` builds.
- `POST /v1/trace` — end-of-call commit, identical body shape.

The full Flowise 2.x runtime invariant (canvas wiring, chatflow JSON
schema, prediction endpoint contract) is exercised in the wrapper's
testcontainers E2E suite at
`integrations/flowise/tests/e2e/flowiseContainer.test.ts`. See the
docker-compose.yaml DEVIATION comment for the parallel rationale that
covers D32 Botpress.

## Run

```bash
make demo-up DEMO_MODE=flowise_real
```

Exit code 0 means ALL 3 steps PASS (ALLOW + DENY + STREAM); the Make
target then runs `verify_step_flowise_real.sql` to assert the audit
chain at the ledger DB layer.

## Invariants verified

| Invariant | Where verified                                                            |
| --------- | ------------------------------------------------------------------------- |
| INV-1     | Step 2 (DENY) — counting-stub hit-count UNCHANGED across the reserve POST |
| INV-5     | Step 1 (ALLOW) — commit posts real `inputTokens + outputTokens`           |
| Streaming | Step 3 — decision context carries `stream=true`; SQL gate asserts the row |
