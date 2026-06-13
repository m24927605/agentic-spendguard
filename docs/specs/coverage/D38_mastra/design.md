# D38 — Mastra dedicated adapter (`@spendguard/mastra`)

**Status:** Spec — LOCKED 2026-06-10.
**Parent strategy:** [`framework-coverage-2026-06.md`](../../../strategy/framework-coverage-2026-06.md), Pattern 1 (framework lifecycle-hook middleware).
**Owner sub-agent:** Frontend Developer.
**Upstream contract:** [`D05_ts_sdk_substrate/design.md`](../D05_ts_sdk_substrate/design.md) §4. D38 imports — does not re-derive — every symbol locked there.
**Supersedes:** the "covers Mastra" transitive-coverage claim of [`D06_vercel_ai_sdk/design.md`](../D06_vercel_ai_sdk/design.md) (see §9 Phase-0 reconciliation).
**Sibling adapters:** D04 (`@spendguard/langchain`, callback handler), D06 (`@spendguard/vercel-ai`, model middleware). D38 mirrors their package discipline; it does NOT mirror their fail-open degradation branch (see §7 — fail-closed is a LOCKED D38 deviation).

## 1. Problem

D06 shipped on the rationale "Mastra Agents call `generateText`/`streamText` from `ai`, so one wrap covers both ecosystems." That rationale is stale:

1. **Mastra owns its own agent loop since v0.14.0 (Aug 2025).** `@mastra/core` (1.41.0 as of 2026-06-04; 1.0.0 since 2026-01-20; ~966k npm downloads/week) no longer calls `generateText`/`streamText` from `ai`. It still *consumes* AI SDK `LanguageModel` instances (`doGenerate`/`doStream`), so explicit-instance users who wrap via `wrapLanguageModel` remain covered by D06.
2. **The flagship default-DX path is uncovered.** Mastra's model-router string syntax (`model: "openai/gpt-4o"`, 40+ providers) resolves models internally via `MastraModelGateway.resolveLanguageModel(): Promise<LanguageModelV2>`. There is **no injection point for `wrapLanguageModel`** on that path. A Mastra user following the front-page quickstart gets zero SpendGuard enforcement today.
3. **Mastra's first-party cost control is not a hard gate.** `CostGuardProcessor` (`@mastra/core/processors`) is, by its own documentation, best-effort and fail-open (see §2). Teams that need a hard ceiling have no in-ecosystem answer.

Mastra exposes the right attach point for us: the **`Processor` interface** (mastra.ai/reference/processors/processor-interface) with lifecycle hooks around every agent step and every provider call — including `processInputStep`, documented as running "at every step including tool call continuations, before sent to the LLM". A processor mounts on an `Agent` regardless of whether the model came from an explicit AI SDK instance or the model-router string. That is the model-source-independent boundary D38 gates.

D38 ships `@spendguard/mastra`: a `SpendGuardProcessor` that reserves budget pre-dispatch at the before-LLM-step boundary, commits on response, and settles failures — hard, fail-closed, against the durable SpendGuard ledger with the signed audit chain.

## 2. Positioning — factual contrast (LOCKED wording discipline)

This section is the canonical positioning text. README / docs page / CHANGELOG derive from it. Rule: factual contrast only, sourced from upstream's own documentation; no disparagement of Mastra or `CostGuardProcessor`.

| Dimension | Mastra `CostGuardProcessor` (per its own docs) | `@spendguard/mastra` `SpendGuardProcessor` |
|---|---|---|
| Enforcement point | After cost data is observed; cost persisted **asynchronously** | **Pre-dispatch**: budget reserved BEFORE the provider call leaves the process |
| Ceiling semantics | "treat `maxCost` as a best-effort threshold, not a hard ceiling" | Hard ceiling: reservation against a durable ledger; DENY halts the step |
| Failure posture | **Fail-open** on missing context / query failure | **Fail-closed**: sidecar unreachable or DENY ⇒ step aborts with a typed error |
| Backing store | Requires OLAP observability store (DuckDB/ClickHouse; Postgres unsupported for metrics) | SpendGuard sidecar + Postgres ledger + signed audit chain (already deployed for every other SpendGuard adapter) |
| Scope | run / resource / thread, block or warn | tenant / budget / window via SpendGuard contract DSL; shared budgets across Python, LangChain, proxy, and gateway adapters |
| Cross-runtime budget | Mastra-only | Same `budget_id` enforced across every SpendGuard integration |

The two are complementary: `CostGuardProcessor` remains a good soft-warn UX layer; `SpendGuardProcessor` is the hard enforcement layer. The docs page MUST say exactly that.

