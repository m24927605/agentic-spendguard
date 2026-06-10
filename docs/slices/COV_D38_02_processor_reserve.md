# COV_D38_02 — D38 Mastra adapter: SpendGuardProcessor + reserve path

> **Deliverable**: D38 Mastra dedicated adapter (`@spendguard/mastra`)
> **Slice**: 2 of 7 (M — core enforcement slice)
> **Spec set**: [`docs/specs/coverage/D38_mastra/`](../specs/coverage/D38_mastra/)
> **Precedence**: `design.md` is LOCKED and trumps this doc (review-standards §1). Any disagreement here is a slice-author bug — follow design.md and flag the drift.

## Scope

Ship the LOCKED public surface and the pre-dispatch reserve path: `SpendGuardProcessor` (design §5 class shell), `SpendGuardProcessorOptions` (design §5 verbatim, including `unitId` DAY 1), identity derivation via the substrate (design §6.3), deterministic step-text flatten, bounded inflight correlation (design §6.5), and `processInputStep` reserve wiring (design §6.2) — fail-closed, NO catch around `client.reserve()`. This slice pins V1/V2/V3/V5 against the installed `@mastra/core` (devDep `^1.41.0`) and proves DENY-before-inner-call (TP-10) with a real `@mastra/core` Agent + stub model recording zero `doGenerate`/`doStream` invocations.

Commit/failure hooks (`processLLMResponse` / `processOutputStep` bodies, `usage.ts`) are COV_D38_03. `processLLMRequest` is a no-op in v1 (design §11.3) — any reserve logic there is drift.

> NOTE-TO-ORCHESTRATOR (file-map tension, resolved by design.md precedence):
> 1. implementation.md §8 row COV_D38_02 omits `src/index.ts`, but completing the design §5 verbatim barrel (adding `SpendGuardProcessor` + option/estimator type exports to the COV_D38_01 placeholder) requires editing it. Treated as in-scope here.
> 2. tests.md §4 maps TP-01..TP-22 + TP-32..TP-35 to COV_D38_02 and acceptance.md §8 gives this slice A3.4 (runs `tests/failClosed.test.ts`) — but implementation.md §8 lists `tests/failClosed.test.ts` under COV_D38_04 and `tests/processor.test.ts` under COV_D38_03. Resolution adopted (design §13: slice 2 = "DENY-before-inner-call proven"; slice 4 = "full fail-closed matrix"): COV_D38_02 CREATES `tests/failClosed.test.ts` (reserve-path subset: TP-10, TP-13..TP-16) and `tests/processor.test.ts` (reserve subset: TP-11, TP-12, TP-17..TP-21); COV_D38_03 extends `processor.test.ts` with the commit TPs; COV_D38_04 extends `failClosed.test.ts` to the full matrix. Reviewer should treat the tests.md §4 mapping as governing which TP numbers each slice delivers.
> 3. `tests/_support/{mockSidecar,stubModel,sampleConsumer}.ts` are not listed in any implementation.md §8 row, but TP-10 needs `stubModel` + `mockSidecar` and A4.3 (`sampleConsumer`) is in this slice's acceptance subset — created here.

## Files touched

| File | Why |
|------|-----|
| `sdk/typescript-mastra/src/processor.ts` | NEW — class shell per design §5; `processInputStep` reserve per design §6 / implementation.md §3.4; `processLLMRequest` no-op |
| `sdk/typescript-mastra/src/options.ts` | NEW — design §5 verbatim options block (incl. `unitId` day 1) |
| `sdk/typescript-mastra/src/identity.ts` | NEW — implementation.md §3.1 (substrate delegation only) |
| `sdk/typescript-mastra/src/flatten.ts` | NEW — `flattenStepText` per implementation.md §3.2 |
| `sdk/typescript-mastra/src/inflight.ts` | NEW — `InflightMap` per implementation.md §3.3 / design §6.5 |
| `sdk/typescript-mastra/src/index.ts` | barrel completed to the design §5 verbatim shape (see NOTE 1) |
| `sdk/typescript-mastra/tests/lockedSurface.test.ts` | NEW — TP-01..TP-06 |
| `sdk/typescript-mastra/tests/identity.test.ts` | NEW — TP-07..TP-09 |
| `sdk/typescript-mastra/tests/inflight.test.ts` | NEW — TP-32..TP-35 |
| `sdk/typescript-mastra/tests/mastraIntegration.test.ts` | NEW — real `@mastra/core` Agent mount; V1/V2/V3/V5 pins; TP-22 |
| `sdk/typescript-mastra/tests/processor.test.ts` | NEW — reserve-path subset TP-11, TP-12, TP-17..TP-21 (see NOTE 2) |
| `sdk/typescript-mastra/tests/failClosed.test.ts` | NEW — reserve fail-closed subset TP-10, TP-13..TP-16 (see NOTE 2) |
| `sdk/typescript-mastra/tests/_support/{mockSidecar,stubModel,sampleConsumer}.ts` | NEW — see NOTE 3 |

