# D38 — Implementation

Directory layout, file responsibilities, and code skeletons for `@spendguard/mastra`. Pair with `design.md` (LOCKED surface — §5 is copied verbatim, never re-typed) and `tests.md`. Substrate symbols come from D05 `design.md` §4 and are NOT re-derived here.

## 1. Repo layout

```
sdk/typescript-mastra/
├── package.json
├── tsconfig.json
├── tsconfig.tests.json
├── tsup.config.ts
├── biome.json
├── vitest.config.ts
├── README.md
├── LICENSE_NOTICES.md
├── CHANGELOG.md
├── scripts/
│   ├── size-budget.sh            # copied from sdk/typescript-langchain/scripts
│   ├── version-check.sh
│   └── prepublish.sh
├── src/
│   ├── index.ts                  # public barrel — design.md §5 verbatim
│   ├── processor.ts              # SpendGuardProcessor (hooks per design §6)
│   ├── options.ts                # design.md §5 verbatim options block
│   ├── identity.ts               # §6.3 derivation — delegates to @spendguard/sdk
│   ├── inflight.ts               # bounded inflight correlation (§6.5)
│   ├── flatten.ts                # flattenStepText — deterministic text flatten
│   ├── usage.ts                  # usage extraction from response hook args (V4)
│   ├── errors.ts                 # re-export DecisionDenied/SidecarUnavailable/SpendGuardError
│   └── version.ts                # VERSION constant (version-check.sh keeps in sync)
└── tests/
    ├── lockedSurface.test.ts     # barrel exports, no default export, options shape
    ├── processor.test.ts         # reserve/commit lifecycle vs mock sidecar
    ├── failClosed.test.ts        # full §7 matrix
    ├── identity.test.ts          # derivation parity vs substrate (golden vectors)
    ├── inflight.test.ts
    ├── usage.test.ts
    ├── hashReuse.test.ts         # P0: no local hashing (grep src + package.json)
    ├── mastraIntegration.test.ts # real @mastra/core Agent + processor mount (V1/V2/V3/V5 pins)
    └── _support/
        ├── mockSidecar.ts        # re-exports sdk/typescript/tests/_support mock
        ├── stubModel.ts          # counting AI-SDK-shaped model for Agent tests
        └── sampleConsumer.ts     # typecheck-only consumer of the locked surface
```

Companion trees (slices 0 / 5):

```
docs/specs/coverage/D06_vercel_ai_sdk/design.md     # slice 0: dated amendment APPENDED (§9.1)
sdk/typescript-vercel-ai/package.json               # slice 0: "ai": ">=4.0.0 <5", version 0.2.0
sdk/typescript-vercel-ai/CHANGELOG.md               # slice 0: 0.2.0 entry
pnpm-workspace.yaml                                 # slice 1: + "sdk/typescript-mastra"
examples/mastra-processor/
├── package.json
├── index.mjs                                       # 3-step ALLOW + DENY + STREAM runner
└── README.md
deploy/demo/mastra_processor/docker-compose.yaml    # counting-stub + mastra-processor-runner
deploy/demo/verify_step_mastra_processor.sql        # COV_D38_GATE assertions
deploy/demo/Makefile                                # DEMO_MODE=mastra_processor branches +
                                                    # demo-verify-mastra-processor + .PHONY
```

Anti-scope (all slices): `deploy/demo/vercel_ai_mastra/**` and `deploy/demo/verify_step_vercel_ai_mastra.sql` are read-only for D38.

## 2. `package.json` skeleton

