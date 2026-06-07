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

`0.1.0-pre` — SLICE 1 (package skeleton). The full `createSpendGuardMiddleware`
factory + streaming instrumentation + Mastra subpath alias + provider matrix
land in SLICEs 2–8. See
[`docs/specs/coverage/D06_vercel_ai_sdk/`](https://github.com/m24927605/agentic-spendguard/tree/main/docs/specs/coverage/D06_vercel_ai_sdk)
for the locked spec set.

## What this is (when complete)

Vercel AI SDK v5+ is the dominant TS-side LLM router. Mastra Agents call
`generateText` / `streamText` from `ai` underneath, so a single middleware
covers both ecosystems. Shipping only the Python SpendGuard adapter would
leave the JS/TS estate unguarded.

`@spendguard/vercel-ai` ships a `LanguageModelV2Middleware` factory —
`createSpendGuardMiddleware` — that you drop onto any `@ai-sdk/*` provider
via `wrapLanguageModel({ model, middleware })`. No model subclassing, no
proxy fork. The Mastra subpath (`@spendguard/vercel-ai/mastra`) re-exports
the same factory under a function-reference alias.

## Install (preview)

```bash
pnpm add @spendguard/sdk @spendguard/vercel-ai ai
```

`@spendguard/sdk`, `ai` (Vercel AI SDK), and `zod` are declared as peer
dependencies so the adapter pins none of them — your project's lockfile
wins. Node 20.10+ is required.

## Documentation

Full integration guide, configuration reference, troubleshooting, and the
demo walkthrough will land on the docs site in SLICE 7:

- `/docs/integrations/vercel-ai-and-mastra/` — full Vercel AI SDK + Mastra
  integration guide (forthcoming)

## License

Apache-2.0 — see the repo root [`LICENSE`](https://github.com/m24927605/agentic-spendguard/blob/main/LICENSE).
Third-party notices will land in `LICENSE_NOTICES.md` in SLICE 8.
