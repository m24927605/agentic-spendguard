# `@spendguard/mastra`

> Mastra `Processor` for Agentic SpendGuard budget guardrails — **hard,
> fail-closed, pre-dispatch** budget reservation for Mastra Agents.
> `SpendGuardProcessor` reserves budget against the durable SpendGuard
> ledger BEFORE the provider call leaves the process; a sidecar DENY (or an
> unreachable sidecar) aborts the step with a typed error. There is no
> fail-open knob and no env escape hatch.

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://github.com/m24927605/agentic-spendguard/blob/main/LICENSE)

## Status

`0.1.0` — first public release. Closes coverage deliverable D38
(Mastra dedicated adapter). Locked spec set:
[`docs/specs/coverage/D38_mastra/`](https://github.com/m24927605/agentic-spendguard/tree/main/docs/specs/coverage/D38_mastra)
· feature list: [`CHANGELOG.md`](./CHANGELOG.md).

## Why a dedicated Mastra adapter

Mastra owns its own agent loop since v0.14.0 — `@mastra/core` no longer
calls `generateText` / `streamText` from `ai`. Its model-router string
syntax (`model: "openai/gpt-4o"`) resolves models internally, with **no
injection point for `wrapLanguageModel`** — the
[`@spendguard/vercel-ai`](https://www.npmjs.com/package/@spendguard/vercel-ai)
middleware (D06) cannot reach that path. D06 gates a *model instance*;
`@spendguard/mastra` gates an *agent step*: `SpendGuardProcessor` mounts on
the Agent's processor list and covers Mastra Agents regardless of whether
the model came from a router string or an explicit AI SDK instance.

### Positioning vs Mastra's `CostGuardProcessor`

Factual contrast, sourced from upstream's own documentation:

| Dimension | Mastra `CostGuardProcessor` (per its own docs) | `@spendguard/mastra` `SpendGuardProcessor` |
|---|---|---|
| Enforcement point | After cost data is observed; cost persisted **asynchronously** | **Pre-dispatch**: budget reserved BEFORE the provider call leaves the process |
| Ceiling semantics | "treat `maxCost` as a best-effort threshold, not a hard ceiling" | Hard ceiling: reservation against a durable ledger; DENY halts the step |
| Failure posture | **Fail-open** on missing context / query failure | **Fail-closed**: sidecar unreachable or DENY ⇒ step aborts with a typed error |
| Backing store | Requires OLAP observability store (DuckDB/ClickHouse; Postgres unsupported for metrics) | SpendGuard sidecar + Postgres ledger + signed audit chain (already deployed for every other SpendGuard adapter) |
| Scope | run / resource / thread, block or warn | tenant / budget / window via SpendGuard contract DSL; shared budgets across Python, LangChain, proxy, and gateway adapters |
| Cross-runtime budget | Mastra-only | Same `budget_id` enforced across every SpendGuard integration |

The two are complementary: `CostGuardProcessor` remains a good soft-warn UX
layer; `SpendGuardProcessor` is the hard enforcement layer.

## Install

```bash
pnpm add @spendguard/sdk @spendguard/mastra @mastra/core
```

`@spendguard/sdk` and `@mastra/core` (`>=1.0.0 <2`) are peer dependencies —
your project's lockfile wins. Node `>=22.13.0` is required (Mastra 1.x
floor). ESM-only.

## Quickstart

`SpendGuardProcessor` mounts via the Agent's `inputProcessors` list (the
installed `@mastra/core` 1.x mount key — it drives the reserve at
`processInputStep` and the SUCCESS commit at `processLLMResponse`). Mount
the SAME instance on `outputProcessors` too: that arms the backstop commit
at `processOutputStep` for streamed-step ordering.

### Variant A — model-router string (the path only D38 covers)

