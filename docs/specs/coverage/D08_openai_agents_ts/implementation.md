# D08 — Implementation

## 1. Package layout

```
sdk/typescript/packages/openai-agents/
├── package.json
├── tsconfig.json
├── tsup.config.ts
├── biome.json
├── vitest.config.ts
├── README.md
├── CHANGELOG.md
├── LICENSE_NOTICES.md
├── src/
│   ├── index.ts                # re-exports
│   ├── withSpendGuard.ts       # factory entry
│   ├── model.ts                # SpendGuardAgentsModel class form
│   ├── core.ts                 # shared bracketing logic (used by both)
│   ├── signature.ts            # blake2b16 input fingerprint
│   ├── usage.ts                # totalTokens extraction
│   ├── runContext.ts           # AsyncLocalStorage shared with D04/D06/D29
│   └── defaultEstimator.ts     # ClaimEstimator built from inner.model name
├── tests/
│   ├── withSpendGuard.test.ts
│   ├── model.test.ts
│   ├── runContext.test.ts
│   ├── signature.test.ts
│   ├── usage.test.ts
│   ├── defaultEstimator.test.ts
│   ├── crossLanguageSignature.test.ts
│   └── _support/
│       ├── mockClient.ts        # in-process SpendGuardClient double
│       └── mockInnerModel.ts    # in-process Model double
└── dist/                        # tsup output (gitignored)
```

## 2. `package.json` (key fields)

```jsonc
{
  "name": "@spendguard/openai-agents",
  "version": "0.1.0",
  "description": "SpendGuard adapter for the OpenAI Agents SDK (TypeScript).",
  "type": "module",
  "sideEffects": false,
  "license": "Apache-2.0",
  "repository": { "type": "git", "url": "https://github.com/m24927605/agentic-spendguard.git",
                  "directory": "sdk/typescript/packages/openai-agents" },
  "exports": {
    ".":              { "import": "./dist/index.js",        "types": "./dist/index.d.ts" },
    "./model":        { "import": "./dist/model.js",        "types": "./dist/model.d.ts" },
    "./run-context":  { "import": "./dist/runContext.js",   "types": "./dist/runContext.d.ts" }
  },
  "files": ["dist", "README.md", "LICENSE_NOTICES.md", "CHANGELOG.md"],
  "engines": { "node": ">=20.10" },
  "peerDependencies": {
    "@openai/agents": ">=0.3 <1",
    "@spendguard/sdk": "^0.1.0"
  },
  "peerDependenciesMeta": {
    "@openai/agents": { "optional": false }
  },
  "dependencies": {
    "@noble/hashes": "^1.5.0"
  },
  "devDependencies": {
    "@biomejs/biome": "^1.9.0",
    "@openai/agents": "^0.3.0",
    "@spendguard/sdk": "workspace:*",
    "tsup": "^8.3.0",
    "typescript": "^5.5.0",
    "vitest": "^2.1.0"
  },
  "scripts": {
    "build":     "tsup",
    "lint":      "biome check src tests",
    "typecheck": "tsc --noEmit",
    "test":      "vitest run --coverage",
    "size":      "node ../../scripts/size-check.mjs dist/index.js 60 18",
    "prepack":   "pnpm run build && pnpm run size"
  }
}
```