## LOCKED surface quoted verbatim — design.md §5

> Slices copy this block exactly (§1.2-style verbatim contract). Any drift is a P0 finding (review-standards §2).

```ts
// src/index.ts — public barrel of @spendguard/mastra. Named exports only.

export { SpendGuardProcessor } from "./processor.js";
export type {
  SpendGuardProcessorOptions,
  ClaimEstimator,
  ClaimEstimatorInput,
} from "./options.js";
export { DecisionDenied, SidecarUnavailable, SpendGuardError } from "./errors.js";
export { VERSION } from "./version.js";
```

```ts
// src/options.ts — LOCKED option shape (all camelCase).

import type { BudgetClaim, SpendGuardClient } from "@spendguard/sdk";

export interface ClaimEstimatorInput {
  /** Deterministic flattened text of the step's messages (text parts only,
   *  joined with "\n" — same flatten discipline as D06 `flattenPromptText`). */
  stepText: string;
  /** Resolved run id for this step (derivation rule: design.md §6.3). */
  runId: string;
  /** Derived per-step call id (design.md §6.3). */
  llmCallId: string;
}

export type ClaimEstimator = (input: ClaimEstimatorInput) => readonly BudgetClaim[];

export interface SpendGuardProcessorOptions {
  /** Configured SpendGuardClient from @spendguard/sdk. Consumer owns the
   *  lifecycle (connect/handshake/close); the processor never closes it. */
  client: SpendGuardClient;
  /** Tenant the step bills to. REQUIRED and explicit (D06 discipline). */
  tenantId: string;
  /** Budget scope UUID for the projected claim's scopeId. Default: tenantId. */
  budgetId?: string;
  /** Ledger unit-row UUID — threads to BudgetClaim.unit.unitId on the wire.
   *  DAY-1 field (HARDEN_D05_UR). Ledger-backed reserves MUST set it;
   *  typical source is the SPENDGUARD_UNIT_ID env var at construction. */
  unitId?: string;
  /** Route label on ReserveRequest.route. Default "mastra-llm". */
  route?: string;
  /** Cap (atomic micros, bigint) used by the default claim projection when
   *  no claimEstimator is given. Mirrors D04's defaultBudgetMicrosCap. */
  defaultBudgetMicrosCap?: bigint;
  /** Custom pre-call claim projection. Default: chars/4 heuristic (§6.4). */
  claimEstimator?: ClaimEstimator;
  /** Override the run-id resolution (§6.3). Wins over Mastra-context-derived
   *  and content-derived run ids. */
  runIdProvider?: () => string;
}
```

```ts
// src/processor.ts — class shell (LOCKED shape; hook bodies per §6).

import type { Processor } from "@mastra/core/processors";
import type { SpendGuardProcessorOptions } from "./options.js";

export class SpendGuardProcessor implements Processor {
  /** Stable processor name (Mastra requires one per processor instance). */
  readonly name = "spendguard-processor";
  constructor(options: SpendGuardProcessorOptions);
}
```

Surface rules (all P0, review-standards §2) — design.md §5 verbatim:

> - `src/index.ts` exports exactly the symbols above. No `default` export. No re-export of other `@spendguard/sdk` symbols (consumers import `ApprovalRequired`, `DecisionStopped`, `HandshakeError`, etc. from the substrate directly — D06 anti-list discipline; note `DecisionStopped` / `ApprovalRequired` are subclasses of `DecisionDenied`, so `instanceof DecisionDenied` catches all denial flavours).
> - `SpendGuardProcessor implements Processor` from `@mastra/core/processors` and the package typechecks against the installed peer. **The `implements Processor` typecheck IS the hook-signature gate** — hook parameter/return types are pinned by the real package, not hand-copied into this doc. Exact installed signatures are recorded by slice `COV_D38_02` (`[VERIFY-AT-IMPL: V1]`, §12).
> - Constructor validation: missing `client` or empty `tenantId` ⇒ `TypeError` at construction (matches D06 `validateOpts`).
> - Options type contains NO fail-open field (no `failOpen`, no `degradeOnUnavailable`, no `enforcementMode`). Adding one is a P0 finding.