```json
{
  "name": "@spendguard/mastra",
  "version": "0.1.0",
  "description": "Mastra Processor for SpendGuard budget guardrails — hard, fail-closed, pre-dispatch budget reservation for Mastra Agents (model-router strings included)",
  "license": "Apache-2.0",
  "author": "Michael Chen <m24927605@gmail.com>",
  "homepage": "https://github.com/m24927605/agentic-spendguard",
  "repository": {
    "type": "git",
    "url": "https://github.com/m24927605/agentic-spendguard.git",
    "directory": "sdk/typescript-mastra"
  },
  "bugs": "https://github.com/m24927605/agentic-spendguard/issues",
  "keywords": ["llm", "agent", "spend", "budget", "spendguard", "mastra", "processor", "guardrails"],
  "type": "module",
  "engines": { "node": ">=22.13.0" },
  "sideEffects": false,
  "publishConfig": { "access": "public", "provenance": true },
  "files": ["dist/**/*.js", "dist/**/*.d.ts", "README.md", "LICENSE_NOTICES.md", "CHANGELOG.md"],
  "main": "./dist/index.js",
  "module": "./dist/index.js",
  "types": "./dist/index.d.ts",
  "exports": {
    ".": { "types": "./dist/index.d.ts", "import": "./dist/index.js" },
    "./package.json": "./package.json"
  },
  "scripts": {
    "build": "tsup",
    "test": "vitest run",
    "test:watch": "vitest",
    "lint": "biome check src tests",
    "format": "biome format --write src tests",
    "typecheck": "tsc --noEmit && tsc -p tsconfig.tests.json --noEmit",
    "size": "bash scripts/size-budget.sh",
    "version-check": "bash scripts/version-check.sh",
    "prepublishOnly": "bash scripts/prepublish.sh"
  },
  "peerDependencies": {
    "@mastra/core": ">=1.0.0 <2",
    "@spendguard/sdk": "workspace:*"
  },
  "devDependencies": {
    "@biomejs/biome": "^1.9.4",
    "@mastra/core": "^1.41.0",
    "@spendguard/sdk": "workspace:*",
    "@types/node": "^22.10.0",
    "tsup": "^8.3.0",
    "typescript": "^5.6.0",
    "vitest": "^2.1.0"
  }
}
```

Notes:

