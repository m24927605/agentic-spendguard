# D29 — Implementation

Directory layout, file responsibilities, and code skeleton for `@spendguard/inngest-agent-kit`. Pair with `design.md` (public surface) and `tests.md` (verification). Symbols imported from `@spendguard/sdk` come from D05 `design.md` §4 and are NOT re-derived. The narrative mirrors `D04_langchain_ts/implementation.md`; only Inngest-specific divergences are spelled out.

## 1. Repo layout

```
sdk/typescript/integrations/inngest-agent-kit/
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
│   ├── wrap.ts                   # wrapWithSpendGuard factory
│   ├── options.ts                # WrapOptions + ClaimEstimator types
│   ├── identity.ts               # step-identity → SpendGuard-id derivation
│   ├── extract.ts                # token-usage + provider-event-id parsers
│   └── version.ts                # auto-generated VERSION constant
└── tests/
    ├── wrap.test.ts              # factory + reserve/commit unit tests
    ├── retryDedup.test.ts        # the headline retry-dedup guarantee
    ├── errors.test.ts            # throw propagation, PROVIDER_ERROR path
    ├── identity.test.ts          # identity-derivation invariants
    ├── extract.test.ts           # usage parsers per provider shape
    ├── treeShaking.test.ts       # bundle does not pull grpc directly
    ├── _support/
    │   ├── mockSidecar.ts        # re-exports the @spendguard/sdk mock
    │   └── mockAgentKit.ts       # tiny step.ai shim that fires real-shape events
    └── e2e/
        └── inngestDev.test.ts    # runs Inngest dev runtime in-memory + stubbed fetch
```

Top-level demo at `examples/inngest-agent-kit/` (slice 5):

```
examples/inngest-agent-kit/
├── package.json
├── tsconfig.json
├── README.md
└── src/
    └── agent_real_inngest_agent_kit.ts   # demo runner
```

The Node script is launched from `deploy/demo/demo/run_demo.py` via `subprocess.run(["node", ...])`. The demo container's Dockerfile already has a Node 20 stage from D04 — D29 reuses it.

## 2. `package.json` skeleton

```json
{
  "name": "@spendguard/inngest-agent-kit",
  "version": "0.1.0",
  "description": "SpendGuard adapter for Inngest AgentKit — pre-call budget enforcement inside step.ai.",
  "license": "Apache-2.0",
  "homepage": "https://github.com/m24927605/agentic-spendguard",
  "repository": {
    "type": "git",
    "url": "https://github.com/m24927605/agentic-spendguard.git",
    "directory": "sdk/typescript/integrations/inngest-agent-kit"
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
    "size": "tsx ../../scripts/verify-size.ts --max 35kb --gz 10kb dist/index.js",
    "prepack": "pnpm run build && pnpm run size"
  },
  "peerDependencies": {
    "@spendguard/sdk": "^0.1.0",
    "@inngest/agent-kit": "^0.1.0"
  },
  "devDependencies": {
    "@biomejs/biome": "^1.9.4",
    "@inngest/agent-kit": "^0.1.0",
    "inngest": "^3.27.0",
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

Bundle budget: **35 KB minified, 10 KB gzipped** — smaller than D04 because there's no inflight Map module.

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
  /** Inngest step.id — used as both stepId and llmCallId. */
  stepId: string;
  /** Inngest's attempt counter (0 = first try, 1+ = retries). */
  attempt: number;
  /** Inngest's per-step idempotency key (if the step.ai call supplied one). */
  inngestIdempotencyKey?: string;
  /** Inngest function runId. */
  runId: string;
  /** Inngest event id (if available). */
  eventId?: string;
  /** Wrapped step.ai input shape — provider-agnostic. */
  model: unknown;
  body: unknown;
}

export type ClaimEstimator = (input: ClaimEstimatorInput) => readonly BudgetClaim[];
export type CallSignatureFn = (input: ClaimEstimatorInput) => string;

export interface WrapOptions {
  budgetId: string;
  windowInstanceId: string;
  unit: UnitRef;
  pricing: PricingFreeze;
  claimEstimator: ClaimEstimator;

  // Optional
  route?: string;                       // default "llm.call.inngest"
  callSignatureFn?: CallSignatureFn;
  claimEstimate?: ClaimEstimate;
  onApprovalRequired?: (
    err: ApprovalRequired,
    input: ClaimEstimatorInput,
  ) => Promise<DecisionOutcome | null | undefined>;
}
```

### 3.2 `src/identity.ts`

