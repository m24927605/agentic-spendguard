# D38 — Tests

Numbered as **TP-XX** (unit/integration, vitest, no live network) and **TA-XX** (acceptance — runnable gates, cross-referenced from `acceptance.md`). Mock sidecar comes from `sdk/typescript/tests/_support` (re-exported via `tests/_support/mockSidecar.ts`); Mastra integration tests run against the REAL installed `@mastra/core` devDep with a stub model (`tests/_support/stubModel.ts`) — no provider HTTP.

## 1. Coverage targets

| Module | Floor |
|---|---|
| `processor.ts` | ≥ 90 % stmt, ≥ 85 % branch |
| `identity.ts`, `inflight.ts`, `flatten.ts`, `usage.ts` | 100 % stmt, ≥ 90 % branch |
| Package overall | ≥ 90 % stmt, ≥ 85 % branch |

## 2. Unit / integration tests (TP)

### Locked surface (`tests/lockedSurface.test.ts`)

| # | Test | Verifies |
|---|---|---|
| TP-01 | Barrel exports are exactly `SpendGuardProcessor`, `DecisionDenied`, `SidecarUnavailable`, `SpendGuardError`, `VERSION` (+ type-only exports) | design §5 verbatim contract; no default export |
| TP-02 | `SpendGuardProcessor` satisfies the installed `@mastra/core/processors` `Processor` type (`implements` compiles; instance assignable to `Processor`) | hook-signature gate (V1) |
| TP-03 | `new SpendGuardProcessor({} as any)` / missing `client` / empty `tenantId` → `TypeError` | constructor validation |
| TP-04 | `SpendGuardProcessorOptions` type has NO `failOpen` / `degradeOnUnavailable` / `enforcementMode` key (type-level + runtime `Object.keys` probe of a fully-populated options literal) | fail-closed P0 — no knob exists to find |
| TP-05 | Re-exported error classes are reference-identical to `@spendguard/sdk`'s (`===`) | `instanceof` across boundary |
| TP-06 | `readonly name === "spendguard-processor"` | Mastra processor identity |

### Identity derivation (`tests/identity.test.ts`)

| # | Test | Verifies |
|---|---|---|
| TP-07 | `deriveStepIdentity` output equals a direct `@spendguard/sdk` `deriveIdempotencyKey({tenantId, sessionId: runId, runId, stepId: "llm_call", llmCallId, trigger: "LLM_CALL_PRE"})` call for 8 fixture tuples | delegation proof — adapter adds no transformation |
| TP-08 | Same `(tenantId, stepText)` twice → identical `llmCallId` / `decisionId` / `idempotencyKey`; differing `stepText` → all three differ | retry collapse + per-step distinctness (design §6.3) |
| TP-09 | Golden vector: one fixture input recomputed against the substrate's published FX vector format (`sg-` + 32 hex; byte-equal to Python `derive_idempotency_key` for the shared fixture tuple) | BLAKE2b cross-language byte-equivalence rides the substrate's P0 gate (D05 §13) |

### Reserve path / fail-closed (`tests/processor.test.ts`, `tests/failClosed.test.ts`)

