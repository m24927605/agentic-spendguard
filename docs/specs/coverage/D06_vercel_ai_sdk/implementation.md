# D06 — Implementation

Concrete module layout, key types, and code skeletons. Slice authors should not re-litigate names or signatures — they are locked here.

## 1. Package layout

```
sdk/typescript-vercel-ai/
├── package.json                 # "@spendguard/vercel-ai", ESM-only, peer-dep on @spendguard/sdk + ai
├── tsconfig.json                # extends D05's tsconfig.base.json
├── tsup.config.ts               # ESM build with subpath entries
├── biome.json                   # extends repo root biome config
├── vitest.config.ts             # vitest 2.x, jsdom not needed (node target)
├── README.md                    # install + Vercel AI SDK quickstart + Mastra quickstart
├── CHANGELOG.md
├── LICENSE_NOTICES.md
├── src/
│   ├── index.ts                 # re-exports
│   ├── middleware.ts            # createSpendGuardMiddleware()
│   ├── streaming.ts             # TransformStream-based wrapStream impl
│   ├── identity.ts              # deriveCallIdentity()
│   ├── claim.ts                 # default claim estimator (chars/4 fallback)
│   ├── errors.ts                # re-export + Vercel-SDK-specific wrap helpers
│   └── mastra.ts                # alias entry — re-exports under Mastra-convention names
└── tests/
    ├── middleware.test.ts
    ├── streaming.test.ts
    ├── identity.test.ts
    ├── providers/
    │   ├── openai.test.ts
    │   └── anthropic.test.ts
    ├── mastra/
    │   └── agent.test.ts
    └── _support/
        ├── mockSidecar.ts           # @grpc/grpc-js mock server (UDS)
        ├── recordedResponses.ts     # JSON fixtures of provider responses
        └── makeMockLanguageModel.ts # in-memory LanguageModelV2 fixture
```

## 2. `package.json` highlights

```jsonc
{
  "name": "@spendguard/vercel-ai",
  "version": "0.1.0",
  "type": "module",
  "license": "Apache-2.0",
  "sideEffects": false,
  "exports": {
    ".":            { "import": "./dist/index.js",      "types": "./dist/index.d.ts" },
    "./mastra":     { "import": "./dist/mastra.js",     "types": "./dist/mastra.d.ts" },
    "./streaming":  { "import": "./dist/streaming.js",  "types": "./dist/streaming.d.ts" }
  },
  "peerDependencies": {
    "@spendguard/sdk": "^0.1",
    "ai": "^5.0.0"
  },
  "peerDependenciesMeta": {
    "@opentelemetry/api": { "optional": true }
  },
  "devDependencies": {
    "ai": "^5.0.0",
    "@ai-sdk/openai": "^1.0.0",
    "@ai-sdk/anthropic": "^1.0.0",
    "@mastra/core": "^0.x",
    "@spendguard/sdk": "workspace:*"
  }
}
```

## 3. Core types — `src/middleware.ts`

