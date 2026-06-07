# `@spendguard/langchain`

> LangChain.js callback handler for Agentic SpendGuard budget guardrails.
> Drop-in via `callbacks: [handler]` on any `BaseChatModel` / `BaseLLM` —
> pre-call budget reservation before the upstream provider HTTP call fires,
> end-of-stream commit reconciles real token usage, signed audit trail.

[![npm version](https://img.shields.io/npm/v/@spendguard/langchain.svg)](https://www.npmjs.com/package/@spendguard/langchain)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://github.com/m24927605/agentic-spendguard/blob/main/LICENSE)

## What this is

LangChain.js (`@langchain/core@^0.3`) is the dominant TS agent stack. Shipping
only the Python SpendGuard adapter would leave the JS/TS ecosystem unguarded
— every `ChatOpenAI` / `ChatAnthropic` / any `BaseChatModel` would call the
provider with zero pre-call refusal.

`@spendguard/langchain` ships a `BaseCallbackHandler` subclass —
`SpendGuardCallbackHandler` — that you drop onto any LangChain.js model via
`callbacks: [handler]`. No model subclassing, no proxy fork. The same handler
covers LangGraph because LangGraph builds on `BaseChatModel`.

## Install

```bash
pnpm add @spendguard/sdk @spendguard/langchain @langchain/core @langchain/openai
```

Both `@spendguard/sdk` AND `@langchain/core` are declared as peer dependencies
so the adapter pins neither — your project's lockfile wins. Node 20.10+ is
required.

## Quick start

```ts
import { ChatOpenAI } from "@langchain/openai";
import { HumanMessage } from "@langchain/core/messages";
import { SpendGuardClient } from "@spendguard/sdk";
import { SpendGuardCallbackHandler } from "@spendguard/langchain";

const client = new SpendGuardClient({
  socketPath: "/var/run/spendguard/adapter.sock",
  tenantId: "00000000-0000-4000-8000-000000000001",
  runtimeKind: "langchain-js",
});
await client.connect();
await client.handshake();

const handler = new SpendGuardCallbackHandler({
  client,
  budgetId: "44444444-4444-4444-8444-444444444444",
});

const model = new ChatOpenAI({
  model: "gpt-4o-mini",
  callbacks: [handler],
});

try {
  const res = await model.invoke([new HumanMessage("hello")]);
  console.log(res.content);
} finally {
  await client.close();
}
```

On `DecisionDenied`, the handler throws — `raiseError = true` is pinned so
the throw propagates through `CallbackManager` and halts `model.invoke()`
BEFORE the provider HTTP call.

## Documentation

Full integration guide, configuration reference, troubleshooting, and the
demo walkthrough are on the docs site:

- [`/docs/integrations/langchain-ts/`](https://agenticspendguard.dev/docs/integrations/langchain-ts/) — full LangChain.js integration guide

## Demo

```bash
make demo-up DEMO_MODE=langchain_ts
```

Boots `postgres + sidecar + langchain-runner + counting-stub` and proves the
ALLOW + DENY + STREAM matrix end-to-end against a real `ChatOpenAI`. See
[`examples/langchain-ts/`](https://github.com/m24927605/agentic-spendguard/tree/main/examples/langchain-ts)
for the runnable Node project.

## Limitations

`@spendguard/langchain` v0.1.0 ships a narrow options surface
(`client`, `tenantId?`, `budgetId?`). The fuller surface design.md §4
anticipates — `windowInstanceId`, `unit`, `pricing`, `claimEstimator`,
`route`, `callSignatureFn`, `claimEstimate`, `onApprovalRequired` —
is deferred per **D04/5 deviation #1** until the substrate broadens
`UnitRef` to carry `unit_id`. See
[`CHANGELOG.md`](./CHANGELOG.md) → "Known limitations" for the full list.

## License

Apache-2.0 — see the repo root [`LICENSE`](https://github.com/m24927605/agentic-spendguard/blob/main/LICENSE).
Third-party notices are in [`LICENSE_NOTICES.md`](./LICENSE_NOTICES.md).