| # | Test | Verifies |
|---|---|---|
| TP-10 | **DENY-before-inner-call**: mock sidecar returns DENY → `agent.generate()` (real `@mastra/core` Agent + stub model) rejects AND the stub model records **zero** `doGenerate`/`doStream` invocations | THE core enforcement proof (design §7.3); also pins V2 |
| TP-11 | Reserve request wire shape: `trigger="LLM_CALL_PRE"`, `stepId="llm_call"`, `route` default `"mastra-llm"`, `decisionId === llmCallId`, `idempotencyKey` matches TP-07 derivation | design §6.2 |
| TP-12 | `processInputStep` fires once per step including a tool-call continuation step (Agent run with one tool call → 2 reserves) | per-step gating (design §6.1 row 1) |
| TP-13 | `SidecarUnavailable` from `reserve()` → step aborts; stub model: 0 calls; error (or its `cause` chain) is `instanceof SidecarUnavailable` | **fail-closed on outage** — no log-and-proceed branch |
| TP-14 | `DecisionStopped` and `ApprovalRequired` propagate identically (both `instanceof DecisionDenied`) | denial taxonomy |
| TP-15 | `HandshakeError` (client not handshaken) propagates; 0 model calls | fail-closed completeness |
| TP-16 | Source-level: `grep -E "catch" src/processor.ts` reserve section contains no catch that continues the step (asserted structurally: a thrown sentinel from a stubbed `client.reserve` always rejects the step promise) | no hidden degradation branch |
| TP-17 | `claimEstimator` provided → called exactly once per reserve with `{stepText, runId, llmCallId}`; its claims forwarded verbatim on `projectedClaims` | estimator contract |
| TP-18 | Default projection: chars/4 heuristic, `defaultBudgetMicrosCap` override, `scopeId = budgetId ?? tenantId` | design §6.4 |
| TP-19 | **unitId threading**: options `unitId` set → reserve's `projectedClaims[0].unit.unitId` equals it; unset → `unitId` absent from the wire `UnitRef` | HARDEN_D05_UR day-1 P0 |
| TP-20 | `runIdProvider` wins over content-derived runId; absent → `runId === llmCallId` (or V3 context id when pinned) | design §6.3 precedence |
| TP-21 | The processor never mutates the step messages (deep-equal before/after `processInputStep`) | passthrough contract |
| TP-22 | Processor mounts on an Agent built with a **model-router string** (gateway stubbed/intercepted at the resolve boundary) and `processInputStep` still fires | the D38 raison d'être — router path gated (pins V5; demo V6 fallback independence) |

### Commit / failure paths (`tests/processor.test.ts`, `tests/usage.test.ts`)

| # | Test | Verifies |
|---|---|---|
| TP-23 | Happy path: reserve → stub model responds → exactly ONE `commitEstimated` with `outcome="SUCCESS"`, `outcomeKind="SUCCESS"`, `decisionId`/`reservationId` from the reserve outcome | lifecycle correlation |
| TP-24 | Usage exposed (V4 shape, camelCase) → `actualInputTokensWire`/`actualOutputTokensWire` carry it; `estimatedAmountAtomic="0"` | actuals path |
| TP-25 | Usage exposed snake_case → same as TP-24 | cross-shape parity (D04/D06 discipline) |
| TP-26 | Usage ABSENT → commit carries `estimatedAmountAtomic === projectedAmountAtomic` from the reserve-time inflight entry; no actuals fields | §6.6 LOCKED fallback |
| TP-27 | Provider error mid-step → FAILURE commit (`outcome="PROVIDER_ERROR"`, `outcomeKind="FAILURE"`, `actualErrorMessage`) when the V7 signal exists; if V7 pinned "no error hook", test asserts NO success commit fires and the inflight entry remains for TTL settlement | failure settlement (design §6.1) |
| TP-28 | Commit RPC failure after success response → consumer still receives the step result; error logged; no throw | §7.4 pre/post asymmetry |
| TP-29 | Commit hook with no inflight entry → warn + no-op (no throw, no RPC) | idempotent re-delivery |
| TP-30 | Streaming step (`agent.stream`) → exactly one reserve at step open + one commit after stream completion; no per-chunk RPCs | design §8 whole-step bracket |
| TP-31 | At-most-one-commit: when both `processLLMResponse` and `processOutputStep` fire for the same step, exactly one commit RPC is issued | §6.1 backstop guard |

### Inflight (`tests/inflight.test.ts`)

| # | Test | Verifies |
|---|---|---|
| TP-32 | push/pop round-trip; second pop → `undefined` | one-shot |
| TP-33 | FIFO-within-key: two pushes same key → pops return in push order | §6.5 fallback ordering |
| TP-34 | Capacity 10_000 → oldest evicted on overflow | bounded memory |
| TP-35 | Two concurrent runs (distinct runIds) never cross-correlate | isolation |

### Hash-reuse P0 (`tests/hashReuse.test.ts`)