```ts
import { deriveIdempotencyKey, deriveUuidFromSignature } from "@spendguard/sdk";
import type { ClaimEstimatorInput } from "./options.js";

export interface DerivedIdentity {
  decisionId: string;
  idempotencyKey: string;
  llmCallId: string;
  stepId: string;
}

/**
 * Inngest step identity → SpendGuard identity.
 * MUST be attempt-invariant: same step.id + same inngestIdempotencyKey
 * across all retries produce the same idempotencyKey, so D05's
 * DecisionCache returns the cached outcome.
 */
export function deriveIdentity(args: {
  tenantId: string;
  sessionId: string;
  input: ClaimEstimatorInput;
}): DerivedIdentity {
  const seed = args.input.inngestIdempotencyKey ?? args.input.stepId;
  const decisionId = deriveUuidFromSignature(seed, { scope: "decision_id" });
  const stepId = args.input.stepId;
  const llmCallId = args.input.stepId;
  const idempotencyKey = deriveIdempotencyKey({
    tenantId: args.tenantId,
    sessionId: args.sessionId,
    runId: args.input.runId,
    stepId,
    llmCallId,
    trigger: "LLM_CALL_PRE",
  });
  return { decisionId, idempotencyKey, llmCallId, stepId };
}
```

The seed choice (`inngestIdempotencyKey ?? stepId`) is the design's retry-dedup contract. Both inputs are attempt-invariant by Inngest's own contract.

### 3.3 `src/wrap.ts` — skeleton

```ts
import { ApprovalRequired, type DecisionOutcome } from "@spendguard/sdk";
import type { SpendGuardClient } from "@spendguard/sdk";
import type { WrapOptions, ClaimEstimatorInput } from "./options.js";
import { deriveIdentity } from "./identity.js";
import { extractTotalTokens, extractProviderEventId } from "./extract.js";

// Narrow alias for the @inngest/agent-kit `step.ai` shape we depend on.
// We intentionally type-alias the slice instead of importing the public type
// so a minor 0.1.x churn does not break the build.
interface StepAi {
  infer<TOut = unknown>(
    name: string,
    opts: { model: unknown; body: unknown },
    runtimeCtx?: Record<string, unknown>,
  ): Promise<TOut>;
  wrap<TFn extends (...args: never[]) => Promise<unknown>>(
    name: string,
    fn: TFn,
    ...args: Parameters<TFn>
  ): Promise<Awaited<ReturnType<TFn>>>;
}

interface InngestRuntimeCtx {
  runId: string;
  eventId?: string;
  step: { id: string; attempt?: number; idempotencyKey?: string };
}

/**
 * Wrap an Inngest `step.ai` namespace so every `infer()` / `wrap()` call
 * passes through SpendGuard reserve → provider → commit transparently.
 *
 * Retry-safe: the SpendGuard idempotencyKey is derived from Inngest's own
 * step identity, so a retried step short-circuits to the cached decision.
 */
export function wrapWithSpendGuard(
  stepAi: StepAi,
  client: SpendGuardClient,
  options: WrapOptions,
): StepAi {
  const route = options.route ?? "llm.call.inngest";

  async function runReserveAndCommit<TOut>(
    name: string,
    body: () => Promise<TOut>,
    inputBuilder: () => ClaimEstimatorInput,
  ): Promise<TOut> {
    const input = inputBuilder();
    const id = deriveIdentity({
      tenantId: client.tenantId,
      sessionId: client.sessionId,
      input,
    });
    let outcome: DecisionOutcome;
    try {
      outcome = await client.reserve({
        trigger: "LLM_CALL_PRE",
        runId: input.runId,
        stepId: id.stepId,
        llmCallId: id.llmCallId,
        decisionId: id.decisionId,
        route,
        projectedClaims: [...options.claimEstimator(input)],
        idempotencyKey: id.idempotencyKey,
        claimEstimate: options.claimEstimate,
      });
    } catch (err) {
      if (err instanceof ApprovalRequired && options.onApprovalRequired) {
        const resumed = await options.onApprovalRequired(err, input);
        if (!resumed) throw err;
        outcome = resumed;
      } else {
        throw err; // halts the step body — Inngest records the step as failed
      }
    }

    try {
      const result = await body();
      const totalTokens = extractTotalTokens(result);
      const providerEventId = extractProviderEventId(result);
      await client.commitEstimated({
        runId: input.runId,
        stepId: id.stepId,
        llmCallId: id.llmCallId,
        decisionId: outcome.decisionId,
        reservationId: outcome.reservationIds[0] ?? "",
        estimatedAmountAtomic: String(totalTokens),
        unit: options.unit,
        pricing: options.pricing,
        providerEventId,
        outcome: "SUCCESS",
      });
      return result;
    } catch (err) {
      await client.commitEstimated({
        runId: input.runId,
        stepId: id.stepId,
        llmCallId: id.llmCallId,
        decisionId: outcome.decisionId,
        reservationId: outcome.reservationIds[0] ?? "",
        estimatedAmountAtomic: "0",
        unit: options.unit,
        pricing: options.pricing,
        providerEventId: "",
        outcome: "PROVIDER_ERROR",
      });
      throw err;
    }
  }

  function inputFromCtx(
    ctx: InngestRuntimeCtx | undefined,
    name: string,
    model: unknown,
    body: unknown,
  ): ClaimEstimatorInput {
    return {
      stepId: ctx?.step.id ?? name,
      attempt: ctx?.step.attempt ?? 0,
      inngestIdempotencyKey: ctx?.step.idempotencyKey,
      runId: ctx?.runId ?? "",
      eventId: ctx?.eventId,
      model,
      body,
    };
  }

  return {
    async infer(name, opts, runtimeCtx) {
      const ctx = runtimeCtx as InngestRuntimeCtx | undefined;
      return runReserveAndCommit(
        name,
        () => stepAi.infer(name, opts, runtimeCtx),
        () => inputFromCtx(ctx, name, opts.model, opts.body),
      );
    },
    async wrap(name, fn, ...args) {
      const ctx = (args[args.length - 1] as InngestRuntimeCtx | undefined);
      return runReserveAndCommit(
        name,
        () => stepAi.wrap(name, fn, ...args),
        () => inputFromCtx(ctx, name, undefined, args),
      );
    },
  };
}
```