## LOCKED reserve semantics quoted verbatim

### Reserve request shape — design.md §6.2

```ts
const req: ReserveRequest = {
  trigger: "LLM_CALL_PRE",
  runId,                       // §6.3
  stepId: STEP_ID_LLM_CALL,    // constant "llm_call" — D04/D06 parity
  llmCallId,                   // §6.3
  decisionId: llmCallId,       // content-derived; stable across step retries
  route: opts.route ?? "mastra-llm",
  projectedClaims: [projectClaim(stepText, opts)],   // §6.4
  idempotencyKey,              // §6.3
};
```

### Identity derivation — design.md §6.3 (substrate helpers only)

```
stepText       = flattenStepText(stepMessages)        // text parts only, "\n"-joined
signature      = "v1|" + tenantId + "|" + stepText
llmCallId      = deriveUuidFromSignature(signature, { scope: "mastra_llm_call_id" })
runId          = opts.runIdProvider?.()
                 ?? <Mastra run id from hook context, when exposed — [VERIFY-AT-IMPL: V3]>
                 ?? llmCallId
decisionId     = llmCallId
idempotencyKey = deriveIdempotencyKey({
                   tenantId, sessionId: runId, runId,
                   stepId: "llm_call", llmCallId, trigger: "LLM_CALL_PRE" })
```

Properties (design §6.3 verbatim):

> - A retry of the SAME step (same accumulated messages) re-derives the same `llmCallId` → sidecar idempotency cache + ledger `UNIQUE` collapse onto the first decision.
> - Each loop step appends messages, so `stepText` differs per step → distinct `llmCallId`, `decisionId`, `idempotencyKey` per step. The multi-step agent loop is gated per step, not once per run.
> - Two byte-identical steps in the same run share a key — the same accepted trade-off D06 documents (callers wanting fresh ids salt prompts or supply `runIdProvider`).
> - ALL derivation goes through `@spendguard/sdk`. The adapter contains zero `node:crypto` / `@noble/hashes` imports (P0, review-standards §4).

### Default claim projection — design.md §6.4

> Mirrors D04/D06 exactly: `estimatedTokens = max(1, ceil(stepText.length / 4))`, `amountMicros = defaultBudgetMicrosCap > 0n ? defaultBudgetMicrosCap : estimatedTokens * 1_000n`, `scopeId = budgetId ?? tenantId`, `unit = { unit: "USD_MICROS", denomination: 1, ...(unitId ? { unitId } : {}) }`. The PRE number is a coarse probe; authoritative spend lands on the commit. `claimEstimator` overrides the whole projection.

### Inflight correlation — design.md §6.5

> - **Primary (preferred)**: if the Mastra hook args expose a stable per-call/per-step correlation id visible at BOTH `processInputStep` and `processLLMResponse`/`processOutputStep`, key the inflight map by it. `[VERIFY-AT-IMPL: V3]` — slice `COV_D38_02` pins whether such an id exists in `@mastra/core` ≥1.0.
> - **LOCKED fallback** (used when V3 finds no shared id): per-`runId` FIFO queue. Mastra's agent loop is sequential within a run (step N+1 starts after step N settles), so the response hook pops the oldest open entry for its run. Parallel agents/runs have distinct `runId`s and never cross-talk.
> - Global capacity bound 10_000 entries, FIFO eviction (D04 parity) — a hook that never fires cannot leak memory unbounded.
> - Entry carries `{ decisionId, reservationId, runId, llmCallId, idempotencyKey, projectedAmountAtomic }` — `projectedAmountAtomic` feeds the §6.6 usage fallback.

`InflightMap` class shape — implementation.md §3.3 (copy verbatim):

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

### Fail-closed abort — design.md §7 LOCKED rules 1–3 (reserve path)