```ts
import { Agent } from "@mastra/core/agent";
import { SpendGuardClient } from "@spendguard/sdk";
import { SpendGuardProcessor } from "@spendguard/mastra";

const client = new SpendGuardClient({
  socketPath: "/var/run/spendguard/adapter.sock",
  tenantId: "00000000-0000-4000-8000-000000000001",
  runtimeKind: "mastra-js",
});
await client.connect();
await client.handshake();

const guard = new SpendGuardProcessor({
  client,
  tenantId: "00000000-0000-4000-8000-000000000001",
  budgetId: "44444444-4444-4444-8444-444444444444",
  // Ledger-backed reserves MUST set the ledger unit-row UUID:
  unitId: process.env.SPENDGUARD_UNIT_ID,
});

const agent = new Agent({
  id: "guarded-agent",
  name: "guarded-agent",
  instructions: "You are a budget-guarded assistant.",
  model: "openai/gpt-4o-mini", // Mastra model-router string — no wrapLanguageModel needed
  inputProcessors: [guard],
  outputProcessors: [guard], // same instance: arms the backstop commit
});

try {
  const result = await agent.generate("hello mastra");
  console.log(result.text);
} finally {
  await client.close();
}
```

### Variant B — explicit AI SDK model instance

The mount is identical — only `model` changes:

```ts
import { Agent } from "@mastra/core/agent";
import { SpendGuardProcessor } from "@spendguard/mastra";
import type { SpendGuardClient } from "@spendguard/sdk";
import type { MastraLanguageModel } from "@mastra/core/agent";

declare const client: SpendGuardClient; // connected + handshaken, as above
declare const myModel: MastraLanguageModel; // e.g. openai("gpt-4o-mini") from @ai-sdk/openai

const guard = new SpendGuardProcessor({
  client,
  tenantId: "00000000-0000-4000-8000-000000000001",
});

const agent = new Agent({
  id: "guarded-agent-explicit",
  name: "guarded-agent-explicit",
  instructions: "You are a budget-guarded assistant.",
  model: myModel,
  inputProcessors: [guard],
  outputProcessors: [guard],
});
```

### Catching a denial