Size budget: **≤ 60 KB minified, ≤ 18 KB gzipped** (substantially smaller than D05's substrate because the bulk of the gRPC/proto code lives in the peer dep).

## 3. `tsup.config.ts`

```ts
import { defineConfig } from "tsup";

export default defineConfig({
  entry: {
    index:      "src/index.ts",
    model:      "src/model.ts",
    runContext: "src/runContext.ts",
  },
  format: ["esm"],
  dts: true,
  splitting: false,
  sourcemap: true,
  clean: true,
  treeshake: true,
  target: "node20",
  external: ["@openai/agents", "@spendguard/sdk"],
});
```

## 4. `src/index.ts`

```ts
export { withSpendGuard } from "./withSpendGuard.js";
export { SpendGuardAgentsModel } from "./model.js";
export {
  runContext,
  currentRunContext,
  type RunContext,
} from "./runContext.js";
export type {
  SpendGuardModelOptions,
  ClaimEstimator,
} from "./core.js";
```

## 5. `src/core.ts` — shared bracketing logic

```ts
import type { Model, ModelRequest, ModelResponse } from "@openai/agents";
import type {
  SpendGuardClient,
  BudgetClaim,
  UnitRef,
  PricingFreeze,
  DecisionOutcome,
} from "@spendguard/sdk";
import { deriveIdempotencyKey, deriveUuidFromSignature } from "@spendguard/sdk";
import { signatureOf } from "./signature.js";
import { extractTotalTokens } from "./usage.js";
import { currentRunContext } from "./runContext.js";
import { defaultClaimEstimator } from "./defaultEstimator.js";

export type ClaimEstimator = (input: unknown) => BudgetClaim[];

export interface SpendGuardModelOptions {
  client: SpendGuardClient;
  budgetId: string;
  windowInstanceId: string;
  unit: UnitRef;
  pricing: PricingFreeze;
  claimEstimator?: ClaimEstimator;
}

export interface BracketedCallArgs extends ModelRequest {
  // openai-agents Model.getResponse arg shape; passed verbatim to inner.
  systemInstructions: string | null;
  input: unknown;
  modelSettings: unknown;
  tools: unknown;
  outputSchema: unknown;
  handoffs: unknown;
  tracing: unknown;
  previousResponseId?: string;
  conversationId?: string;
  prompt?: unknown;
}

export async function bracketedGetResponse(
  inner: Model,
  args: BracketedCallArgs,
  opts: SpendGuardModelOptions,
  innerModelName: string,
): Promise<ModelResponse> {
  const ctx = currentRunContext();
  const sig = signatureOf(args.input, args.systemInstructions);
  const llmCallId  = deriveUuidFromSignature(sig, { scope: "llm_call_id" });
  const decisionId = deriveUuidFromSignature(sig, { scope: "decision_id" });
  const stepId = `${ctx.runId}:oai-call:${sig.slice(0, 16)}`;

  const idempotencyKey = deriveIdempotencyKey({
    tenantId:  opts.client.tenantId,
    sessionId: opts.client.sessionId,
    runId:     ctx.runId,
    stepId, llmCallId,
    trigger:   "LLM_CALL_PRE",
  });

  const estimator = opts.claimEstimator
    ?? defaultClaimEstimator({
         budgetId:         opts.budgetId,
         windowInstanceId: opts.windowInstanceId,
         unit:             opts.unit,
         model:            innerModelName,
       });

  // (a) PRE — throws on DENY/STOP/SKIP/APPROVAL; inner is never reached.
  const outcome: DecisionOutcome = await opts.client.reserve({
    trigger: "LLM_CALL_PRE",
    runId:   ctx.runId,
    stepId, llmCallId, decisionId,
    route:   "llm.call",
    projectedClaims: estimator(args.input),
    idempotencyKey,
  });

  // (b) Inner call — same args verbatim. Model.getResponse signature is identical
  //     across OpenAIChatCompletionsModel and OpenAIResponsesModel.
  const inner_response = await inner.getResponse(
    args.systemInstructions, args.input, args.modelSettings,
    args.tools, args.outputSchema, args.handoffs, args.tracing,
    { previousResponseId: args.previousResponseId,
      conversationId:     args.conversationId,
      prompt:             args.prompt },
  );

  // (c) POST — commit estimated usage.
  if (outcome.reservationIds.length > 0) {
    const totalTokens = extractTotalTokens(inner_response);
    const providerEventId =
      (inner_response as { responseId?: string }).responseId ?? "";

    await opts.client.commitEstimated({
      runId: ctx.runId, stepId, llmCallId,
      decisionId:                  outcome.decisionId,
      reservationId:               outcome.reservationIds[0],
      providerReportedAmountAtomic: "",
      estimatedAmountAtomic:       String(totalTokens),
      unit:    opts.unit,
      pricing: opts.pricing,
      providerEventId,
      outcome: "SUCCESS",
    });
  }

  return inner_response;
}
```

## 6. `src/withSpendGuard.ts`

```ts
import type { Model } from "@openai/agents";
import { bracketedGetResponse, type SpendGuardModelOptions } from "./core.js";

export function withSpendGuard<M extends Model>(
  inner: M,
  opts: SpendGuardModelOptions,
): Model {
  const innerModelName = (inner as { model?: string }).model ?? "";

  return {
    async getResponse(
      systemInstructions, input, modelSettings, tools,
      outputSchema, handoffs, tracing,
      { previousResponseId, conversationId, prompt } = {},
    ) {
      return bracketedGetResponse(inner, {
        systemInstructions, input, modelSettings, tools,
        outputSchema, handoffs, tracing,
        previousResponseId, conversationId, prompt,
      }, opts, innerModelName);
    },

    streamResponse(...args) {
      // POC parity with Python: pass-through, no per-chunk gating.
      return inner.streamResponse(...args);
    },

    async close() { await inner.close?.(); },

    getRetryAdvice(request) {
      return inner.getRetryAdvice?.(request);
    },
  } as Model;
}
```

## 7. `src/model.ts` — subclass form

```ts
import type { Model } from "@openai/agents";
import { bracketedGetResponse, type SpendGuardModelOptions } from "./core.js";

export class SpendGuardAgentsModel implements Model {
  private readonly inner: Model;
  private readonly opts:  SpendGuardModelOptions;
  private readonly innerModelName: string;

  constructor(opts: SpendGuardModelOptions & { inner: Model }) {
    this.inner = opts.inner;
    this.opts  = opts;
    this.innerModelName = (opts.inner as { model?: string }).model ?? "";
  }

  async getResponse(systemInstructions, input, modelSettings, tools,
                    outputSchema, handoffs, tracing,
                    { previousResponseId, conversationId, prompt } = {}) {
    return bracketedGetResponse(this.inner, {
      systemInstructions, input, modelSettings, tools, outputSchema,
      handoffs, tracing, previousResponseId, conversationId, prompt,
    }, this.opts, this.innerModelName);
  }

  streamResponse(...args) { return this.inner.streamResponse(...args); }
  async close()           { await this.inner.close?.(); }
  getRetryAdvice(request) { return this.inner.getRetryAdvice?.(request); }
}
```

## 8. `src/signature.ts`

```ts
import { blake2b } from "@noble/hashes/blake2b";
import { bytesToHex } from "@noble/hashes/utils";

export function signatureOf(input: unknown, systemInstructions: string | null): string {
  // Canonical input rendering parity with Python `repr(input)`. JSON.stringify
  // with sorted-keys via @spendguard/sdk would be ideal but the Python wrapper
  // uses `repr`, which for str / list-of-dict / None is deterministic. We
  // mirror with JSON.stringify + an `undefined` -> "None" sentinel only for
  // string inputs (list-of-Item inputs serialize as JSON in both languages
  // already — the cross-language fixture verifies this).
  const repr = typeof input === "string"
    ? `'${input.replace(/\\/g, "\\\\").replace(/'/g, "\\'")}'`
    : JSON.stringify(input);
  const text = `${repr}|${systemInstructions ?? ""}`;
  return bytesToHex(blake2b(text, { dkLen: 16 }));
}
```

Note: the Python `repr` ↔ TS rendering parity is the **only** place where TS deviates from Python's exact byte output, because Python `repr` is not portable. The cross-language fixture (slice S08_03) covers the agreed-upon canonical input shapes (string, list-of-message-dict). Mixed-type inputs route through the same canonicalisation in both Python (`__repr__` adjusted) and TS — the fixture verifies the agreement, and a P0 note in `review-standards.md` §2 calls this out.

## 9. `src/usage.ts`

```ts
import type { ModelResponse } from "@openai/agents";