> 1. **No fail-open branch anywhere.** Unlike the shipped D04/D06 adapters (which log-and-proceed on `SidecarUnavailable` per their "operational degradation" stance), `SpendGuardProcessor` propagates EVERY reserve-path error. This is a deliberate, positioning-bearing deviation (§2): in the Mastra ecosystem the fail-open niche is already occupied by `CostGuardProcessor`; D38's reason to exist is the hard gate. Any `catch`-and-continue around `client.reserve()` is a P0 finding.
> 2. **No env escape hatch.** The adapter reads NO environment variable that weakens enforcement. (`SPENDGUARD_DISABLE` exists on the substrate client for tests — the adapter neither reads nor documents it as a production path.)
> 3. **Abort mechanism**: the adapter throws the substrate typed error from `processInputStep`. `[VERIFY-AT-IMPL: V2]`: slice `COV_D38_02` MUST verify against the installed `@mastra/core` that a throw from `processInputStep` halts the step before the provider call — and if Mastra's processor runner instead requires its `abort()` mechanism to halt (TripWire-style), the adapter calls that mechanism **with the typed error preserved on the `cause` chain**. Either way the observable contract is fixed and test-pinned: **DENY ⇒ zero provider HTTP calls** (tests.md TP-10/TA-04) and the consumer can reach the typed error via the thrown error or its `cause` chain.

Identity module skeleton: implement per implementation.md §3.1 (`STEP_ID_LLM_CALL = "llm_call"`, scope string `"mastra_llm_call_id"`, `deriveStepIdentity` delegating to `deriveUuidFromSignature` + `deriveIdempotencyKey`) — copy the skeleton from implementation.md, do not re-derive.

## VERIFY-AT-IMPL pins owned by this slice (design.md §12)

Record each pin here at impl time with the answer + the exact installed `@mastra/core` version. A pin may only select between the pre-declared alternatives — never introduce a third option or weaken a LOCKED decision.

| ID | Question (design §12 verbatim) | Pre-declared alternatives (design §12 verbatim) | PIN (record at impl) |
|---|---|---|---|
| V1 | Exact `Processor` hook signatures (args object shape, async contract) for `processInputStep` / `processLLMRequest` / `processLLMResponse` / `processOutputStep` | n/a — `implements Processor` typecheck is the gate; doc records shapes | _unpinned_ |
| V2 | Does a throw from `processInputStep` halt the step pre-provider, or is the hook-provided `abort()` (TripWire) required? | throw directly / call abort() with typed error on `cause` — observable contract fixed either way (§7.3) | _unpinned_ |
| V3 | Is a stable per-call/per-step correlation id visible at both reserve and commit hooks? | key inflight by that id / LOCKED per-runId FIFO fallback (§6.5) | _unpinned_ |
| V5 | Exact Agent constructor key for mounting processors in `@mastra/core` 1.x (`inputProcessors`/`outputProcessors`/unified list) | record + use the installed key; quickstart copies it | _unpinned_ |

## Test/verification plan (tests.md §4: TP-01..TP-22, TP-32..TP-35)

| ID | One-liner |
|----|-----------|
| TP-01 | Barrel exports exactly `SpendGuardProcessor`, `DecisionDenied`, `SidecarUnavailable`, `SpendGuardError`, `VERSION` (+ type-only); no default export |
| TP-02 | `SpendGuardProcessor` satisfies the installed `Processor` type (V1 gate) |
| TP-03 | Missing `client` / empty `tenantId` → `TypeError` at construction |
| TP-04 | Options type has NO `failOpen` / `degradeOnUnavailable` / `enforcementMode` key |
| TP-05 | Re-exported error classes reference-identical (`===`) to `@spendguard/sdk`'s |
| TP-06 | `readonly name === "spendguard-processor"` |
| TP-07 | `deriveStepIdentity` equals direct substrate `deriveIdempotencyKey` call for 8 fixture tuples |
| TP-08 | Same `(tenantId, stepText)` → identical ids; differing `stepText` → all three differ |
| TP-09 | Golden vector byte-equal to Python `derive_idempotency_key` fixture (BLAKE2b P0 rides substrate) |
| TP-10 | **DENY-before-inner-call**: DENY → real Agent rejects AND stub model records ZERO `doGenerate`/`doStream` (pins V2) |
| TP-11 | Reserve wire shape: `trigger="LLM_CALL_PRE"`, `stepId="llm_call"`, default route `"mastra-llm"`, `decisionId === llmCallId` |
| TP-12 | `processInputStep` fires per step incl. tool-call continuation (1 tool call → 2 reserves) |
| TP-13 | `SidecarUnavailable` → step aborts; 0 model calls; error reachable via `instanceof` (direct or `cause`) |
| TP-14 | `DecisionStopped` / `ApprovalRequired` propagate identically (both `instanceof DecisionDenied`) |
| TP-15 | `HandshakeError` propagates; 0 model calls |
| TP-16 | No catch-and-continue in the reserve section (thrown sentinel from stubbed `reserve` always rejects the step) |
| TP-17 | `claimEstimator` called exactly once per reserve with `{stepText, runId, llmCallId}`; claims forwarded verbatim |
| TP-18 | Default projection: chars/4, `defaultBudgetMicrosCap` override, `scopeId = budgetId ?? tenantId` |
| TP-19 | **unitId threading**: set → `projectedClaims[0].unit.unitId` equals it; unset → absent from wire `UnitRef` |
| TP-20 | `runIdProvider` wins; absent → `runId === llmCallId` (or V3 context id when pinned) |
| TP-21 | Processor never mutates step messages (deep-equal before/after) |
| TP-22 | Processor mounts on a **model-router-string** Agent and `processInputStep` fires (pins V5) |
| TP-32 | InflightMap push/pop round-trip; second pop → `undefined` |
| TP-33 | FIFO-within-key pop order |
| TP-34 | Capacity 10_000 → oldest evicted |
| TP-35 | Concurrent runs (distinct runIds) never cross-correlate |

