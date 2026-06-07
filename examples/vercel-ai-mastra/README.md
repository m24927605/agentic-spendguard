# Vercel AI SDK + Mastra + SpendGuard — runnable Node example

> **Status: first-party reference example.** Drops a SpendGuard
> `createSpendGuardMiddleware` into the Vercel AI SDK v4
> `wrapLanguageModel({ model, middleware })` so every `generateText` /
> `streamText` call reserves against a budget BEFORE the upstream
> provider HTTP call leaves the process. No provider subclassing, no
> proxy. The same middleware covers Mastra Agents transparently via the
> `@spendguard/vercel-ai/mastra` subpath alias (Mastra `Agent.generate()`
> resolves down to `generateText` from `ai`).

This is the JS/TS sibling of
[`examples/langchain-ts/`](../langchain-ts/) for the Vercel AI SDK +
Mastra ecosystem.

## What this proves

Three hard invariants:

1. **SpendGuard DENY ⇒ the upstream provider is NEVER invoked.** If the
   budget is exhausted (or the contract evaluator emits
   `SPENDGUARD_DENY`), the middleware's `transformParams` throws
   `DecisionDenied`; `generateText` propagates it BEFORE the inner
   `doGenerate()` HTTP call fires. Verified end-to-end by the DENY
   step's counting-stub `pre==post` assertion.
2. **End-of-stream commit reconciles real usage.** `wrapStream`
   instruments the `ReadableStream<LanguageModelV1StreamPart>` so the
   SUCCESS commit fires AFTER the consumer drains the final `finish`
   part. The commit ships the provider's `usage.completionTokens`, not
   the estimator worst-case.
3. **Mastra subpath alias = byte-equal function reference.** At boot
   the demo asserts
   `createSpendGuardLanguageMiddleware === createSpendGuardMiddleware`
   (strict equality from `@spendguard/vercel-ai/mastra` vs
   `@spendguard/vercel-ai`). One factory, two import paths — Mastra
   users replace the import name; behaviour is identical.

## Quickstart

The example is wired for the in-tree counting-stub upstream (no real
OpenAI key required). To run end-to-end:

```bash
# Boot the full SpendGuard demo stack with the vercel_ai_mastra overlay.
cd /path/to/agentic-spendguard
make -C deploy/demo demo-up DEMO_MODE=vercel_ai_mastra
```

Expected output (from the vercel-ai-mastra-runner container logs):

```
[demo] vercel_ai_mastra driver: socket=/var/run/spendguard/adapter.sock tenant=... openai_base=http://counting-stub:8765/v1
[demo] mastra alias parity: createSpendGuardLanguageMiddleware === createSpendGuardMiddleware
[demo] handshake ok session_id=...
[demo] (1) ALLOW step — invoking generateText within budget
[demo] (1) ALLOW reply="..." counter pre=0 post=1 usage={promptTokens:5,completionTokens:7}
[demo] (2) DENY step — forcing hard-cap overflow
[demo] (2) DENY caught DecisionDenied (instanceof DecisionDenied=true): ...
[demo] (2) DENY counter pre=1 post=1 threw=true kind=DecisionDenied
[demo] (3) STREAM step — streaming chunks within budget
[demo] (3) STREAM chunks=1 text="..." counter pre=1 post=2 usage={promptTokens:5,completionTokens:7}
[demo] vercel_ai_mastra ALL 3 steps PASS (ALLOW + DENY + STREAM)
```

## Topology

```
┌─────────────────────────────────────────────────────────────┐
│  index.mjs (Node 20)                                        │
│    generateText / streamText                                │
│    + wrapLanguageModel({middleware: createSpendGuardMiddleware(...)})│
│    + SpendGuardClient (UDS)                                 │
└────────────────┬───────────────┬────────────────────────────┘
                 │               │
                 │ UDS gRPC      │ HTTP (OpenAI baseURL)
                 ▼               ▼
┌─────────────────────────────────────────────────────────────┐
│  spendguard-sidecar (Rust)                                  │
│    • contract DSL (hard-cap, approval, deny rules)          │
│    • per-pod fencing lease                                  │
└────────────────┬────────────────────────────────────────────┘
                 │ mTLS gRPC
                 ▼
┌─────────────────────────────────────────────────────────────┐
│  spendguard-ledger (Rust, Postgres-backed)                  │
└─────────────────────────────────────────────────────────────┘
```

## Mastra path

Mastra Agents call `generateText` / `streamText` from `ai` underneath.
Replace the root import with the subpath alias:

```ts
// Before:
import { createSpendGuardMiddleware } from "@spendguard/vercel-ai";

// After (Mastra-idiomatic):
import { createSpendGuardLanguageMiddleware } from "@spendguard/vercel-ai/mastra";
```

The factory is byte-identical (`===` strict equality). The example
above imports BOTH and asserts the equality at boot to make the
function-reference alias contract explicit.

## Standalone use

To embed the wrapped model in your own Mastra `Agent`:

```ts
import { Agent } from "@mastra/core";
import { wrapLanguageModel } from "ai";
import { openai } from "@ai-sdk/openai";
import { createSpendGuardLanguageMiddleware } from "@spendguard/vercel-ai/mastra";

const middleware = createSpendGuardLanguageMiddleware({
  client,
  tenantId: "tenant-prod",
  budgetId: "...",
});

const guardedModel = wrapLanguageModel({
  model: openai("gpt-4o-mini"),
  middleware,
});

const agent = new Agent({
  name: "my-budget-aware-agent",
  model: guardedModel,
});

// agent.generate(...) and agent.stream(...) now reserve before each
// LLM call and commit after; deny + approval flows propagate to the
// caller as typed errors.
```

## Files

| Path | Purpose |
|------|---------|
| `package.json` | `@spendguard/vercel-ai` + `ai` deps + npm scripts |
| `index.mjs` | Demo driver (ALLOW + DENY + STREAM, plus Mastra alias assertion) |
| `README.md` | This file |

## Why no real `@ai-sdk/openai`?

The official `@ai-sdk/openai` provider works fine with this
middleware — and the SLICE 6 provider matrix tests use a hand-rolled
`LanguageModelV1` that mirrors `@ai-sdk/openai@^1`'s exact surface
shape. The demo uses an in-process counting-stub-backed
`LanguageModelV1` so the docker container does not require
`OPENAI_API_KEY` to run. The middleware exercises its real
`transformParams` + `wrapGenerate` + `wrapStream` paths through this
counting-stub-backed model identically to what an
`@ai-sdk/openai`-backed model would surface — same `LanguageModelV1`
contract, same wire shape.

For production use with a real provider, swap the
`makeCountingStubModel()` call for `openai("gpt-4o-mini")` (or any
`@ai-sdk/*` provider) and you are done. The middleware works
unchanged.
