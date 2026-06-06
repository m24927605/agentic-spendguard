# D04 — Implementation

This document specifies the directory layout, file responsibilities, and code skeleton for `@spendguard/langchain`. Pair with `design.md` (public surface) and `tests.md` (verification). Surface symbols imported from `@spendguard/sdk` come from D05 `design.md` §4 and are NOT re-derived here.

## 1. Repo layout

```
sdk/typescript/integrations/langchain/
├── package.json
├── tsconfig.json
├── tsup.config.ts
├── biome.json
├── vitest.config.ts
├── README.md
├── LICENSE_NOTICES.md
├── CHANGELOG.md
├── src/
│   ├── index.ts                  # public re-exports
│   ├── handler.ts                # SpendGuardCallbackHandler
│   ├── options.ts                # SpendGuardCallbackHandlerOptions + ClaimEstimator types
│   ├── inflight.ts               # in-memory PRE→POST correlation Map
│   ├── extract.ts                # token-usage + provider-event-id parsers (per provider)
│   └── version.ts                # auto-generated VERSION constant
└── tests/
    ├── handler.test.ts           # unit tests vs. mock sidecar
    ├── streaming.test.ts         # streaming PRE + POST behaviour
    ├── errors.test.ts            # throw propagation, PROVIDER_ERROR path
    ├── treeShaking.test.ts       # no transitive grpc pull when import surface trimmed
    ├── _support/
    │   ├── mockSidecar.ts        # re-exports @spendguard/sdk's mock helper
    │   └── mockLangchain.ts      # tiny BaseChatModel subclass that fires real callback events
    └── e2e/
        └── chatOpenAI.test.ts    # uses @langchain/openai with a stubbed HTTP fetch
```

Top-level files at `examples/langchain-ts/` (slice 5):

```
examples/langchain-ts/
├── package.json
├── tsconfig.json
├── README.md
└── src/
    └── agent_real_langchain_ts.ts   # demo runner
```

The Node script is launched from the existing `deploy/demo/demo/run_demo.py` orchestrator via `subprocess` (slice 5 §3). It is **not** wrapped in the demo container's Python entrypoint — it runs as a node process inside the same compose service via a `nodejs` base layer.

## 2. `package.json` skeleton

```json
{
  "name": "@spendguard/langchain",
  "version": "0.1.0",
  "description": "SpendGuard adapter for LangChain.js — pre-call budget enforcement via BaseCallbackHandler.",
  "license": "Apache-2.0",
  "homepage": "https://github.com/m24927605/agentic-spendguard",
  "repository": {
    "type": "git",
    "url": "https://github.com/m24927605/agentic-spendguard.git",
    "directory": "sdk/typescript/integrations/langchain"
  },
  "type": "module",
  "engines": { "node": ">=20.10" },
  "sideEffects": false,
  "files": ["dist/", "README.md", "LICENSE_NOTICES.md", "CHANGELOG.md"],
  "main": "./dist/index.js",
  "types": "./dist/index.d.ts",
  "exports": {
    ".": { "types": "./dist/index.d.ts", "import": "./dist/index.js" }
  },
  "scripts": {
    "build": "tsup",
    "lint": "biome check src tests",
    "typecheck": "tsc --noEmit",
    "test": "vitest run",
    "size": "tsx ../../scripts/verify-size.ts --max 40kb --gz 12kb dist/index.js",
    "prepack": "pnpm run build && pnpm run size"
  },
  "peerDependencies": {
    "@spendguard/sdk": "^0.1.0",
    "@langchain/core": "^0.3.0"
  },
  "devDependencies": {
    "@biomejs/biome": "^1.9.4",
    "@langchain/core": "^0.3.0",
    "@langchain/openai": "^0.3.0",
    "@spendguard/sdk": "workspace:*",
    "@types/node": "^20.14.0",
    "tsup": "^8.3.0",
    "tsx": "^4.19.0",
    "typescript": "^5.6.0",
    "vitest": "^2.1.0"
  },
  "publishConfig": { "access": "public", "provenance": true }
}
```