## Acceptance gates (acceptance.md §8 subset: A3.2, A3.3, A3.4, A3.6, A3.8; A4.3)

```sh
pnpm -C sdk/typescript-mastra run test tests/lockedSurface.test.ts        # A3.2 — TP-01..TP-06
pnpm -C sdk/typescript-mastra run test tests/identity.test.ts            # A3.3 — TP-07..TP-09
pnpm -C sdk/typescript-mastra run test tests/failClosed.test.ts          # A3.4 — TP-10, TP-13..TP-16 (full matrix completes in COV_D38_04)
pnpm -C sdk/typescript-mastra run test tests/inflight.test.ts            # A3.6 — TP-32..TP-35
pnpm -C sdk/typescript-mastra run test tests/mastraIntegration.test.ts   # A3.8 — TP-22 router-string mount
pnpm -C sdk/typescript-mastra run typecheck                              # A4.3 — sampleConsumer.ts constructs + mounts on a typed Agent
```

## Anti-scope (review-standards §13 row COV_D38_02)

- NO commit-path code: `processLLMResponse` / `processOutputStep` bodies, `src/usage.ts`, §6.6 fallback — COV_D38_03.
- NO reserve logic in `processLLMRequest` — no-op in v1 (design §11.3); drift is a finding.
- NO `tests/hashReuse.test.ts` / coverage top-up — COV_D38_04 (the zero-local-hashing rule still applies to all code written here).
- NO demo overlay, example runner, Makefile, or SQL — COV_D38_05. NO docs page / README content / publish workflow — COV_D38_06.
- NO fail-open knob, env escape hatch, or D04/D06 "operational degradation" copy-paste — forbidden forever (design §7, review-standards §2.7).
- NO per-chunk stream gating, auxiliary-LLM coverage, Workflow gating, tool-call PRE gating, or AI SDK v6 V3 middleware (design §4, §9.3).
- `deploy/demo/vercel_ai_mastra/**` + `verify_step_vercel_ai_mastra.sql` byte-untouched (design §9.4).

## Backlinks

- [`design.md`](../specs/coverage/D38_mastra/design.md) — §5 (verbatim surface), §6.2–§6.5, §7 (fail-closed), §11.2–§11.7, §12 (V1/V2/V3/V5), §13
- [`implementation.md`](../specs/coverage/D38_mastra/implementation.md) — §3.1–§3.4 (module skeletons), §8 (slice → file map)
- [`tests.md`](../specs/coverage/D38_mastra/tests.md) — §2 (TP-01..TP-22, TP-32..TP-35), §4
- [`acceptance.md`](../specs/coverage/D38_mastra/acceptance.md) — §3 (A3.2/A3.3/A3.4/A3.6/A3.8), §4 (A4.3), §8
- [`review-standards.md`](../specs/coverage/D38_mastra/review-standards.md) — §2 (fail-closed P0), §3 (surface lock P0), §4 (hash-reuse P0), §5 (unitId P0), §6 (Mastra protocol), §9 (inflight), §13