```ts
import type {
  LanguageModelV2,
  LanguageModelV2Middleware,
  LanguageModelV2CallOptions,
  LanguageModelV2FinishReason,
  LanguageModelV2StreamPart,
} from "@ai-sdk/provider";
import {
  SpendGuardClient,
  type DecisionOutcome,
  type BudgetClaim,
  type UnitRef,
  type PricingFreeze,
  DecisionDenied,
  MutationApplyFailed,
} from "@spendguard/sdk";
import { deriveCallIdentity, type CallIdentity } from "./identity.js";
import { instrumentStream } from "./streaming.js";

export interface SpendGuardMiddlewareOptions {
  client: SpendGuardClient;
  budgetId: string;
  windowInstanceId: string;
  unit: UnitRef;
  pricing: PricingFreeze;
  claimEstimator?: (params: LanguageModelV2CallOptions) => BudgetClaim[];
  callSignature?: (params: LanguageModelV2CallOptions) => string;
  runIdProvider?: () => string;
  route?: string;
  providerEventIdExtractor?: (result: { response?: unknown }) => string;
}

interface StashEntry {
  identity: CallIdentity;
  outcome: DecisionOutcome;
  runId: string;
  parentRunId: string;
  budgetGrantJti: string;
  traceparent: string;
  tracestate: string;
  route: string;
}

const STASH = new WeakMap<LanguageModelV2CallOptions, StashEntry>();

export function createSpendGuardMiddleware(
  opts: SpendGuardMiddlewareOptions,
): LanguageModelV2Middleware {
  validateOpts(opts);
  const route = opts.route ?? "llm.call";
  const claimEstimator = opts.claimEstimator ?? defaultClaimEstimator(opts);
  const providerEventIdExtractor =
    opts.providerEventIdExtractor ?? (() => "");

  return {
    transformParams: async ({ type, params }) => {
      const runCtx = resolveRunContext(opts);
      const identity = deriveCallIdentity(params, runCtx, opts);
      const projectedClaims = claimEstimator(params);

      let outcome: DecisionOutcome;
      try {
        outcome = await opts.client.reserve({
          trigger: "LLM_CALL_PRE",
          runId: runCtx.runId,
          stepId: identity.stepId,
          llmCallId: identity.llmCallId,
          decisionId: identity.traceDecisionId,
          route,
          projectedClaims,
          idempotencyKey: identity.idempotencyKey,
          traceparent: runCtx.traceparent,
          tracestate: runCtx.tracestate,
          parentRunId: runCtx.parentRunId,
          budgetGrantJti: runCtx.budgetGrantJti,
          projectedUnit: opts.unit,
          promptText: flattenPromptText(params),
        });
      } catch (err) {
        // Sidecar denial surfaces as DecisionDenied / ApprovalRequired —
        // re-throw so wrapLanguageModel's caller sees the typed error.
        throw err;
      }

      STASH.set(params, {
        identity,
        outcome,
        runId: runCtx.runId,
        parentRunId: runCtx.parentRunId,
        budgetGrantJti: runCtx.budgetGrantJti,
        traceparent: runCtx.traceparent,
        tracestate: runCtx.tracestate,
        route,
      });
      return params;
    },

    wrapGenerate: async ({ doGenerate, params }) => {
      const entry = requireStash(params);
      try {
        const result = await doGenerate();
        await commitOnSuccess(opts, entry, {
          totalTokens: extractTotalTokens(result),
          providerEventId: providerEventIdExtractor(result),
        });
        return result;
      } catch (err) {
        await rollbackOnFailure(opts, entry, err);
        throw err;
      }
    },

    wrapStream: async ({ doStream, params }) => {
      const entry = requireStash(params);
      const inner = await doStream();
      const instrumented = instrumentStream(inner.stream, {
        onFinish: async ({ totalTokens, providerEventId }) => {
          await commitOnSuccess(opts, entry, { totalTokens, providerEventId });
        },
        onError: async (err) => {
          await rollbackOnFailure(opts, entry, err);
        },
        providerEventIdExtractor,
      });
      return { ...inner, stream: instrumented };
    },
  };
}

function validateOpts(opts: SpendGuardMiddlewareOptions): void {
  if (!opts.client) throw new Error("createSpendGuardMiddleware: client is required");
  if (!opts.budgetId) throw new Error("createSpendGuardMiddleware: budgetId is required");
  if (!opts.windowInstanceId)
    throw new Error("createSpendGuardMiddleware: windowInstanceId is required");
  if (!opts.unit) throw new Error("createSpendGuardMiddleware: unit is required");
  if (!opts.pricing) throw new Error("createSpendGuardMiddleware: pricing is required");
}

function requireStash(params: LanguageModelV2CallOptions): StashEntry {
  const entry = STASH.get(params);
  if (!entry) {
    throw new Error(
      "spendguard middleware: wrapGenerate/wrapStream called without prior transformParams; " +
        "did you forget to compose via wrapLanguageModel()?",
    );
  }
  return entry;
}
```

## 4. Streaming instrumentation — `src/streaming.ts`

