# `DEMO_MODE=vercel_ai_mastra` (COV_D06 SLICE 7)

Demo bundle that proves the **Vercel AI SDK middleware path**
(`createSpendGuardMiddleware` from `@spendguard/vercel-ai`, transitively
covers Mastra Agents via the `@spendguard/vercel-ai/mastra` subpath
alias) gates a `generateText` / `streamText` call **before** the
upstream provider HTTP request leaves the process, with a hard-cap
deny short-circuit and end-of-stream commit reconciliation.

This is the Vercel AI SDK + Mastra sibling of the LangChain.js demo
(`DEMO_MODE=langchain_ts`,
`deploy/demo/langchain_ts/docker-compose.yaml`), which uses the
`SpendGuardCallbackHandler` callback shape instead. Both paths must
keep working — design.md §6 (Vercel AI SDK middleware is the
TS-idiomatic surface; callback handler is the LangChain idiom).

## Files

| Path | Purpose |
|------|---------|
| `docker-compose.yaml` | Overlay declaring `counting-stub` + `vercel-ai-mastra-runner` (Node 20) services |
| `README.md` | This file |

The actual Node script lives at
[`examples/vercel-ai-mastra/index.mjs`](../../../examples/vercel-ai-mastra/index.mjs)
— mounted read-only into the `vercel-ai-mastra-runner` container at boot.

## Bring-up

```bash
make demo-up DEMO_MODE=vercel_ai_mastra
```

The `vercel-ai-mastra-runner` container
(`spendguard-vercel-ai-mastra-runner`):

1. Stages `examples/vercel-ai-mastra/{package.json,index.mjs}` to a
   tmpfs so `npm install` can patch the SpendGuard halves to `file:`
   deps against the workspace's pre-built `sdk/typescript/dist/` +
   `sdk/typescript-vercel-ai/dist/`.
2. Runs `npm install` (pulls `ai@^4` + `zod` from npm; resolves
   `@spendguard/sdk` + `@spendguard/vercel-ai` locally).
3. Waits for `/var/run/spendguard/adapter.sock` to appear (sidecar
   readiness gate).
4. Asserts at boot that
   `createSpendGuardLanguageMiddleware === createSpendGuardMiddleware`
   (the `/mastra` subpath alias is a function-reference alias — D06
   review-standards §1.6 LOCK).
5. Connects + handshakes via `SpendGuardClient`, then drives 3
   `generateText` / `streamText` calls through
   `wrapLanguageModel({ model, middleware: createSpendGuardMiddleware(...) })`:
   - ALLOW: small message within budget → counting-stub counter +1,
     SUCCESS commit with provider-reported tokens.
   - DENY: extra body `spendguard_estimate_override=2000000000` blows
     past the seeded 1B hard-cap → contract evaluator emits
     `SPENDGUARD_DENY`; middleware's `transformParams` throws
     `DecisionDenied`; counting-stub counter UNCHANGED.
   - STREAM: `streamText` consumed via `result.textStream` async
     iterator → PRE fires once at stream open, POST commits once at
     stream end via the `TransformStream` `flush()` hook.

Success line on a clean run (the CI grep depends on this exact
spelling):

```
[demo] vercel_ai_mastra ALL 3 steps PASS (ALLOW + DENY + STREAM)
```

## Verification

After the runner exits 0, the Makefile target runs
`verify_step_vercel_ai_mastra.sql` against `spendguard_ledger` to
assert the ledger-side gates:

- `reserve >= 2` (ALLOW + STREAM each produce a reservation)
- `commit_estimated >= 2` (both ALLOW paths commit)
- `denied_decision >= 1` (DENY step short-circuits at the sidecar
  before the upstream HTTP call leaves the runner)
- INV-2 strict-order: earliest reservation precedes earliest
  `spendguard.audit.outcome` row

Then a 5-second outbox drain wait + cross-DB
`canonical_events` count against `spendguard_canonical` asserts
`decision >= 2 AND outcome >= 1` after the outbox forwarder lands the
audit rows.

## Topology

```
┌──────────────────────────────────────────────────────────┐
│  vercel-ai-mastra-runner (Node 20)                       │
│    generateText / streamText (ai@4)                      │
│    + wrapLanguageModel({                                 │
│        middleware: createSpendGuardMiddleware(...)       │
│      })                                                  │
│    + SpendGuardClient (UDS)                              │
└─────────┬───────────────────────────────────┬────────────┘
          │                                   │
          │ UDS gRPC                          │ HTTP (counting-stub /v1)
          ▼                                   ▼
┌──────────────────────────┐         ┌────────────────────────────┐
│  sidecar (Rust, gRPC)    │         │  counting-stub (Python)    │
│   + contract evaluator   │         │   + OpenAI /v1 shape       │
│   + ledger reserve()     │         │   + GET /_count tally      │
└──────────┬───────────────┘         └────────────────────────────┘
           │ mTLS
           ▼
┌──────────────────────────┐
│  ledger (Rust, Postgres) │
└──────────────────────────┘
```

## Mastra coverage

Mastra Agents call `generateText` / `streamText` from `ai`
underneath. The demo runner's `wrapLanguageModel(...)` call is what a
Mastra `Agent.generate()` / `Agent.stream()` call eventually
delegates to — the middleware is wired at the `LanguageModelV1`
boundary so the same code path covers both ecosystems byte-for-byte.

A Mastra-side consumer would import the factory under its
Mastra-idiomatic name via the subpath alias:

```ts
import { createSpendGuardLanguageMiddleware } from "@spendguard/vercel-ai/mastra";
```

The demo runner imports BOTH names and asserts
`createSpendGuardLanguageMiddleware === createSpendGuardMiddleware`
at boot so the function-reference alias contract (D06 review-standards
§1.6) is exercised in production-shape code, not just unit tests.

## Anti-scope

This demo does NOT exercise:

- Real `@ai-sdk/openai` / `@ai-sdk/anthropic` provider HTTP — the
  SLICE 6 provider matrix tests under
  [`sdk/typescript-vercel-ai/tests/providers.test.ts`](../../../sdk/typescript-vercel-ai/tests/providers.test.ts)
  cover the LanguageModelV1 surface parity against hand-rolled
  doubles of those providers. The demo uses an in-process
  counting-stub-backed model with the same `LanguageModelV1`
  surface so the middleware exercises its real paths without
  requiring `OPENAI_API_KEY`.
- Mid-stream tool-call gating — anti-scope per design.md §3, deferred
  to v0.2.
- DEGRADE patch application — anti-scope per design.md §3, deferred
  to v0.2.