| # | Test | Verifies |
|---|---|---|
| TP-36 | `grep -RE "@noble/hashes|node:crypto|createHash|createHmac|blake2" sdk/typescript-mastra/src/` → zero matches (test shells out / reads files) | substrate-hash-reuse-only P0 |
| TP-37 | `package.json` has no `@noble/hashes` in any dependency block | no parallel hash dep |
| TP-38 | Built `dist/index.js` contains none of the TP-36 tokens and no inlined BLAKE2 table constants; imports resolve to externalized `@spendguard/sdk` | bundle-level enforcement |

## 3. Acceptance tests (TA)

| # | Gate | Pass condition |
|---|---|---|
| TA-01 | `pnpm -C sdk/typescript-mastra run test` | vitest exit 0; coverage floors of §1 met |
| TA-02 | `pnpm -C sdk/typescript-mastra run typecheck` | exit 0 — includes the `implements Processor` conformance against installed `@mastra/core` |
| TA-03 | `make demo-up DEMO_MODE=mastra_processor` | exit 0; LOCKED success line `[demo] mastra_processor ALL 3 steps PASS (ALLOW + DENY + STREAM)` printed |
| TA-04 | DENY step proof (inside TA-03 runner): counting-stub `/_count` UNCHANGED across step 2 | zero provider HTTP on DENY — live fail-closed proof |
| TA-05 | `make -C deploy/demo demo-verify-mastra-processor` | `verify_step_mastra_processor.sql` HARD gates pass: reserve ≥ 2, commit_estimated ≥ 2, denied_decision ≥ 1, INV-2 strict order, decision rows ≥ 2; canonical + outbox-closure blocks pass |
| TA-06 | **D06 demo non-regression**: `make demo-up DEMO_MODE=vercel_ai_mastra && make -C deploy/demo demo-verify-vercel-ai-mastra` | green, run at slice 0 AND after the final slice; `git diff --stat` shows zero changes under `deploy/demo/vercel_ai_mastra/` and `verify_step_vercel_ai_mastra.sql` |
| TA-07 | Phase-0: `git log -1 --format=%H -- docs/specs/coverage/D06_vercel_ai_sdk/design.md` shows the amendment commit; the file's pre-amendment content is byte-identical (append-only) | no history rewrite |
| TA-08 | Phase-0: `node -p 'require("./sdk/typescript-vercel-ai/package.json").peerDependencies.ai'` → `>=4.0.0 <5`; version `0.2.0`; `pnpm -C sdk/typescript-vercel-ai run test` green | peer-dep drift resolved per design §9.2 |
| TA-09 | unitId E2E: with the demo's `SPENDGUARD_UNIT_ID` set, ledger reserve rows exist for the demo budget (the reserve would be rejected with `INVALID_REQUEST: claim[0].unit.unit_id empty` otherwise) | day-1 unitId proven against the real sidecar |
| TA-10 | `pnpm -C sdk/typescript-mastra run size` | dist/index.js ≤ 40 KB min / ≤ 12 KB gz |
| TA-11 | Node engine gate: demo runner uses `node:22.13-bookworm-slim`; `package.json` engines `>=22.13.0` | Mastra floor honored, not "harmonized" to 20.10 |
| TA-12 | `[VERIFY-AT-IMPL]` register closure: every V1–V8 marker has a recorded answer (slice doc section "VERIFY-AT-IMPL pins") with the `@mastra/core` version it was pinned against | no unresolved markers at ship |

## 4. Slice → test mapping

| Slice | Tests added/passing |
|---|---|
| COV_D38_00 | TA-06 (first run), TA-07, TA-08 |
| COV_D38_01 | sanity import test (pre-TP-01) |
| COV_D38_02 | TP-01..TP-22, TP-32..TP-35; V1/V2/V3/V5 pins recorded |
| COV_D38_03 | TP-23..TP-31; V4/V7 pins recorded |
| COV_D38_04 | TP-36..TP-38, full TP suite to coverage floor, TA-01/TA-02 |
| COV_D38_05 | TA-03, TA-04, TA-05, TA-09, TA-11; V6 pin recorded |
| COV_D38_06 | TA-06 (final run), TA-10, TA-12; V8 pin recorded |
