# `@spendguard/openai-agents`

> OpenAI Agents SDK (TypeScript) adapter for Agentic SpendGuard budget guardrails.
> Drop-in via `withSpendGuard(model, opts)` on any `@openai/agents` `Model` — pre-call
> budget reservation before the upstream provider HTTP call fires, post-call commit
> reconciles real token usage, signed audit trail.

## Status

`0.1.0-pre` — SLICE 1 + SLICE 2 of D08 (OpenAI Agents TS adapter). Tracks
`docs/specs/coverage/D08_openai_agents_ts/`. The first publishable release
ships at SLICE 6 once the cross-language fixture (SLICE 3) and the
end-to-end real-`@openai/agents` demo (SLICE 4-5) land.

## Quickstart (preview)

```ts
import { Agent, Runner } from "@openai/agents";
import { OpenAIChatCompletionsModel } from "@openai/agents/openai";
import { withSpendGuard, runContext } from "@spendguard/openai-agents";
import { SpendGuardClient, newUuid7 } from "@spendguard/sdk";

const client = new SpendGuardClient({ socketPath: "/run/spendguard.sock", tenantId: "tenant-prod" });
await client.connect();
await client.handshake();

const inner = new OpenAIChatCompletionsModel({ model: "gpt-4o-mini" });
const guarded = withSpendGuard(inner, { client, tenantId: "tenant-prod" });

const agent = new Agent({ name: "demo", instructions: "Reply concisely.", model: guarded });

const runId = newUuid7();
await runContext({ runId }, () => Runner.run(agent, "Say hello in three words."));
```

## What it does

For every `getResponse(request)` call the Agents Runner makes:

1. PRE: `client.reserve({ trigger: "LLM_CALL_PRE", ... })` with a deterministic
   `(decisionId, llmCallId)` derived from `(input, systemInstructions)`. A
   non-`CONTINUE` substrate outcome — DENY, STOP, SKIP, APPROVAL — throws
   the typed `@spendguard/sdk` error AND the inner model is NEVER invoked.
2. INNER: `inner.getResponse(request)` runs only on CONTINUE / DEGRADE.
   Request passed verbatim — DEGRADE mutation application is v0.2 scope.
3. POST: `client.commitEstimated({ outcome: "SUCCESS", ... })` with
   `totalTokens` from the inner response usage. Provider error → commit
   with `outcome: "PROVIDER_ERROR"` first, then re-throw.

Stream calls (`getStreamedResponse`) pass through unchanged at v0.1.x —
per-chunk gating is tracked as POST_D08.

## Type collision note

This package exports `RunContext` (`{ readonly runId: string }`) — the
per-call SpendGuard context. `@openai/agents` also exports a class called
`RunContext` (the per-run state the OpenAI Runner threads through tools).
The two are DIFFERENT concepts. Consumers who import both should alias
one of them:

```ts
import type { RunContext as SpendGuardRunContext } from "@spendguard/openai-agents";
import { RunContext } from "@openai/agents";
```

## Anti-scope

Per `docs/specs/coverage/D08_openai_agents_ts/design.md` §3 (non-goals)
+ §10 (locked decisions):

- Per-chunk stream gating — v0.2 follow-on.
- DEGRADE mutation patch application — v0.2 follow-on.
- Bundling `@openai/agents` — peer dep is intentional.
- Browser support — UDS only.
- `OpenAIResponsesModel`-specific features (mid-call adapters land in v0.2).

## License

Apache-2.0. See [LICENSE_NOTICES.md](./LICENSE_NOTICES.md) once SLICE 6
ships the third-party attribution list.