export function extractTotalTokens(response: ModelResponse): number {
  const usage = (response as { usage?: { totalTokens?: number | string } }).usage;
  if (!usage) return 0;
  const total = usage.totalTokens;
  if (typeof total === "number" && Number.isFinite(total)) return total;
  if (typeof total === "string") {
    const parsed = Number(total);
    return Number.isFinite(parsed) ? parsed : 0;
  }
  return 0;
}
```

## 10. `src/runContext.ts`

```ts
import { AsyncLocalStorage } from "node:async_hooks";

export interface RunContext { readonly runId: string }

// Shared module-singleton key. D04/D06/D08/D29 all import this same module;
// pnpm dedupes so they share one AsyncLocalStorage instance.
const STORAGE_KEY = Symbol.for("@spendguard/run-context/v1");
type GlobalSlot = { [STORAGE_KEY]?: AsyncLocalStorage<RunContext> };

function storage(): AsyncLocalStorage<RunContext> {
  const slot = globalThis as GlobalSlot;
  if (!slot[STORAGE_KEY]) slot[STORAGE_KEY] = new AsyncLocalStorage();
  return slot[STORAGE_KEY];
}

export async function runContext<T>(
  ctx: RunContext,
  fn: () => Promise<T>,
): Promise<T> {
  return storage().run(ctx, fn);
}