Bundle budget: **40 KB minified, 12 KB gzipped** (smaller than D05's 120/35 — adapter is thin glue).

## 3. Type declarations (locked)

### 3.1 `src/options.ts`

```ts
import type {
  SpendGuardClient,
  BudgetClaim,
  UnitRef,
  PricingFreeze,
  DecisionOutcome,
  ClaimEstimate,
  ApprovalRequired,
} from "@spendguard/sdk";

export interface ClaimEstimatorInput {
  /** Chat or completion variant. */
  kind: "chat" | "llm";
  /** Either `messages` (chat) or `prompts` (LLM). One is always set. */
  messages?: ReadonlyArray<unknown>;  // BaseMessage[] — typed as unknown to avoid hard dep
  prompts?: ReadonlyArray<string>;
  /** LangChain RunManager runId — also used as llmCallId. */
  runId: string;
  parentRunId?: string;
  tags?: ReadonlyArray<string>;
  metadata?: Record<string, unknown>;
  /** Invocation params as supplied by the model (model name, temperature, …). */
  invocationParams?: Record<string, unknown>;
  /** `extraParams` raw from LangChain. */
  extraParams?: Record<string, unknown>;
}

export type ClaimEstimator = (input: ClaimEstimatorInput) => readonly BudgetClaim[];

export type CallSignatureFn = (input: ClaimEstimatorInput) => string;

export interface SpendGuardCallbackHandlerOptions {
  client: SpendGuardClient;
  budgetId: string;
  windowInstanceId: string;
  unit: UnitRef;
  pricing: PricingFreeze;
  claimEstimator: ClaimEstimator;

  // Optional
  route?: string;                       // default "llm.call"
  callSignatureFn?: CallSignatureFn;    // default uses @spendguard/sdk/ids
  claimEstimate?: ClaimEstimate;        // forwarded to reserve()
  /** Called when reserve() raises ApprovalRequired. If returns a resumed
   *  DecisionOutcome, that outcome continues the call; if returns null/undef,
   *  the error rethrows (caller halts). */
  onApprovalRequired?: (
    err: ApprovalRequired,
    input: ClaimEstimatorInput,
  ) => Promise<DecisionOutcome | null | undefined>;
}
```

### 3.2 `src/handler.ts` — skeleton

```ts
import { BaseCallbackHandler } from "@langchain/core/callbacks/base";
import type { Serialized } from "@langchain/core/load/serializable";
import type { BaseMessage } from "@langchain/core/messages";
import type { LLMResult } from "@langchain/core/outputs";
import {
  deriveIdempotencyKey,
  deriveUuidFromSignature,
  defaultCallSignature,
  type DecisionOutcome,
  ApprovalRequired,
} from "@spendguard/sdk";
import type {
  SpendGuardCallbackHandlerOptions,
  ClaimEstimatorInput,
} from "./options.js";
import { InflightMap } from "./inflight.js";
import { extractTotalTokens, extractProviderEventId } from "./extract.js";

export class SpendGuardCallbackHandler extends BaseCallbackHandler {
  // BaseCallbackHandler requires a static `name` for serialization.
  static lc_name() { return "SpendGuardCallbackHandler"; }
  readonly name = "spendguard_callback_handler";
  // Run handler events inline so a throw propagates synchronously.
  override readonly awaitHandlers = true;
  override readonly raiseError = true;

  private readonly opts: SpendGuardCallbackHandlerOptions;
  private readonly inflight = new InflightMap();

  constructor(opts: SpendGuardCallbackHandlerOptions) {
    super();
    this.opts = opts;
  }

  override async handleChatModelStart(
    serialized: Serialized,
    messages: BaseMessage[][],
    runId: string,
    parentRunId?: string,
    extraParams?: Record<string, unknown>,
    tags?: string[],
    metadata?: Record<string, unknown>,
    _runName?: string,
  ): Promise<void> {
    await this.reserve({
      kind: "chat",
      messages: messages[0] ?? [],
      runId, parentRunId, tags, metadata, extraParams,
      invocationParams: extraParams?.invocation_params as Record<string, unknown> | undefined,
    });
  }

  override async handleLLMStart(
    serialized: Serialized,
    prompts: string[],
    runId: string,
    parentRunId?: string,
    extraParams?: Record<string, unknown>,
    tags?: string[],
    metadata?: Record<string, unknown>,
    _runName?: string,
  ): Promise<void> {
    await this.reserve({
      kind: "llm",
      prompts,
      runId, parentRunId, tags, metadata, extraParams,
      invocationParams: extraParams?.invocation_params as Record<string, unknown> | undefined,
    });
  }

  override async handleLLMEnd(output: LLMResult, runId: string): Promise<void> {
    const pending = this.inflight.take(runId);
    if (!pending) return; // not ours
    const totalTokens = extractTotalTokens(output);
    const providerEventId = extractProviderEventId(output);
    await this.opts.client.commitEstimated({
      runId: pending.runId,
      stepId: pending.stepId,
      llmCallId: pending.llmCallId,
      decisionId: pending.outcome.decisionId,
      reservationId: pending.outcome.reservationIds[0] ?? "",
      estimatedAmountAtomic: String(totalTokens),
      unit: this.opts.unit,
      pricing: this.opts.pricing,
      providerEventId,
      outcome: "SUCCESS",
    });
  }

  override async handleLLMError(err: Error, runId: string): Promise<void> {
    const pending = this.inflight.take(runId);
    if (!pending) return;
    await this.opts.client.commitEstimated({
      runId: pending.runId,
      stepId: pending.stepId,
      llmCallId: pending.llmCallId,
      decisionId: pending.outcome.decisionId,
      reservationId: pending.outcome.reservationIds[0] ?? "",
      estimatedAmountAtomic: "0",
      unit: this.opts.unit,
      pricing: this.opts.pricing,
      providerEventId: "",
      outcome: "PROVIDER_ERROR",
    });
  }

  private async reserve(input: ClaimEstimatorInput): Promise<void> {
    const sigFn = this.opts.callSignatureFn ?? ((i) =>
      defaultCallSignature(i.messages ?? i.prompts ?? [], i.invocationParams));
    const signature = sigFn(input);
    const llmCallId = input.runId; // LangChain runId IS the call ID
    const decisionId = deriveUuidFromSignature(signature, { scope: "decision_id" });
    const stepId = `${input.runId}:lc:${signature.slice(0, 16)}`;
    const idempotencyKey = deriveIdempotencyKey({
      tenantId: this.opts.client.tenantId,
      sessionId: this.opts.client.sessionId,
      runId: input.runId,
      stepId,
      llmCallId,
      trigger: "LLM_CALL_PRE",
    });

    let outcome: DecisionOutcome;
    try {
      outcome = await this.opts.client.reserve({
        trigger: "LLM_CALL_PRE",
        runId: input.runId,
        stepId,
        llmCallId,
        decisionId,
        route: this.opts.route ?? "llm.call",
        projectedClaims: [...this.opts.claimEstimator(input)],
        idempotencyKey,
        claimEstimate: this.opts.claimEstimate,
        parentRunId: input.parentRunId,
      });
    } catch (err) {
      if (err instanceof ApprovalRequired && this.opts.onApprovalRequired) {
        const resumed = await this.opts.onApprovalRequired(err, input);
        if (!resumed) throw err;
        outcome = resumed;
      } else {
        throw err;  // halts the LangChain invoke()
      }
    }

    this.inflight.put(input.runId, {
      runId: input.runId, stepId, llmCallId, outcome,
    });
  }
}
```

### 3.3 `src/inflight.ts`

Tiny `Map`-backed correlation, keyed by `runId`. Drops entries after `take()`. Caps at 10 k entries with FIFO eviction so a forgotten `handleLLMEnd` cannot leak memory.

### 3.4 `src/extract.ts`

Mirrors `_extract_total_tokens` and `_extract_provider_event_id` in the Python adapter (`langchain.py:340-368`). Reads:

1. `output.generations[0][0].message.usage_metadata.total_tokens`
2. Fallback: `output.generations[0][0].message.response_metadata.token_usage.total_tokens`
3. `provider_event_id`: `response_metadata.id` or `response_metadata.response_id`

Returns 0 / `""` on absence — the commit still fires, just with a zero estimate (Python parity).

## 4. Demo script — `examples/langchain-ts/src/agent_real_langchain_ts.ts`

```ts
import { ChatOpenAI } from "@langchain/openai";
import { HumanMessage } from "@langchain/core/messages";
import { SpendGuardClient } from "@spendguard/sdk";
import { SpendGuardCallbackHandler } from "@spendguard/langchain";

async function main() {
  const client = new SpendGuardClient({
    socketPath: process.env.SPENDGUARD_SIDECAR_UDS!,
    tenantId: process.env.SPENDGUARD_TENANT_ID!,
    runtimeKind: "langchain-js",
  });
  await client.connect();
  await client.handshake();

  const handler = new SpendGuardCallbackHandler({
    client,
    budgetId: process.env.SPENDGUARD_BUDGET_ID!,
    windowInstanceId: process.env.SPENDGUARD_WINDOW_INSTANCE_ID!,
    unit: { unit: "USD_MICROS", denomination: 1 },
    pricing: { pricingVersion: process.env.SPENDGUARD_PRICING_VERSION! },
    claimEstimator: () => [{
      scopeId: process.env.SPENDGUARD_BUDGET_ID!,
      amountAtomic: "1000000",
      unit: { unit: "USD_MICROS", denomination: 1 },
    }],
  });

  const model = new ChatOpenAI({ model: "gpt-4o-mini", callbacks: [handler] });
  const res = await model.invoke([new HumanMessage("ping")]);
  console.log("[demo] ChatOpenAI reply:", res.content);
  await client.close();
}

main().catch((e) => { console.error("[demo] FAIL:", e); process.exit(2); });
```

The demo container's Dockerfile gets a Node 20 stage at slice 5; the orchestrator (`run_demo.py`) handles `DEMO_MODE=agent_real_langchain_ts` by spawning `node` against this script with the same env vars the Python langchain demo already passes.

## 5. Behaviour notes vs. Python adapter

| Concern | Python `SpendGuardChatModel` | TS `SpendGuardCallbackHandler` |
|---|---|---|
| Halt mechanism | `_agenerate` raises | Throw inside `handleChatModelStart` propagates because `raiseError = true` |
| Run context | `contextvars.ContextVar` + `run_context()` async-CM | LangChain `runId` (per-invoke UUID from `RunManager`) is the natural key; D05's `withRunPlan` ALS still wraps if the consumer wants Signal 3 |
| Idempotency key derivation | `derive_idempotency_key` w/ `step_id = f"{run_id}:lc-call:{sig[:16]}"` | identical — same `deriveIdempotencyKey` |
| Decision id derivation | `derive_uuid_from_signature(sig, scope="decision_id")` | identical |
| Token-usage extraction | `usage_metadata.total_tokens` → `response_metadata.token_usage.total_tokens` | identical |
| Streaming | PRE only; POST after final chunk | identical (LangChain `handleLLMNewToken` fires per chunk but is ignored; `handleLLMEnd` is the commit point) |
| DEGRADE mutation patch | surfaced as APPLY_FAILED | identical |
| Provider error path | not currently committed in Python | TS commits with `outcome="PROVIDER_ERROR"` — small improvement over Python that we will backport |

## 6. Idempotency + retry alignment

The handler does not add its own retry — the substrate (D05 §6.5) handles `UNAVAILABLE` / `DEADLINE_EXCEEDED` retries. Idempotency-key derivation is deterministic per `(tenantId, sessionId, runId, stepId, llmCallId)`; a LangChain retry that re-issues the same `runId` will hit D05's in-process `DecisionCache` and short-circuit.

When the consumer's framework retries with a NEW `runId` (LangChain's default), a fresh decision is requested — same as Python behaviour today.

## 7. Tree-shaking + bundle hygiene

`src/index.ts` re-exports `SpendGuardCallbackHandler` and `SpendGuardCallbackHandlerOptions`. NO re-export of `@spendguard/sdk` symbols — consumers import those directly. This keeps the published bundle small and prevents accidental dual-bundling of the substrate.