```ts
import type { LanguageModelV2StreamPart } from "@ai-sdk/provider";

interface InstrumentOpts {
  onFinish: (args: { totalTokens: number; providerEventId: string }) => Promise<void>;
  onError: (err: unknown) => Promise<void>;
  providerEventIdExtractor: (result: { response?: unknown }) => string;
}

export function instrumentStream(
  inner: ReadableStream<LanguageModelV2StreamPart>,
  opts: InstrumentOpts,
): ReadableStream<LanguageModelV2StreamPart> {
  let terminal = false; // commit | release race guard
  let lastUsageTokens = 0;
  let lastResponseMeta: unknown = null;

  const transform = new TransformStream<LanguageModelV2StreamPart, LanguageModelV2StreamPart>({
    transform(part, controller) {
      // Always forward to consumer first.
      controller.enqueue(part);

      // Track usage as it accumulates.
      if (part.type === "finish") {
        lastUsageTokens =
          (part.usage?.inputTokens ?? 0) + (part.usage?.outputTokens ?? 0);
        lastResponseMeta = part;
      }
    },
    async flush() {
      if (terminal) return;
      terminal = true;
      try {
        await opts.onFinish({
          totalTokens: lastUsageTokens,
          providerEventId: opts.providerEventIdExtractor({ response: lastResponseMeta }),
        });
      } catch {
        // commit-side failure must not corrupt the stream; rely on sidecar TTL.
      }
    },
  });

  // Wire error path: if inner stream errors, mirror to consumer + fire onError once.
  const piped = inner.pipeThrough(transform);
  return new ReadableStream<LanguageModelV2StreamPart>({
    async start(controller) {
      const reader = piped.getReader();
      try {
        for (;;) {
          const { value, done } = await reader.read();
          if (done) break;
          controller.enqueue(value);
        }
        controller.close();
      } catch (err) {
        if (!terminal) {
          terminal = true;
          await opts.onError(err);
        }
        controller.error(err);
      }
    },
    async cancel(reason) {
      if (!terminal) {
        terminal = true;
        await opts.onError(reason ?? new Error("stream cancelled"));
      }
    },
  });
}
```

## 5. Identity derivation — `src/identity.ts`

Mirrors `pydantic_ai.py::_derive_call_identity` byte-for-byte using D05's `deriveIdempotencyKey` + `deriveUuidFromSignature` + `defaultCallSignature`.

```ts
import {
  defaultCallSignature,
  deriveIdempotencyKey,
  deriveUuidFromSignature,
} from "@spendguard/sdk";
import type { LanguageModelV2CallOptions } from "@ai-sdk/provider";
import type { SpendGuardClient } from "@spendguard/sdk";

export interface CallIdentity {
  signature: string;
  stepId: string;
  llmCallId: string;
  traceDecisionId: string;
  idempotencyKey: string;
}

interface RunCtx {
  runId: string;
  parentRunId: string;
  budgetGrantJti: string;
  traceparent: string;
  tracestate: string;
}

export function deriveCallIdentity(
  params: LanguageModelV2CallOptions,
  ctx: RunCtx,
  opts: { client: SpendGuardClient; callSignature?: (p: LanguageModelV2CallOptions) => string },
): CallIdentity {
  const signature = (opts.callSignature ?? defaultParamsSignature)(params);
  const stepId = `${ctx.runId}:call:${signature.slice(0, 16)}`;
  const llmCallId = deriveUuidFromSignature(signature, { scope: "llm_call_id" });
  const traceDecisionId = deriveUuidFromSignature(signature, { scope: "trace_decision_id" });
  const idempotencyKey = deriveIdempotencyKey({
    tenantId: opts.client.tenantId,
    sessionId: opts.client.sessionId,
    runId: ctx.runId,
    stepId,
    llmCallId,
    trigger: "LLM_CALL_PRE",
  });
  return { signature, stepId, llmCallId, traceDecisionId, idempotencyKey };
}

function defaultParamsSignature(params: LanguageModelV2CallOptions): string {
  // Canonicalise prompt + modelSettings via D05's defaultCallSignature
  // contract: callers passing custom signatures replace this entirely.
  return defaultCallSignature(params.prompt as unknown[], {
    temperature: params.temperature,
    maxOutputTokens: params.maxOutputTokens,
    topP: params.topP,
    topK: params.topK,
    presencePenalty: params.presencePenalty,
    frequencyPenalty: params.frequencyPenalty,
    seed: params.seed,
    stopSequences: params.stopSequences,
    responseFormat: params.responseFormat,
  });
}
```

## 6. Default claim estimator — `src/claim.ts`

```ts
import type { BudgetClaim, UnitRef } from "@spendguard/sdk";
import type { LanguageModelV2CallOptions } from "@ai-sdk/provider";

export function defaultClaimEstimator(opts: {
  budgetId: string;
  windowInstanceId: string;
  unit: UnitRef;
}): (params: LanguageModelV2CallOptions) => BudgetClaim[] {
  return (params) => {
    const promptChars = flattenPromptText(params).length;
    const estimatedInputTokens = Math.ceil(promptChars / 4);
    const estimatedOutputTokens = params.maxOutputTokens ?? 256;
    return [
      {
        budgetId: opts.budgetId,
        windowInstanceId: opts.windowInstanceId,
        unit: opts.unit,
        atomicAmount: String(estimatedInputTokens + estimatedOutputTokens),
      },
    ];
  };
}

export function flattenPromptText(params: LanguageModelV2CallOptions): string {
  // params.prompt is a normalised array of v5 LanguageModelV2Message[]
  // — flatten all text + system parts deterministically.
  const parts: string[] = [];
  for (const msg of params.prompt) {
    if (msg.role === "system") {
      parts.push(msg.content);
      continue;
    }
    if (typeof msg.content === "string") {
      parts.push(msg.content);
      continue;
    }
    for (const c of msg.content) {
      if (c.type === "text") parts.push(c.text);
    }
  }
  return parts.join("\n");
}
```