export function currentRunContext(): RunContext {
  const ctx = storage().getStore();
  if (!ctx) {
    throw new Error(
      "@spendguard/openai-agents called outside an active runContext().\n" +
      "Wrap your Runner.run call:\n\n" +
      "    await runContext({ runId }, () => Runner.run(agent, input))\n",
    );
  }
  return ctx;
}
```

The `Symbol.for("@spendguard/run-context/v1")` keying matches the convention D04/D06/D29 will adopt — same key across sibling packages so a multi-framework agent shares one storage. D05 v0.2 will subsume this; until then each adapter ships the same 12-line module.

## 11. `src/defaultEstimator.ts`

```ts
import type { BudgetClaim, UnitRef } from "@spendguard/sdk";
import type { ClaimEstimator } from "./core.js";

interface DefaultEstimatorOptions {
  budgetId: string;
  windowInstanceId: string;
  unit: UnitRef;
  model: string;
}

// Mirrors Python sdk/python/src/spendguard/integrations/_default_estimator.py
const MODEL_BASELINE_TOKENS: Record<string, number> = {
  "gpt-4o-mini":  500,
  "gpt-4o":      1500,
  "gpt-4.1-mini": 500,
  "gpt-4.1":     1500,
  "o1":          3000,
  "o3-mini":     1500,
  "o3":          3000,
};

export function defaultClaimEstimator(opts: DefaultEstimatorOptions): ClaimEstimator {
  const baseline = MODEL_BASELINE_TOKENS[opts.model] ?? 800;
  return (_input: unknown): BudgetClaim[] => [{
    budgetId:         opts.budgetId,
    unit:             opts.unit,
    amountAtomic:     String(baseline),
    direction:        "DEBIT",
    windowInstanceId: opts.windowInstanceId,
  }];
}
```

The model table is a literal copy of the Python module's table; a cross-language test in S08_03 reads both and asserts equality.

## 12. Demo wiring (slices S08_04, S08_05)

### 12.1 `examples/openai-agents-ts-composite/`

```
examples/openai-agents-ts-composite/
├── package.json
├── tsconfig.json
├── demo.ts                 # main entry; --mock or --real
├── README.md               # parallel to Python composite README
└── dist/                   # built by pnpm
```

`demo.ts` structure (parallels Python):

```ts
import { parseArgs } from "node:util";

async function mockMain() { /* in-process MockSpendGuard transport */ }