- Peer `@spendguard/sdk: workspace:*` mirrors the shipped `@spendguard/vercel-ai` convention verbatim (the prepublish script handles the published range, same as D06's pipeline).
- Node floor `>=22.13.0` is the Mastra 1.x requirement, NOT the D04/D06 `>=20.10` — review gate asserts nobody "harmonizes" it downward.
- `@mastra/core` is Apache-2.0 except `ee/` dirs; `LICENSE_NOTICES.md` records exactly that. No CLA/DCO concern — D38 is in-repo.
- Mastra ships weekly minors; peer range `>=1.0.0 <2` is deliberate (core is stable post-1.0; satellite packages — which we do not depend on — are the churn surface).

**Bundle budget: 40 KB minified, 12 KB gzipped** for `dist/index.js` (D04 parity — thin glue; `@mastra/core` and `@spendguard/sdk` are externalized peers). Budget breach fails the build via `size-budget.sh` wired into `prepublishOnly`.

## 3. Module skeletons

### 3.1 `src/identity.ts`

All derivation delegates to the substrate (design §6.3 — P0 hash-reuse gate):

```ts
import { deriveIdempotencyKey, deriveUuidFromSignature } from "@spendguard/sdk";

export const STEP_ID_LLM_CALL = "llm_call";
const LLM_CALL_ID_SCOPE = "mastra_llm_call_id";

export interface StepIdentity {
  runId: string;
  llmCallId: string;
  decisionId: string;
  idempotencyKey: string;
}

export function deriveStepIdentity(args: {
  tenantId: string;
  stepText: string;
  /** From opts.runIdProvider / Mastra hook context (V3); undefined → content-derived. */
  externalRunId?: string;
}): StepIdentity {
  const signature = `v1|${args.tenantId}|${args.stepText}`;
  const llmCallId = deriveUuidFromSignature(signature, { scope: LLM_CALL_ID_SCOPE });
  const runId = args.externalRunId ?? llmCallId;
  return {
    runId,
    llmCallId,
    decisionId: llmCallId,
    idempotencyKey: deriveIdempotencyKey({
      tenantId: args.tenantId,
      sessionId: runId,
      runId,
      stepId: STEP_ID_LLM_CALL,
      llmCallId,
      trigger: "LLM_CALL_PRE",
    }),
  };
}
```

No other module in the package may import `node:crypto` or any hashing library (tests/hashReuse.test.ts + review-standards §4 enforce).

### 3.2 `src/flatten.ts`

`flattenStepText(messages: unknown): string` — walks the hook-provided step messages, concatenates text parts only (string content verbatim; array content → text-typed parts), `"\n"`-joined. Mirrors D06 `flattenPromptText` + D04 `measureContentChars` discipline: images / tool-call payloads / binary parts are skipped. Deterministic byte-for-byte for identical input (identity derivation depends on it). The exact message shape at the hook is pinned by V1; `flatten.ts` is written defensively against `unknown` so a Mastra minor bump cannot throw from inside the gate.

### 3.3 `src/inflight.ts`

```ts
export interface InflightEntry {
  decisionId: string;
  reservationId: string;
  runId: string;
  llmCallId: string;
  idempotencyKey: string;
  /** Reserve-time projection — §6.6 commit-estimation fallback. */
  projectedAmountAtomic: string;
}

export class InflightMap {
  constructor(capacity?: number); // default 10_000, FIFO eviction
  push(key: string, entry: InflightEntry): void;   // key: V3 call id, else runId
  pop(key: string): InflightEntry | undefined;     // FIFO within key; deletes
  size(): number;
}
```

Per design §6.5: keyed by the V3 per-call id when COV_D38_02 pins one; LOCKED fallback keys by `runId` with FIFO-within-run pop. `pop` of an unknown key returns `undefined` (commit path warns + no-ops).

### 3.4 `src/processor.ts` — core skeleton

```ts
import type { Processor } from "@mastra/core/processors";
import {
  type BudgetClaim, type CommitEstimatedRequest, type DecisionOutcome,
  type PricingFreeze, type ReserveRequest, type SpendGuardClient, type UnitRef,
} from "@spendguard/sdk";
import { deriveStepIdentity, STEP_ID_LLM_CALL } from "./identity.js";
import { flattenStepText } from "./flatten.js";
import { InflightMap } from "./inflight.js";
import { extractUsage } from "./usage.js";
import type { SpendGuardProcessorOptions } from "./options.js";

const DEFAULT_ROUTE = "mastra-llm";
const DEFAULT_UNIT: UnitRef = { unit: "USD_MICROS", denomination: 1 };
const EMPTY_PRICING: PricingFreeze = { pricingVersion: "", pricingHash: new Uint8Array(0) };
const CHARS_PER_TOKEN_HEURISTIC = 4;
const DEFAULT_MICROS_PER_TOKEN = 1_000n;

export class SpendGuardProcessor implements Processor {
  readonly name = "spendguard-processor";
  private readonly opts: SpendGuardProcessorOptions;
  private readonly inflight = new InflightMap();

  constructor(options: SpendGuardProcessorOptions) {
    if (options === null || typeof options !== "object")
      throw new TypeError("SpendGuardProcessor: options must be an object");
    if (!options.client)
      throw new TypeError("SpendGuardProcessor: options.client is required");
    if (typeof options.tenantId !== "string" || options.tenantId.length === 0)
      throw new TypeError("SpendGuardProcessor: options.tenantId is required (non-empty string)");
    this.opts = options;
  }

  // Hook signatures conform to the installed @mastra/core Processor
  // interface — pinned by COV_D38_02 ([VERIFY-AT-IMPL: V1]). Bodies below
  // show the LOCKED control flow against placeholder arg shapes.

  async processInputStep(args /* : V1-pinned */) {
    const stepText = flattenStepText(/* step messages from args */);
    const identity = deriveStepIdentity({
      tenantId: this.opts.tenantId,
      stepText,
      externalRunId: this.opts.runIdProvider?.() ?? /* Mastra run id, V3 */ undefined,
    });
    const claims = this.opts.claimEstimator
      ? [...this.opts.claimEstimator({ stepText, runId: identity.runId, llmCallId: identity.llmCallId })]
      : [this.projectClaim(stepText)];

    const req: ReserveRequest = {
      trigger: "LLM_CALL_PRE",
      runId: identity.runId,
      stepId: STEP_ID_LLM_CALL,
      llmCallId: identity.llmCallId,
      decisionId: identity.decisionId,
      route: this.opts.route ?? DEFAULT_ROUTE,
      projectedClaims: claims,
      idempotencyKey: identity.idempotencyKey,
    };

    // FAIL-CLOSED (design §7, LOCKED): NO try/catch around reserve(). Every
    // failure — DecisionDenied, DecisionStopped, ApprovalRequired,
    // SidecarUnavailable, HandshakeError, SpendGuardError — propagates and
    // halts the step before the provider call. If V2 pins Mastra's abort()
    // as the required halt mechanism, the throw is replaced by
    // abort-with-typed-error-on-cause; the observable contract (zero
    // provider calls on failure) is identical and test-pinned (TP-10).
    const outcome: DecisionOutcome = await this.opts.client.reserve(req);

    this.inflight.push(/* V3 key ?? */ identity.runId, {
      decisionId: outcome.decisionId,
      reservationId: outcome.reservationIds[0] ?? "",
      runId: identity.runId,
      llmCallId: identity.llmCallId,
      idempotencyKey: identity.idempotencyKey,
      projectedAmountAtomic: claims[0]?.amountAtomic ?? "0",
    });
    return /* args/messages unchanged — the processor never mutates the step */;
  }

  async processLLMResponse(args /* : V1-pinned */) {
    const entry = this.inflight.pop(/* V3 key ?? runId from args */);
    if (entry === undefined) {
      console.warn("[spendguard:mastra] processLLMResponse: no inflight entry (idempotent re-delivery?)");
      return /* passthrough */;
    }
    const usage = extractUsage(args); // V4; undefined when not exposed
    const req: CommitEstimatedRequest = {
      runId: entry.runId,
      stepId: STEP_ID_LLM_CALL,
      llmCallId: entry.llmCallId,
      decisionId: entry.decisionId,
      reservationId: entry.reservationId,
      // §6.6 LOCKED fallback: actuals when exposed, else reserve-time projection.
      estimatedAmountAtomic: usage ? "0" : entry.projectedAmountAtomic,
      unit: this.buildUnit(),
      pricing: EMPTY_PRICING,
      providerEventId: usage?.providerEventId ?? "",
      outcome: "SUCCESS",
      outcomeKind: "SUCCESS",
      ...(usage
        ? {
            actualInputTokensWire: String(usage.inputTokens),
            actualOutputTokensWire: String(usage.outputTokens),
          }
        : {}),
    };
    try {
      await this.opts.client.commitEstimated(req);
    } catch (err) {
      // POST-side failure must not destroy the already-delivered provider
      // result (design §7.4). Reservation settles via TTL sweep.
      console.error(`[spendguard:mastra] commit failed; TTL sweep will settle: ${String(err)}`);
    }
    return /* passthrough */;
  }

  // processOutputStep: backstop commit + FAILURE settlement per design §6.1
  // (V4 ordering / V7 error signal — wired in COV_D38_03 with the same
  // at-most-one-commit-per-reservation guard).

  private projectClaim(stepText: string): BudgetClaim {
    const estimatedTokens = BigInt(Math.max(1, Math.ceil(stepText.length / CHARS_PER_TOKEN_HEURISTIC)));
    const cap = this.opts.defaultBudgetMicrosCap;
    const amountMicros = cap !== undefined && cap > 0n ? cap : estimatedTokens * DEFAULT_MICROS_PER_TOKEN;
    return {
      scopeId: this.opts.budgetId ?? this.opts.tenantId,
      amountAtomic: amountMicros.toString(),
      unit: this.buildUnit(),
    };
  }

  private buildUnit(): UnitRef {
    // HARDEN_D05_UR — day-1 unitId threading (design §11.5).
    return this.opts.unitId ? { ...DEFAULT_UNIT, unitId: this.opts.unitId } : DEFAULT_UNIT;
  }
}
```

### 3.5 `src/usage.ts`

`extractUsage(args: unknown): { inputTokens: number; outputTokens: number; providerEventId?: string } | undefined`. Reads the V4-pinned flat usage fields; accepts both camelCase and snake_case shapes (D04/D06 `extractTokenUsage` discipline) and tolerates non-object bags. Returns `undefined` (NOT zeros) when usage is absent so the caller selects the §6.6 estimated-amount fallback.

### 3.6 `src/errors.ts`

```ts
export { DecisionDenied, SidecarUnavailable, SpendGuardError } from "@spendguard/sdk";
```

Direct re-export (class identity preserved; `instanceof` works across the boundary). Exactly the D06 three-class anti-list; everything else imports from the substrate.

## 4. Substrate call map

| Adapter site | Substrate symbol (D05 §4 — imported, never re-derived) |
|---|---|
| `identity.ts` | `deriveIdempotencyKey`, `deriveUuidFromSignature` |
| `processor.ts` reserve | `SpendGuardClient.reserve(ReserveRequest) → DecisionOutcome` |
| `processor.ts` commit | `SpendGuardClient.commitEstimated(CommitEstimatedRequest)` (multi-event `outcomeKind` extension, same shape D04 ships) |
| `processor.ts` failure | `commitEstimated({ outcome: "PROVIDER_ERROR", outcomeKind: "FAILURE", actualErrorMessage })`; `client.release()` only if V7 pins a cancel-before-dispatch path |
| types | `BudgetClaim`, `UnitRef`, `PricingFreeze`, `ReserveRequest`, `CommitEstimatedRequest`, `DecisionOutcome`, `SpendGuardClient` |
| errors | `DecisionDenied`, `DecisionStopped`, `ApprovalRequired`, `SidecarUnavailable`, `SpendGuardError`, `HandshakeError` |

## 5. Phase-0 file changes (slice 0 — exact)

1. `docs/specs/coverage/D06_vercel_ai_sdk/design.md` — APPEND the §9.1 amendment section (dated 2026-06-10). Zero edits above the appended section.
2. `sdk/typescript-vercel-ai/package.json` — `"ai": ">=4.0.0"` → `">=4.0.0 <5"` in BOTH `peerDependencies` and (range-compatible) `devDependencies` stays `^4.0.0`; `"version": "0.1.0"` → `"0.2.0"`.
3. `sdk/typescript-vercel-ai/CHANGELOG.md` — `0.2.0` entry: peer-dep correction rationale (design §9.2 wording), pointer to `@spendguard/mastra` for Mastra users, D06-follow-on note for v5/v6 middleware.
4. Re-run: `pnpm -C sdk/typescript-vercel-ai run test` + `make demo-up DEMO_MODE=vercel_ai_mastra` + `make -C deploy/demo demo-verify-vercel-ai-mastra` — all green, zero source changes under `sdk/typescript-vercel-ai/src/`.

## 6. Demo runner — `examples/mastra-processor/index.mjs` (slice 5)

Mirrors `examples/vercel-ai-mastra/index.mjs` structure (env contract, `connectWithRetry`, counting-stub `/_count` probes, exit codes):

```
env: SPENDGUARD_SIDECAR_UDS, SPENDGUARD_TENANT_ID, SPENDGUARD_BUDGET_ID,
     SPENDGUARD_UNIT_ID, SPENDGUARD_COUNTING_STUB_URL, OPENAI_BASE_URL, OPENAI_API_KEY

client = new SpendGuardClient({ socketPath, tenantId, runtimeKind: "mastra-js" })
guard  = new SpendGuardProcessor({ client, tenantId, budgetId, unitId: process.env.SPENDGUARD_UNIT_ID })
agent  = new Agent({ model: "openai/gpt-4o-mini" /* V6; LOCKED fallback: explicit
         provider instance with baseURL at the counting-stub */,
         <processor-mount key per V5>: [guard] })

step 1 ALLOW : pre=/_count → agent.generate("ping") → post=/_count; assert post === pre+1
step 2 DENY  : denyGuard = new SpendGuardProcessor({ ...same, claimEstimator: () =>
               [{ scopeId: budgetId, amountAtomic: "2000000000", unit }] })  // > 1B hard cap
               assert agent2.generate(...) rejects with DecisionDenied (direct or on
               the cause chain, per V2); assert /_count UNCHANGED
step 3 STREAM: pre=/_count → agent.stream("count to 3") drained → assert exactly one
               reserve + one commit for the step; post === pre+1

success line (LOCKED): [demo] mastra_processor ALL 3 steps PASS (ALLOW + DENY + STREAM)
```

Compose overlay `deploy/demo/mastra_processor/docker-compose.yaml`: copy `deploy/demo/vercel_ai_mastra/docker-compose.yaml` structure verbatim with: service `mastra-processor-runner`, image **`node:22.13-bookworm-slim`**, named volume `mastra-processor-runner-modules`, `file:` dep rewrite to `/opt/spendguard/sdk/typescript` + `/opt/spendguard/sdk/typescript-mastra`, same env constants (incl. `SPENDGUARD_UNIT_ID: "66666666-6666-4666-8666-666666666666"`), same counting-stub block.

Makefile (deploy/demo/Makefile): `DEMO_MODE=mastra_processor` branches in the `demo-up` echo/compose section and the run/verify dispatch section (mirror the `vercel_ai_mastra` branches at lines ~163-178 and ~747-751); new target `demo-verify-mastra-processor` mirroring `demo-verify-langchain-ts` (ledger SQL via `verify_step_mastra_processor.sql`, cross-DB canonical_events decision/outcome check, outbox-closure check); add to `.PHONY`. The `demo-verify-all-d05-ur` master target is NOT touched (HARDEN_D05_UR scope is closed).

## 7. Tree-shaking + bundle hygiene

- `src/index.ts` re-exports ONLY the design §5 surface. No `@spendguard/sdk` type re-exports beyond the three error classes.
- tsup: ESM-only, `external: ["@mastra/core", "@spendguard/sdk"]`, no CJS artifact, `sideEffects: false`.
- `dist/index.js` must not contain `blake2`, `createHash`, `createHmac`, or an inlined copy of substrate code (hashReuse + tree-shake tests).

## 8. Slice → file mapping

| Slice | Files touched |
|---|---|
| COV_D38_00 | D06 design.md (append only), vercel-ai package.json + CHANGELOG.md |
| COV_D38_01 | sdk/typescript-mastra/{package.json,tsconfig*,tsup,biome,vitest,scripts/}, src/{index,version,errors}.ts placeholder barrel, pnpm-workspace.yaml |
| COV_D38_02 | src/{processor,options,identity,flatten,inflight}.ts, tests/{lockedSurface,identity,inflight,mastraIntegration}.test.ts |
| COV_D38_03 | src/{processor,usage}.ts, tests/{processor,usage}.test.ts |
| COV_D38_04 | tests/{failClosed,hashReuse}.test.ts + coverage top-up; NO src behavior changes beyond review fixes |
| COV_D38_05 | examples/mastra-processor/*, deploy/demo/mastra_processor/*, verify_step_mastra_processor.sql, deploy/demo/Makefile |
| COV_D38_06 | README, CHANGELOG, LICENSE_NOTICES, docs/site-v2 integrations page, repo-root README adapter row, .github/workflows/sdk-ts-mastra-publish.yml |