**Vs. D06 (`@spendguard/vercel-ai`)**: D06 gates a *model instance*; D38 gates an *agent step*. Post Phase-0 (§9), D06's coverage claim is scoped to "explicit AI SDK model instances"; D38 owns Mastra Agents — both model-router strings and explicit instances — at the processor boundary.

## 3. Goals

1. Publish `@spendguard/mastra` npm package, version `0.1.0`, Apache-2.0, in-tree at `sdk/typescript-mastra/` (new pnpm workspace member). Peer-deps: `@mastra/core` `>=1.0.0 <2`, `@spendguard/sdk` (workspace convention identical to D06's published shape). Node `>=22.13.0` (Mastra 1.x floor).
2. Core export: `SpendGuardProcessor` — a Mastra `Processor` implementation. Reserve at the before-LLM-step hook (`processInputStep`); commit at the LLM-response/output hook; FAILURE-commit / TTL-sweep settlement on failure. Mounts via the Agent processor list, so it covers model-router-string agents — the path D06 cannot reach.
3. **Fail-closed only.** Sidecar unreachable or DENY ⇒ the step aborts with a typed error. There is NO fail-open knob, NO env escape hatch (LOCKED — see §7).
4. `unitId?: string` on the options surface from day 1 (HARDEN_D05_UR invariant — threads to `claim[0].unit.unit_id`; empty `unit_id` is rejected by the sidecar for ledger-backed reserves).
5. All hashing / id derivation reuses `@spendguard/sdk` (`deriveIdempotencyKey`, `deriveUuidFromSignature`, `computePromptHash`). BLAKE2b cross-language byte-equivalence is P0 (D05 §13). Zero hash code in the adapter.
6. Demo mode `mastra_processor` (overlay `deploy/demo/mastra_processor/` — name LOCKED) with HARD verify SQL gates mirroring `verify_step_langchain_ts.sql`. D06's `vercel_ai_mastra` demo remains untouched and passing.
7. Phase-0 reconciliation: amend D06's stale Mastra rationale and resolve its `ai` peer-dep drift (§9) BEFORE the new package's claims publish.

## 4. Non-goals

- **Auxiliary LLM calls** — Mastra memory title generation, `ModerationProcessor`'s classifier call, scorers. OUT of v1 scope. Documented known limitation; workaround: wrap those models explicitly via D06 `wrapLanguageModel`. (Docs page MUST carry this limitation box.)
- **Per-chunk stream gating** — same posture as D04/D24: reserve brackets the whole step; commit after the stream completes (§8).
- **Mastra `Workflow` step gating** — workflows are a different execution surface; v2 candidate.
- **AI SDK v6 `LanguageModelV3` middleware variant** — explicitly OUT of D38; recorded as a D06 follow-on (§9.3).
- **Approval-resume UI** — `ApprovalRequired` propagates; pattern documented, no helper.
- **`CostGuardProcessor` interop/import** — we do not read or write its cost records.
- **Tool-call PRE gating (`TOOL_CALL_PRE`)** — `processInputStep` fires on tool-call continuation steps too, which gates the *LLM call after* a tool result; gating the tool execution itself is v0.2.

## 5. Public surface — LOCKED (verbatim contract)

Slices copy this block exactly (§1.2-style verbatim contract). Any drift is a P0 finding (review-standards §2).

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

Surface rules (all P0, review-standards §2):

- `src/index.ts` exports exactly the symbols above. No `default` export. No re-export of other `@spendguard/sdk` symbols (consumers import `ApprovalRequired`, `DecisionStopped`, `HandshakeError`, etc. from the substrate directly — D06 anti-list discipline; note `DecisionStopped` / `ApprovalRequired` are subclasses of `DecisionDenied`, so `instanceof DecisionDenied` catches all denial flavours).
- `SpendGuardProcessor implements Processor` from `@mastra/core/processors` and the package typechecks against the installed peer. **The `implements Processor` typecheck IS the hook-signature gate** — hook parameter/return types are pinned by the real package, not hand-copied into this doc. Exact installed signatures are recorded by slice `COV_D38_02` (`[VERIFY-AT-IMPL: V1]`, §12).
- Constructor validation: missing `client` or empty `tenantId` ⇒ `TypeError` at construction (matches D06 `validateOpts`).
- Options type contains NO fail-open field (no `failOpen`, no `degradeOnUnavailable`, no `enforcementMode`). Adding one is a P0 finding.

## 6. Architecture

```
new Agent({ model: "openai/gpt-4o-mini" | explicitAiSdkInstance,
            processors-mount: [spendGuardProcessor] })   // exact key: [VERIFY-AT-IMPL: V5]
        │
        ▼  per step (incl. tool-call continuations)
SpendGuardProcessor
   ├── processInputStep  ─► flattenStepText → derive (llmCallId, runId,
   │                        decisionId, idempotencyKey) via @spendguard/sdk
   │                        → client.reserve(LLM_CALL_PRE)
   │                          ├─ CONTINUE/DEGRADE → push InflightEntry; step proceeds
   │                          └─ DENY / STOP / APPROVAL / UNREACHABLE → THROW
   │                            (fail-closed: provider call never fires)
   ├── processLLMRequest ─► no-op in v1 (reserve already brackets the step;
   │                        kept as the pinned fallback reserve point if a
   │                        model path skips processInputStep — [VERIFY-AT-IMPL: V1])
   ├── processLLMResponse ─► pop InflightEntry →
   │                        client.commitEstimated(outcome=SUCCESS,
   │                          outcomeKind=SUCCESS, actuals from usage when
   │                          exposed — [VERIFY-AT-IMPL: V4])
   └── processOutputStep ─► backstop commit (at most one commit per
                            reservation; no-op when LLMResponse already
                            committed) + failure settlement when the hook
                            surface exposes an error signal ([VERIFY-AT-IMPL: V7])

failure with no hook fired (process crash, hard abort)
        └─► sidecar TTL sweep settles the open reservation (ledger backstop)
```

### 6.1 Lifecycle mapping table (LOCKED)

| Mastra hook | When it runs (per Mastra docs) | SpendGuard action |
|---|---|---|
| `processInputStep` | every step including tool-call continuations, before messages are sent to the LLM | RESERVE — `client.reserve(trigger="LLM_CALL_PRE")`; throw on any failure (fail-closed) |
| `processLLMRequest` | immediately before each provider call | v1: no-op (assert-only in tests: reserve must already be inflight) |
| `processLLMResponse` | after each provider response | COMMIT — `client.commitEstimated(outcome="SUCCESS", outcomeKind="SUCCESS")` with usage actuals when exposed |
| `processOutputStep` / output hooks | after the step's output is assembled | backstop COMMIT if the response hook did not fire for this reservation (streaming ordering — `[VERIFY-AT-IMPL: V4]`); FAILURE settlement if an error/abort signal is exposed (`[VERIFY-AT-IMPL: V7]`) |
| step failure (provider error) | error surfaced through whichever hook/callback Mastra exposes | FAILURE-COMMIT — `commitEstimated(outcome="PROVIDER_ERROR", outcomeKind="FAILURE", actualErrorMessage=err.message)`; if no error hook exists, the sidecar TTL sweep is the LOCKED settlement backstop |

### 6.2 Reserve request shape (LOCKED)

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

### 6.3 Identity derivation (LOCKED — substrate helpers only)

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

Properties:

- A retry of the SAME step (same accumulated messages) re-derives the same `llmCallId` → sidecar idempotency cache + ledger `UNIQUE` collapse onto the first decision.
- Each loop step appends messages, so `stepText` differs per step → distinct `llmCallId`, `decisionId`, `idempotencyKey` per step. The multi-step agent loop is gated per step, not once per run.
- Two byte-identical steps in the same run share a key — the same accepted trade-off D06 documents (callers wanting fresh ids salt prompts or supply `runIdProvider`).
- ALL derivation goes through `@spendguard/sdk`. The adapter contains zero `node:crypto` / `@noble/hashes` imports (P0, review-standards §4).

### 6.4 Default claim projection (LOCKED)

Mirrors D04/D06 exactly: `estimatedTokens = max(1, ceil(stepText.length / 4))`, `amountMicros = defaultBudgetMicrosCap > 0n ? defaultBudgetMicrosCap : estimatedTokens * 1_000n`, `scopeId = budgetId ?? tenantId`, `unit = { unit: "USD_MICROS", denomination: 1, ...(unitId ? { unitId } : {}) }`. The PRE number is a coarse probe; authoritative spend lands on the commit. `claimEstimator` overrides the whole projection.

### 6.5 Inflight correlation (LOCKED fallback + pinned primary)

- **Primary (preferred)**: if the Mastra hook args expose a stable per-call/per-step correlation id visible at BOTH `processInputStep` and `processLLMResponse`/`processOutputStep`, key the inflight map by it. `[VERIFY-AT-IMPL: V3]` — slice `COV_D38_02` pins whether such an id exists in `@mastra/core` ≥1.0.
- **LOCKED fallback** (used when V3 finds no shared id): per-`runId` FIFO queue. Mastra's agent loop is sequential within a run (step N+1 starts after step N settles), so the response hook pops the oldest open entry for its run. Parallel agents/runs have distinct `runId`s and never cross-talk.
- Global capacity bound 10_000 entries, FIFO eviction (D04 parity) — a hook that never fires cannot leak memory unbounded.
- Entry carries `{ decisionId, reservationId, runId, llmCallId, idempotencyKey, projectedAmountAtomic }` — `projectedAmountAtomic` feeds the §6.6 usage fallback.

### 6.6 Commit estimation (LOCKED fallback per constraint)

- When the response/output hook exposes provider usage (Mastra 1.x normalizes nested AI SDK v6 usage to flat fields — `[VERIFY-AT-IMPL: V4]` pins the exact field names), commit with `estimatedAmountAtomic: "0"`, `actualInputTokensWire` / `actualOutputTokensWire` from usage — identical wire shape to the shipped D04 handler.
- **When usage is NOT available at the hook** (LOCKED fallback): commit with `estimatedAmountAtomic = projectedAmountAtomic` carried in the inflight entry (the §6.4 default-estimator projection) and actuals omitted. The reservation settles at the estimate; the audit chain records that no provider actuals were observed.

### 6.7 Dated amendments (append-only)

**2026-06-10 — orchestrator-ratified (COV_D38_03 R1; HARDEN_D05_WI):**

1. **§6.5 entry shape — ADDITIVE `unit` field.** The inflight entry carries the reserve-time unit: `{ decisionId, reservationId, runId, llmCallId, idempotencyKey, projectedAmountAtomic, unit }`, where `unit` is the projected claims' `claim[0].unit`. Rationale: commits must tuple-match the reservation (repo-wide HARDEN_D05_WI invariant; D04 precedent `pending.unit = projectedClaim.unit`, `sdk/typescript-langchain/src/handler.ts:315/373`). A custom `claimEstimator` may reserve under a different unit/unitId than the §6.4 default projection; `settleCommit` therefore reuses `entry.unit` rather than re-deriving the default-options unit. §6.5 above is NOT rewritten — this subsection amends it (COV_D38_03 R1 Major 1).
2. **§6.6 erratum — the `estimatedAmountAtomic: "0"` literal is WRONG** (LOCKED-DISPUTE ratified by R1 + orchestrator). §6.6 simultaneously locks "identical wire shape to the shipped D04 handler", and the shipped D04/HARDEN_D05_WI wire shape on SUCCESS-with-usage is `estimatedAmountAtomic` = input+output token SUM (the ledger rejects `estimated_amount_atomic = 0` bookings); when usage is absent, the reserve-time projection fallback in the second bullet applies unchanged. The D04/HARDEN_D05_WI convention controls; read §6.6's first bullet with estimate = usage sum, not `"0"`. `tests.md` TP-24 is corrected to match.

**2026-06-11 — orchestrator-ratified (COV_D38_05 pre-R1 fix rider; HARDEN_D05_WI):**

3. **§5 options surface + §6.5 entry shape — ADDITIVE `pricing?: PricingFreeze`.** `SpendGuardProcessorOptions` gains an optional `pricing` field (doc-comment mirrors `sdk/typescript-langchain/src/options.ts`; env convention `SPENDGUARD_PRICING_VERSION` + `SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX` + `SPENDGUARD_FX_RATE_VERSION` + `SPENDGUARD_UNIT_CONVERSION_VERSION`), the inflight entry stashes it at reserve time (mirror of amendment #1's `unit` field), and `settleCommit` sends `pricing: entry.pricing ?? EMPTY_PRICING`. Rationale: §6.6's "commits repeat the same empty tuple" assumption is empirically WRONG against the production sidecar — the COV_D38_05 live demo proved the reservation is stamped with the LOADED BUNDLE's pricing freeze and the empty-tuple commit is REJECTED (`pricing freeze mismatch: payload pricing tuple differs from original reservation`; no booking lands). Precedent: the shipped D04 handler's `pricing?: PricingFreeze` option (`sdk/typescript-langchain/src/handler.ts:316/377`, `pending.pricing = opts.pricing` → `pending.pricing ?? EMPTY_PRICING` on the commit); invariant: HARDEN_D05_WI commit-must-tuple-match-reservation. This extends the LOCKED §5 surface — append-only; the §5 block above is NOT rewritten; the verbatim-surface tests (TP-04 family) include the new key. Absent option keeps the pre-amendment wire shape (empty tuple — matches no-bundle reservations only).

**2026-06-11 — recorded by COV_D38_06 (deferred from COV_D38_02 R1 minor 3):**

4. **§5 class shell — ADDITIVE `readonly id` divergence note.** The §5 LOCKED class shell shows only `readonly name = "spendguard-processor"`, but the installed `@mastra/core` 1.41.0 `Processor<TId, TTripwireMetadata>` interface REQUIRES `readonly id: TId` (`name` is optional), and the Agent config requires `id` alongside `name`. Per §5's own "typechecks against the installed peer" rule (the `implements Processor` typecheck IS the hook-signature gate), the shipped `SpendGuardProcessor` carries BOTH `readonly id = "spendguard-processor"` and `readonly name = "spendguard-processor"` (same literal; `src/processor.ts`, V1 pin in COV_D38_02). This is not a surface drift: the §5 shell is amended additively, the §5 block above is NOT rewritten, and the public barrel is unchanged.

**2026-06-13 — residual closure (gh #181; D38 V2 cause-chain):**

5. **§7 rule 3 / V2 consumer-reachability erratum.** The shipped V2 pin against `@mastra/core` 1.41.0 proves the enforcement invariant but narrows the catch contract: a plain throw from `processInputStep` halts the step before provider dispatch, and the typed substrate error remains an `instanceof DecisionDenied` / `SidecarUnavailable` / `SpendGuardError` at the hook boundary. At the public `Agent.generate()` / `Agent.stream()` boundary, Mastra serializes processor workflow errors; the rejection preserves the error message but not the class instance or a typed `cause` chain. Therefore the authoritative v1 consumer contract is: **DENY => zero provider calls; hook boundary => typed `instanceof`; Agent boundary => message-match.** The older §7 rule-3 sentence "consumer can reach the typed error via the thrown error or its `cause` chain" is superseded for `@mastra/core` 1.41.0 by this amendment. Revisit only if a future Mastra release preserves processor error causes across the Agent boundary.

| Condition | Error surfaced | Where | Step outcome |
|---|---|---|---|
| Invalid options (`client` missing, `tenantId` empty) | `TypeError` | constructor | construction fails |
| Reserve → DENY / STOP / STOP_RUN_PROJECTION | `DecisionDenied` / `DecisionStopped` | `processInputStep` | **step aborts; provider never called** |
| Reserve → REQUIRE_APPROVAL | `ApprovalRequired` (subclass of `DecisionDenied`) | `processInputStep` | step aborts; resume pattern documented, no helper in v1 |
| Sidecar unreachable / timeout / handshake missing | `SidecarUnavailable` / `HandshakeError` | `processInputStep` | **step aborts (FAIL-CLOSED)** |
| Any other substrate error on reserve | `SpendGuardError` | `processInputStep` | **step aborts (FAIL-CLOSED)** |
| Provider error mid-step | original provider error rethrown by Mastra; adapter emits FAILURE commit | response/output/error hook (`[VERIFY-AT-IMPL: V7]`) | provider error propagates; reservation settles (or TTL sweep) |
| Commit RPC failure AFTER a successful provider call | logged at error level; **not** thrown into the consumer's result | commit path | step result delivered; reservation settles via sidecar TTL sweep + audit chain |
| Commit hook with no matching inflight entry | warn + no-op | commit path | idempotent re-delivery safe |

LOCKED rules:

1. **No fail-open branch anywhere.** Unlike the shipped D04/D06 adapters (which log-and-proceed on `SidecarUnavailable` per their "operational degradation" stance), `SpendGuardProcessor` propagates EVERY reserve-path error. This is a deliberate, positioning-bearing deviation (§2): in the Mastra ecosystem the fail-open niche is already occupied by `CostGuardProcessor`; D38's reason to exist is the hard gate. Any `catch`-and-continue around `client.reserve()` is a P0 finding.
2. **No env escape hatch.** The adapter reads NO environment variable that weakens enforcement. (`SPENDGUARD_DISABLE` exists on the substrate client for tests — the adapter neither reads nor documents it as a production path.)
3. **Abort mechanism**: the adapter throws the substrate typed error from `processInputStep`. `[VERIFY-AT-IMPL: V2]`: slice `COV_D38_02` MUST verify against the installed `@mastra/core` that a throw from `processInputStep` halts the step before the provider call — and if Mastra's processor runner instead requires its `abort()` mechanism to halt (TripWire-style), the adapter calls that mechanism **with the typed error preserved on the `cause` chain**. Either way the observable contract is fixed and test-pinned: **DENY ⇒ zero provider HTTP calls** (tests.md TP-10/TA-04) and the consumer can reach the typed error via the thrown error or its `cause` chain.
4. **The pre/post asymmetry is intentional**: fail-closed gates *dispatch* (no unguarded provider call), not *result delivery* (a post-call commit failure cannot un-spend; destroying the user's already-paid-for response would add harm without enforcement value). Reviewers must not flag the commit-path swallow as fail-open — it is the same race-guard semantics D06 §6 locked, backed by the TTL sweep.

## 8. Streaming posture (LOCKED — D04/D24 parity)

Mastra's agent loop streams. D38 brackets the WHOLE step at the before-LLM boundary:

- Reserve fires once at `processInputStep` (before the first chunk leaves the provider) and covers the entire step.
- Commit fires once after the step's stream completes, with usage from the response metadata when exposed (§6.6 fallback otherwise).
- Per-chunk gating is explicitly out of scope (§4). Mid-stream abort → FAILURE settlement path (§6.1 last row).

## 9. Phase-0 reconciliation (LOCKED decisions)

Slice `COV_D38_00` lands these BEFORE the new package publishes any claim. History is not rewritten — amendments are dated and appended.

### 9.1 D06 design.md amendment (dated, appended)

Append to `docs/specs/coverage/D06_vercel_ai_sdk/design.md` a new final section:

> `## 9. Amendment 2026-06-10 (D38 Phase-0)` — (a) The §1/§3-era rationale "Mastra Agents call `generateText`/`streamText` from `ai` underneath" is stale: Mastra owns its own agent loop since v0.14.0 (Aug 2025). (b) D06's Mastra coverage is re-scoped to **explicit AI SDK `LanguageModel` instances** handed to Mastra (Mastra still consumes `doGenerate`/`doStream` model objects); the model-router string syntax has no `wrapLanguageModel` injection point and is covered by **D38** (`@spendguard/mastra`). (c) The `@spendguard/vercel-ai/mastra` subpath alias remains published and functional for explicit-instance users; its docs gain a pointer to `@spendguard/mastra` as the recommended Mastra integration. (d) Locked decision #5 ("AI SDK v5+ only") is corrected to match shipped reality — see §9.2.

The original sections are left byte-intact above the amendment (no history rewrite). The title's "(covers Mastra)" stays for historical traceability; the amendment paragraph is the authoritative scope statement.

### 9.2 `ai` peer-dep drift resolution

**Observed drift (two-sided):** D06 design.md locked decision #5 says "AI SDK v5+ only. No v4 back-compat shim." The shipped adapter does the opposite: `sdk/typescript-vercel-ai/package.json` declares `"ai": ">=4.0.0"` (unbounded), and `src/middleware.ts` implements `LanguageModelV1Middleware` with `middlewareVersion: "v1"` — the **AI SDK v4** middleware shape (the file's own header documents the v5→v4 retarget).

**Decision (deviating from the default recommendation, with justification):** tighten the peer-dep to **`"ai": ">=4.0.0 <5"`**, released as `@spendguard/vercel-ai` **0.2.0** with a CHANGELOG entry. We explicitly do NOT adopt the `>=5.0.0 <7` tightening, because:

1. AI SDK v5+ `wrapLanguageModel` consumes `LanguageModelV2Middleware`; the shipped v1-shaped middleware does not satisfy it. Declaring `>=5.0.0 <7` would advertise compatibility the artifact does not have — a worse lie than the current unbounded range, and unlike the current range it can never accidentally work.
2. The truthful fix that *also* covers v5/v6 is the V2/V3 middleware migration — real engineering work, recorded as the **D06 follow-on deliverable** (out of D38 scope per §9.3), not a Phase-0 metadata edit.
3. `>=4.0.0 <5` makes the package manager fail fast for `ai@5/6` consumers instead of letting them install a silently incompatible middleware — strictly better than today on the exact failure mode the drift creates.

D06 design.md locked decision #5 is corrected by the §9.1 amendment to: "shipped 0.x targets the AI SDK v4 line (`LanguageModelV1Middleware`); v5 (`LanguageModelV2Middleware`) and v6 (`LanguageModelV3`) variants are the D06 follow-on."

Consequence recorded honestly in the amendment: because Mastra 1.0 consumes `LanguageModelV2`/`V3` instances, D06's *explicit-instance Mastra* coverage is bounded by the v4 model shape until the follow-on ships — one more reason D38 is the primary Mastra answer.

### 9.3 AI SDK v6 `LanguageModelV3` middleware variant

Explicitly OUT of D38 scope. Recorded here and in the §9.1 amendment as the D06 follow-on (V2 + V3 middleware variants, new peer ranges, new conformance tests). D38 does not block on it: the Processor boundary is model-version-independent.

### 9.4 D06 demo non-regression (HARD gate)

`deploy/demo/vercel_ai_mastra/` and `deploy/demo/verify_step_vercel_ai_mastra.sql` are NOT touched by any D38 slice. Acceptance gate A6.4 re-runs `make demo-up DEMO_MODE=vercel_ai_mastra` + `make -C deploy/demo demo-verify-vercel-ai-mastra` green after Phase-0 and again after the final slice.

## 10. Demo overlay (name LOCKED)

- Overlay: `deploy/demo/mastra_processor/docker-compose.yaml` — `counting-stub` (verbatim copy per existing per-overlay isolation convention) + `mastra-processor-runner` (**`node:22.13-bookworm-slim`** — Mastra needs Node ≥22.13; this is the first demo runner off the node:20.10 base, called out so the image gate doesn't "fix" it back).
- Runner script: `examples/mastra-processor/index.mjs`, 3 steps mirroring `langchain_ts` / `vercel_ai_mastra`:
  - step 1 **ALLOW** — `agent.generate(...)` small prompt → counter +1, SUCCESS commit.
  - step 2 **DENY** — second `SpendGuardProcessor` whose `claimEstimator` projects a claim past the demo contract's 1B-atomic hard cap → sidecar DENY pre-call → step aborts → counter UNCHANGED.
  - step 3 **STREAM** — `agent.stream(...)` → one reserve at step open, one commit after stream end.
  - Success line (LOCKED spelling, D11/6 §6.7 pattern): `[demo] mastra_processor ALL 3 steps PASS (ALLOW + DENY + STREAM)`.
- Model source: PRIMARY — model-router string `"openai/gpt-4o-mini"` pointed at the counting-stub via base-URL override (`[VERIFY-AT-IMPL: V6]`: whether `MastraModelGateway` honors `OPENAI_BASE_URL`/per-provider `baseURL` config). LOCKED FALLBACK if V6 fails: explicit AI SDK provider instance with `baseURL` at the counting-stub — the Processor attach point is identical for both model sources, and a vitest integration test (TP-22) separately proves the processor mounts on a router-string agent.
- Verify: `deploy/demo/verify_step_mastra_processor.sql` — gate structure copied from `verify_step_langchain_ts.sql` (`COV_D38_GATE` prefix): reserve ≥ 2, commit_estimated ≥ 2, denied_decision ≥ 1, INV-2 strict-order (earliest reserve < earliest `spendguard.audit.outcome`), canonical decision rows ≥ 2; plus the cross-DB canonical_events check and outbox-closure check in the Makefile target `demo-verify-mastra-processor` (mirrors `demo-verify-langchain-ts`).
- Demo env: same tenant/budget/window/unit constants as the sibling overlays (`SPENDGUARD_UNIT_ID=66666666-6666-4666-8666-666666666666` proves day-1 unitId threading end-to-end).

## 11. Locked design decisions

1. **Dedicated package `@spendguard/mastra`** at `sdk/typescript-mastra/` — NOT a D06 subpath. The Processor boundary, peer set (`@mastra/core`), Node floor (22.13), and release cadence (Mastra ships weekly minors) all differ from D06.
2. **`SpendGuardProcessor` class implementing Mastra `Processor`** is the canonical surface. No factory-function alternative, no model wrapper.
3. **Reserve at `processInputStep`** — the before-LLM-step boundary, firing on tool-call continuation steps too. `processLLMRequest` is a no-op in v1.
4. **Fail-closed ONLY.** Every reserve-path error aborts the step. No fail-open knob, no env escape hatch. Deliberate deviation from shipped D04/D06 degradation branches; positioning-bearing (§2, §7).
5. **`unitId` on the options surface day 1** (HARDEN_D05_UR invariant), threaded to `claim[0].unit.unit_id`.
6. **All id/hash derivation via `@spendguard/sdk`** — `deriveIdempotencyKey`, `deriveUuidFromSignature`, (`computePromptHash` if prompt hashing is surfaced later). Zero local hash code (P0).
7. **Identity tuple parity with D04/D06**: `stepId = "llm_call"` constant; content-derived `llmCallId`/`decisionId`; `sessionId = runId` in the key tuple (§6.3).
8. **Streaming = whole-step bracket** (§8). Per-chunk gating out of scope.
9. **Failure settlement = FAILURE commit where a hook exists; sidecar TTL sweep is the guaranteed backstop.** Explicit `client.release()` is reserved for a cancel-before-dispatch path if Mastra exposes one (`[VERIFY-AT-IMPL: V7]`); absence does not block v1.
10. **Commit-estimation fallback**: provider usage when exposed; else the reserve-time projected amount as `estimatedAmountAtomic` (§6.6).
11. **Phase-0 resolutions as specified in §9** — dated D06 amendment; `ai` peer tightened to `>=4.0.0 <5` (justified deviation from the `>=5` default recommendation); v6 V3 middleware recorded as D06 follow-on.
12. **Demo mode name `mastra_processor`**, overlay `deploy/demo/mastra_processor/`, verify file `verify_step_mastra_processor.sql`, gate prefix `COV_D38_GATE`.
13. **Aux LLM calls out of v1** with the documented D06 explicit-wrap workaround (§4).
14. **Node engine `>=22.13.0`**, peer `@mastra/core >=1.0.0 <2`, ESM-only, tsup/vitest/biome — D04/D06 package discipline otherwise.

## 12. [VERIFY-AT-IMPL] register

Every marker below is pinned by the named slice against the INSTALLED `@mastra/core` (devDep `^1.41.0`); the slice doc records the verified answer + package version. Markers never weaken a LOCKED decision — they select between pre-declared alternatives.

| ID | Question | Pinned by | Pre-declared alternatives |
|---|---|---|---|
| V1 | Exact `Processor` hook signatures (args object shape, async contract) for `processInputStep` / `processLLMRequest` / `processLLMResponse` / `processOutputStep` | COV_D38_02 | n/a — `implements Processor` typecheck is the gate; doc records shapes |
| V2 | Does a throw from `processInputStep` halt the step pre-provider, or is the hook-provided `abort()` (TripWire) required? | COV_D38_02 | throw directly / call abort() with typed error on `cause` — observable contract fixed either way (§7.3) |
| V3 | Is a stable per-call/per-step correlation id visible at both reserve and commit hooks? | COV_D38_02 | key inflight by that id / LOCKED per-runId FIFO fallback (§6.5) |
| V4 | Which usage fields (flat normalized) does `processLLMResponse` / `processOutputStep` expose, and which hook fires last on streamed steps? | COV_D38_03 | usage actuals / LOCKED estimated-amount fallback (§6.6); backstop-commit ordering (§6.1) |
| V5 | Exact Agent constructor key for mounting processors in `@mastra/core` 1.x (`inputProcessors`/`outputProcessors`/unified list) | COV_D38_02 | record + use the installed key; quickstart copies it |
| V6 | Does the model-router string path honor a base-URL override (env or per-provider config) for `"openai/..."`? | COV_D38_05 | router-string demo / LOCKED explicit-instance fallback + TP-22 router-mount test (§10) |
| V7 | Does the Processor surface expose an error/abort signal usable for the FAILURE commit? | COV_D38_03 | FAILURE commit at the signal / TTL-sweep-only settlement (§6.1) |
| V8 | Does `withMastra()` (plain-AI-SDK mounting) run the same Processor hooks? | COV_D38_06 | document as supported usage variant B / document as unsupported in v1 |

## 13. Slice plan

| # | Slice | Scope | Size |
|---|---|---|---|
| 0 | `COV_D38_00_phase0_reconciliation` | D06 design.md dated amendment (§9.1); `@spendguard/vercel-ai` peer tightened to `>=4.0.0 <5` + CHANGELOG + 0.2.0 version bump; D06 test suite + `vercel_ai_mastra` demo re-run green | S |
| 1 | `COV_D38_01_pkg_init` | `sdk/typescript-mastra/` skeleton (package.json, tsconfig, tsup, biome, vitest), pnpm-workspace.yaml member, sanity import test | S |
| 2 | `COV_D38_02_processor_reserve` | `SpendGuardProcessor` + LOCKED options surface (incl. `unitId` day 1) + identity derivation + inflight map + `processInputStep` reserve wiring; pins V1/V2/V3/V5; DENY-before-inner-call proven | M |
| 3 | `COV_D38_03_commit_failure_paths` | `processLLMResponse`/`processOutputStep` commit + usage extraction + §6.6 fallback + FAILURE settlement; pins V4/V7; streaming whole-step tests | M |
| 4 | `COV_D38_04_failclosed_estimator_tests` | full fail-closed matrix, claimEstimator/route/budgetId/unitId threading tests, hash-reuse lint+test gates, mock-sidecar suite to coverage floor | M |
| 5 | `COV_D38_05_demo_mastra_processor` | `examples/mastra-processor/` + `deploy/demo/mastra_processor/` overlay + `verify_step_mastra_processor.sql` + Makefile branches/targets; pins V6 | M |
| 6 | `COV_D38_06_docs_publish` | README, CHANGELOG, LICENSE_NOTICES, docs site page (positioning §2 + aux-LLM limitation box), repo-root adapter table row, publish workflow, size gate; pins V8 | S |

7 slices, 3 S + 4 M. One more than the 5-6 sketch because Phase-0 touches a *different package* (`sdk/typescript-vercel-ai`) plus D06 docs and must merge — with its own non-regression gates — before any `@spendguard/mastra` claim ships; folding it into pkg_init would put two packages and two spec trees in one reviewable diff, violating the slice-small directive.