async function realMain(args: Args) {
  const { Agent, Runner } = await import("@openai/agents");
  const { OpenAIChatCompletionsModel } = await import("@openai/agents/openai");
  const { SpendGuardClient, newUuid7 } = await import("@spendguard/sdk");
  const { withSpendGuard, runContext } = await import("@spendguard/openai-agents");

  await using client = new SpendGuardClient({ socketPath: args.socket, tenantId: args.tenant });
  await client.handshake();

  const inner   = new OpenAIChatCompletionsModel({ model: "gpt-4o-mini" });
  const guarded = withSpendGuard(inner, {
    client,
    budgetId:         args.budget,
    windowInstanceId: args.window,
    unit:             { unitId: args.unit, tokenKind: "output_token", modelFamily: "gpt-4" },
    pricing:          { pricingVersion: args.pricingVersion },
  });

  const agent = new Agent({ name: "spendguard-demo-ts", instructions: "Reply concisely.", model: guarded });
  const runId = newUuid7();
  const result = await runContext({ runId },
    () => Runner.run(agent, "Say hello in three words."),
  );
  console.log("Runner.run OK", { runId, output: result.finalOutput });
}

const { values } = parseArgs({ options: { /* socket, tenant, budget, ... */ } });
await (values.real ? realMain(values as Args) : mockMain());
```

### 12.2 `deploy/demo/demo/run_demo.py` edit

```python
async def _run_ts_demo(example: str) -> int:
    cmd = ["node", f"/spendguard/examples/{example}/dist/demo.js", "--real",
           "--socket", SIDECAR_SOCKET, "--tenant", TENANT_ID,
           "--budget", BUDGET_ID, "--window", WINDOW_ID, "--unit", UNIT_ID]
    proc = await asyncio.create_subprocess_exec(*cmd, stdout=PIPE, stderr=PIPE)
    out, err = await proc.communicate()
    sys.stdout.write(out.decode()); sys.stderr.write(err.decode())
    return proc.returncode

async def run_openai_agents_ts_mode() -> int:
    if not os.environ.get("OPENAI_API_KEY"):
        print("[demo] FATAL: OPENAI_API_KEY required for agent_real_openai_agents_ts", file=sys.stderr)
        return 1
    return await _run_ts_demo("openai-agents-ts-composite")

# Mode table:
if DEMO_MODE == "agent_real_openai_agents_ts":
    return await run_openai_agents_ts_mode()
```

### 12.3 `Makefile` + `deploy/demo/Dockerfile` edits

`Makefile`:

```make
demo-up: demo-ts-build
        $(DOCKER_COMPOSE) up --build

demo-ts-build:
        cd sdk/typescript && pnpm install --frozen-lockfile && pnpm run build
        cd examples/openai-agents-ts-composite && pnpm install --frozen-lockfile && pnpm run build
```

`deploy/demo/Dockerfile` adds a Node 20 install + copies the prebuilt `dist/` directories.

## 13. Slice → file map

| Slice | Files created / edited |
|---|---|
| S08_01 | `package.json`, `tsconfig.json`, `tsup.config.ts`, `biome.json`, `vitest.config.ts`, `README.md`, `pnpm-workspace.yaml` (edit) |
| S08_02 | `src/{index.ts,withSpendGuard.ts,model.ts,core.ts,signature.ts,usage.ts,runContext.ts,defaultEstimator.ts}` |
| S08_03 | `tests/{withSpendGuard,model,runContext,signature,usage,defaultEstimator,crossLanguageSignature}.test.ts`, `tests/_support/{mockClient,mockInnerModel}.ts`, fixture extension `sdk/fixtures/cross-language/v1.json` (edit) |
| S08_04 | `examples/openai-agents-ts-composite/{package.json,tsconfig.json,demo.ts,README.md}` |
| S08_05 | `deploy/demo/demo/run_demo.py` (edit), `Makefile` (edit), `deploy/demo/Dockerfile` (edit) |
| S08_06 | `docs/site/docs/integrations/openai-agents-ts.md`, `README.md` (edit), `sdk/typescript/packages/openai-agents/{CHANGELOG.md,LICENSE_NOTICES.md}`, `.github/workflows/sdk-ts-openai-agents-publish.yml` |