The processor throws the `@spendguard/sdk` typed errors (`DecisionDenied`
and its subclasses, `SidecarUnavailable`, `SpendGuardError`) from the
reserve hook — the provider call never fires. **Consumer catch contract**
(Mastra 1.41.0 runs input processors inside an internal workflow that
serializes step errors — the typed error's *message* reaches the Agent
boundary, the class instance does not;
[gh #181](https://github.com/m24927605/agentic-spendguard/issues/181)):

- **At the Agent boundary** (`agent.generate()` / `agent.stream()`
  rejection): match on the error **message**, e.g.
  `/sidecar (DENY|STOP|SKIP|REQUIRE_APPROVAL)/`.
- **At the hook boundary** (your own processors / instrumentation running
  in the same processor pipeline): `instanceof DecisionDenied` holds —
  and catches all denial flavours (`DecisionStopped`, `ApprovalRequired`
  are subclasses; import them from `@spendguard/sdk`).

```ts
import { Agent } from "@mastra/core/agent";
import type { SpendGuardClient } from "@spendguard/sdk";
import { SpendGuardProcessor } from "@spendguard/mastra";

declare const client: SpendGuardClient;
declare const agent: Agent;

try {
  await agent.generate("an expensive request");
} catch (err) {
  const message = err instanceof Error ? err.message : String(err);
  if (/sidecar (DENY|STOP|SKIP|REQUIRE_APPROVAL)/.test(message)) {
    console.error("SpendGuard denied the step pre-dispatch:", message);
  } else {
    throw err;
  }
}
```

## Options

| Option | Required | What it does |
|---|---|---|
| `client` | yes | Configured `SpendGuardClient`. You own its lifecycle (`connect` / `handshake` / `close`); the processor never closes it. |
| `tenantId` | yes | Tenant the step bills to — explicit, never inferred. |
| `budgetId` | no | Budget scope UUID for the projected claim's `scopeId`. Default: `tenantId`. |
| `unitId` | no | Ledger unit-row UUID, threaded to `claim[0].unit.unitId` on the wire. Ledger-backed reserves MUST set it (typical source: `SPENDGUARD_UNIT_ID`). |
| `route` | no | Route label on `ReserveRequest.route`. Default `"mastra-llm"`. |
| `defaultBudgetMicrosCap` | no | Cap (atomic micros, `bigint`) for the default claim projection. |
| `claimEstimator` | no | Custom pre-call claim projection — replaces the default `chars/4` heuristic. Claims forward verbatim onto `ReserveRequest.projectedClaims`; this is also the only surface that carries `windowInstanceId` (set it on your claims when reserving against a specific window instance). |
| `runIdProvider` | no | Overrides run-id resolution; wins over content-derived run ids. |
| `pricing` | no | `PricingFreeze` tuple the commit path repeats back to the ledger. Production sidecars stamp reservations with the loaded bundle's pricing freeze — commits that send the empty tuple are rejected (`pricing freeze mismatch`). Source it from `SPENDGUARD_PRICING_VERSION` + `SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX` + `SPENDGUARD_FX_RATE_VERSION` + `SPENDGUARD_UNIT_CONVERSION_VERSION` (same convention as `@spendguard/langchain`). Omit only against recipe-style/no-bundle sidecars. |

There is deliberately NO `failOpen` / `degradeOnUnavailable` /
`enforcementMode` option. In the Mastra ecosystem the fail-open niche is
already occupied by `CostGuardProcessor`; this package's reason to exist is
the hard gate.

## Lifecycle

Per agent step (including tool-call continuations):

1. `processInputStep` → **RESERVE** (`LLM_CALL_PRE`) — any failure throws
   (fail-closed); the provider call never fires on DENY.
2. `processLLMResponse` → **SUCCESS commit** with provider usage actuals
   when exposed (estimated-amount fallback otherwise).
3. `processOutputStep` → backstop commit (at most one commit per
   reservation); provider errors settle via a FAILURE commit.
4. No hook fired (crash / hard abort) → the sidecar TTL sweep settles the
   open reservation. A commit RPC failure after a successful provider call
   is logged, never thrown — your already-paid-for response is delivered,
   and the TTL sweep + audit chain settle the reservation.

Streaming is bracketed whole-step: one reserve before the first chunk, one
commit after the stream completes. No per-chunk gating.

## Known limitations

> **Auxiliary LLM calls are OUT of v1 scope.** Mastra memory title
> generation, `ModerationProcessor`'s classifier call, and scorers invoke
> models outside the agent-step processor pipeline and are NOT gated by
> `SpendGuardProcessor`. Workaround: wrap those models explicitly via
> `@spendguard/vercel-ai`'s `wrapLanguageModel` middleware (D06).

- **Router strings resolve to the OpenAI Responses API** (verified against
  `@mastra/core` 1.41.0): the router path honors `OPENAI_BASE_URL`, but the
  resolved model speaks `POST /v1/responses`. If your gateway/stub only
  serves `/v1/chat/completions`, hand the Agent an explicit AI SDK model
  instance (variant B) — enforcement is identical on both paths.
- **Plain-AI-SDK usage via `withMastra()` is unsupported in v1** —
  `withMastra()` ships in the separate `@mastra/ai-sdk` package, outside
  this adapter's peer set. Use a Mastra `Agent`, or gate plain AI SDK calls
  with `@spendguard/vercel-ai`.
- **Mastra `Workflow` step gating** and **tool-call PRE gating** are v2
  candidates; `processInputStep` already gates the LLM call after each tool
  result.
- `ApprovalRequired` propagates like any denial; an approval-resume helper
  is not included in v1.

## Run the demo

```bash
make demo-up DEMO_MODE=mastra_processor
make -C deploy/demo demo-verify-mastra-processor
```

Boots postgres + sidecar + counting-stub + a real `@mastra/core` Agent
runner and proves ALLOW + DENY + STREAM end-to-end — including that the
provider stub's hit counter did NOT move on the denied step:

```text
[demo] mastra_processor ALL 3 steps PASS (ALLOW + DENY + STREAM)
```

HARD SQL gates then assert reserve/commit/deny rows, strict reserve-before-
outcome ordering, and audit-chain closure in the real ledger.

## Documentation

- [Integration guide](https://agenticspendguard.dev/docs/integrations/mastra/)
- [Runnable example](https://github.com/m24927605/agentic-spendguard/tree/main/examples/mastra-processor)
- [Demo overlay](https://github.com/m24927605/agentic-spendguard/tree/main/deploy/demo/mastra_processor)
- [CHANGELOG](./CHANGELOG.md) · [License notices](./LICENSE_NOTICES.md)

## License

Apache-2.0 — see the repository root
[`LICENSE`](https://github.com/m24927605/agentic-spendguard/blob/main/LICENSE)
and [`LICENSE_NOTICES.md`](./LICENSE_NOTICES.md) for third-party
attribution.
