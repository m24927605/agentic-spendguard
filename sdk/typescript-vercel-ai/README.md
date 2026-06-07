# `@spendguard/vercel-ai`

> Vercel AI SDK middleware for Agentic SpendGuard budget guardrails.
> Drop-in via `wrapLanguageModel({ model, middleware: createSpendGuardMiddleware(...) })`
> on any `@ai-sdk/*` provider — pre-call budget reservation before the
> upstream provider HTTP call fires, end-of-stream commit reconciles real
> token usage, signed audit trail. Transitively covers Mastra Agents via the
> `@spendguard/vercel-ai/mastra` subpath alias.

[![npm version](https://img.shields.io/npm/v/@spendguard/vercel-ai.svg)](https://www.npmjs.com/package/@spendguard/vercel-ai)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://github.com/m24927605/agentic-spendguard/blob/main/LICENSE)

## Status

`0.1.0` — first public release. Closes D06 (Vercel AI SDK middleware).
See [`docs/specs/coverage/D06_vercel_ai_sdk/`](https://github.com/m24927605/agentic-spendguard/tree/main/docs/specs/coverage/D06_vercel_ai_sdk)
for the locked spec set and
[`CHANGELOG.md`](./CHANGELOG.md) for the SLICE-by-SLICE feature list.

## What it does

Vercel AI SDK v4+ (`ai`) is the dominant TS-side LLM router. Mastra Agents
call `generateText` / `streamText` from `ai` underneath, so a single
middleware covers both ecosystems.

`@spendguard/vercel-ai` ships a `LanguageModelV1Middleware` factory —
`createSpendGuardMiddleware` — that you drop onto any `@ai-sdk/*` provider
via `wrapLanguageModel({ model, middleware })`. No model subclassing, no
proxy fork. The Mastra subpath (`@spendguard/vercel-ai/mastra`) re-exports
the same factory under the Mastra-idiomatic name
`createSpendGuardLanguageMiddleware` — strict function-reference equality
holds.

## Install

```bash
pnpm add @spendguard/sdk @spendguard/vercel-ai ai
```

`@spendguard/sdk`, `ai` (Vercel AI SDK), and `zod` are declared as peer
dependencies so the adapter pins none of them — your project's lockfile
wins. Node 20.10+ is required.

For real provider HTTP, add the official `@ai-sdk/*` provider you target:

```bash
pnpm add @ai-sdk/openai     # or @ai-sdk/anthropic, @ai-sdk/google, ...
```

## Quick start

```ts
import { generateText, wrapLanguageModel } from "ai";
import { openai } from "@ai-sdk/openai";
import { SpendGuardClient } from "@spendguard/sdk";
import { createSpendGuardMiddleware } from "@spendguard/vercel-ai";

const client = new SpendGuardClient({
  socketPath: "/var/run/spendguard/adapter.sock",
  tenantId: "00000000-0000-4000-8000-000000000001",
  runtimeKind: "vercel-ai-js",
});
await client.connect();
await client.handshake();

const middleware = createSpendGuardMiddleware({
  client,
  tenantId: "00000000-0000-4000-8000-000000000001",
  budgetId: "44444444-4444-4444-8444-444444444444",
});

const model = wrapLanguageModel({
  model: openai("gpt-4o-mini"),
  middleware,
});

try {
  const { text } = await generateText({ model, prompt: "hello vercel ai" });
  console.log(text);
} finally {
  await client.close();
}
```

For the Mastra-side import:

```ts
import { createSpendGuardLanguageMiddleware } from "@spendguard/vercel-ai/mastra";
```

Same factory. Same options surface. Strict `===` equality with the root
export.

## Documentation

- [Integration guide](https://agenticspendguard.dev/docs/integrations/vercel-ai/) — full walkthrough including Mastra
- [Runnable example](https://github.com/m24927605/agentic-spendguard/tree/main/examples/vercel-ai-mastra) — Node demo with ALLOW + DENY + STREAM
- [Demo overlay](https://github.com/m24927605/agentic-spendguard/tree/main/deploy/demo/vercel_ai_mastra) — `make demo-up DEMO_MODE=vercel_ai_mastra`
- [CHANGELOG](./CHANGELOG.md)
- [License notices](./LICENSE_NOTICES.md)

## License

Apache-2.0 — see the repo root
[`LICENSE`](https://github.com/m24927605/agentic-spendguard/blob/main/LICENSE)
and [`LICENSE_NOTICES.md`](./LICENSE_NOTICES.md) for third-party
attribution.