## 7. Mastra alias — `src/mastra.ts`

```ts
export {
  createSpendGuardMiddleware as createSpendGuardLanguageMiddleware,
  type SpendGuardMiddlewareOptions as SpendGuardLanguageMiddlewareOptions,
} from "./middleware.js";
```

Mastra users do:

```ts
import { Agent } from "@mastra/core/agent";
import { createSpendGuardLanguageMiddleware } from "@spendguard/vercel-ai/mastra";
import { wrapLanguageModel } from "ai";

const agent = new Agent({
  name: "guarded",
  model: wrapLanguageModel({
    model: openai("gpt-4o-mini"),
    middleware: createSpendGuardLanguageMiddleware({...}),
  }),
  instructions: "...",
});
```

## 8. Commit + rollback paths

```ts
async function commitOnSuccess(
  opts: SpendGuardMiddlewareOptions,
  entry: StashEntry,
  result: { totalTokens: number; providerEventId: string },
): Promise<void> {
  if (!entry.outcome.reservationIds.length) {
    await opts.client.confirmPublishOutcome({
      decisionId: entry.outcome.decisionId,
      effectHash: entry.outcome.effectHash,
      outcome: "APPLIED_NOOP",
    });
    return;
  }
  await opts.client.emitLlmCallPost({
    runId: entry.runId,
    stepId: entry.identity.stepId,
    llmCallId: entry.identity.llmCallId,
    decisionId: entry.outcome.decisionId,
    reservationId: entry.outcome.reservationIds[0],
    providerReportedAmountAtomic: "",
    estimatedAmountAtomic: String(result.totalTokens),
    unit: opts.unit,
    pricing: opts.pricing,
    providerEventId: result.providerEventId,
    outcome: "SUCCESS",
    traceparent: entry.traceparent,
    tracestate: entry.tracestate,
  });
  await opts.client.confirmPublishOutcome({
    decisionId: entry.outcome.decisionId,
    effectHash: entry.outcome.effectHash,
    outcome: "APPLIED_NOOP",
  });
}

async function rollbackOnFailure(
  opts: SpendGuardMiddlewareOptions,
  entry: StashEntry,
  err: unknown,
): Promise<void> {
  if (!entry.outcome.reservationIds.length) {
    await opts.client.safeConfirmApplyFailed({
      decisionId: entry.outcome.decisionId,
      effectHash: entry.outcome.effectHash,
      adapterError: String(err),
    });
    return;
  }
  for (const reservationId of entry.outcome.reservationIds) {
    await opts.client.release({
      reservationId,
      decisionId: entry.outcome.decisionId,
      runId: entry.runId,
      stepId: entry.identity.stepId,
      llmCallId: entry.identity.llmCallId,
      reasonCode: "PROVIDER_ERROR",
    });
  }
}
```

## 9. RunPlan integration

When `runIdProvider` is not passed, the middleware reads from D05's `currentRunPlan()` (AsyncLocalStorage). If neither is set, the middleware throws a `SpendGuardConfigError` on the first `transformParams` call — fail fast so misconfigured runs never silently emit unparented audit events.

```ts
function resolveRunContext(opts: SpendGuardMiddlewareOptions): RunCtx {
  if (opts.runIdProvider) {
    return { runId: opts.runIdProvider(), parentRunId: "", budgetGrantJti: "", traceparent: "", tracestate: "" };
  }
  const plan = currentRunPlan();
  if (plan?.runId) {
    return { runId: plan.runId, parentRunId: plan.parentRunId ?? "", budgetGrantJti: plan.budgetGrantJti ?? "", traceparent: plan.traceparent ?? "", tracestate: plan.tracestate ?? "" };
  }
  throw new SpendGuardConfigError(
    "spendguard middleware requires a runId; pass `runIdProvider` or wrap with `withRunPlan({runId})`",
  );
}
```