The runtime-ctx parameter is documented in `@inngest/agent-kit@^0.1`'s `step.ai.infer` signature; the wrap forwards it untouched. If a host pattern emerges where ctx is supplied via `AsyncLocalStorage`, slice 3 adds an ALS fallback (already a published Inngest community pattern). `tests.md` §3.1 W-09 covers this.

### 3.4 `src/extract.ts`

Mirrors D04 `extract.ts`. Reads:

1. `result.usage.total_tokens` (OpenAI shape)
2. `result.usage_metadata.total_tokens` (Anthropic / Gemini shape)
3. Fallback: `result.response_metadata.token_usage.total_tokens`
4. `providerEventId`: `result.id` then `result.response_metadata.id`

Returns 0 / `""` on absence.

## 4. Demo script — `examples/inngest-agent-kit/src/agent_real_inngest_agent_kit.ts`

```ts
import { Inngest } from "inngest";
import { openai } from "@inngest/agent-kit/models";
import { SpendGuardClient } from "@spendguard/sdk";
import { wrapWithSpendGuard } from "@spendguard/inngest-agent-kit";

async function main() {
  const client = new SpendGuardClient({
    socketPath: process.env.SPENDGUARD_SIDECAR_UDS!,
    tenantId: process.env.SPENDGUARD_TENANT_ID!,
    runtimeKind: "inngest-agent-kit",
  });
  await client.connect();
  await client.handshake();

  const inngest = new Inngest({ id: "spendguard-demo" });
  const fn = inngest.createFunction(
    { id: "demo-fn", retries: Number(process.env.SPENDGUARD_DEMO_RETRIES ?? "0") },
    { event: "demo/run" },
    async ({ step }) => {
      const sgStep = wrapWithSpendGuard(step.ai, client, {
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
      return await sgStep.infer("call-openai", {
        model: openai({ model: "gpt-4o-mini" }),
        body: { messages: [{ role: "user", content: "ping" }] },
      });
    },
  );

  // Inngest in-memory dev runner (3.27+ exposes `dev.run()` for one-shot execution).
  const { run } = await import("inngest/dev");
  const res = await run(fn, { name: "demo/run", data: {} });
  console.log("[demo] result:", res);
  await client.close();
}

main().catch((e) => { console.error("[demo] FAIL:", e); process.exit(2); });
```

The `SPENDGUARD_DEMO_RETRIES` env knob drives the retry-dedup demo gate (§A5.4). With `retries=2` and a body that throws on attempts 0 + 1, Inngest will replay the step three times; SpendGuard must record exactly one `LLM_CALL_PRE` audit row.

## 5. Behaviour notes vs. D04 LangChain adapter

| Concern | D04 `SpendGuardCallbackHandler` | D29 `wrapWithSpendGuard` |
|---|---|---|
| Host primitive | LangChain `BaseCallbackHandler` event bus | Inngest `step.ai` namespace |
| Run context | LangChain `RunManager.runId` | Inngest `ctx.runId` + `ctx.step.id` |
| PRE/POST correlation | inflight `Map<runId, …>` | local variable inside one `await` — no map |
| Retry dedup | Each retry gets a fresh `runId` → fresh decision | Each retry reuses `step.id` → cached decision |
| Halt mechanism | Throw in `handleChatModelStart` | Throw in step body → Inngest fails the step |
| Idempotency key seed | `runId + ":" + signature` | `inngestIdempotencyKey ?? step.id` |
| Streaming | PRE-only, POST after final chunk | `step.ai.infer` is non-streaming; no streaming branch |

## 6. Idempotency + retry alignment

The adapter does not add its own retry — the substrate (D05 §6.5) handles `UNAVAILABLE` retries. The retry-dedup guarantee comes from D05's in-process `DecisionCache`: same `idempotencyKey` → same outcome. Because `deriveIdentity` is attempt-invariant by construction, every Inngest retry of the same step is a cache hit.

## 7. Tree-shaking + bundle hygiene

`src/index.ts` re-exports `wrapWithSpendGuard` and `WrapOptions` (type). NO re-export of `@spendguard/sdk` symbols. Bundle stays under the 35 KB / 10 KB budget.
