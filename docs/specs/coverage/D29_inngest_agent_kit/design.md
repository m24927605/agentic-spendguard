# D29 — Inngest AgentKit adapter (`@spendguard/inngest-agent-kit`)

**Status:** Spec — Tier 3 (build plan `framework-coverage-build-plan-2026-06.md` §2.3).
**Parent strategy:** [`framework-coverage-2026-06.md`](../../../strategy/framework-coverage-2026-06.md), Pattern 1.
**Owner sub-agent:** Frontend Developer.
**Upstream contract:** [`D05_ts_sdk_substrate/design.md`](../D05_ts_sdk_substrate/design.md) §4. D29 imports — does not re-derive — every symbol locked there.
**Sibling pattern:** [`D04_langchain_ts/design.md`](../D04_langchain_ts/design.md). Same narrative, different host primitive (durable step, not `RunManager`).

## 1. Problem

Inngest AgentKit (`@inngest/agent-kit@^0.1`, TypeScript, Apache-2.0) wraps every LLM call as a durable step via `step.ai.wrap()` / `step.ai.infer()`. On failure Inngest retries the step with the SAME `step.id` and SAME `idempotencyKey` — the cleanest pre-call hook SpendGuard has in TS. Without an adapter, AgentKit calls the provider with zero pre-call refusal, and retry storms can double-bill if SpendGuard's reservation isn't deduped against step identity.

D29 ships the wrap that places `client.reserve` inside the step body before the provider call and `client.commitEstimated` after, reusing Inngest's `step.idempotencyKey` as the SpendGuard idempotency seed so retries hit D05's `DecisionCache` and short-circuit.

## 2. Goals

1. Publish `@spendguard/inngest-agent-kit` npm package, version `0.1.0`, Apache-2.0, at `sdk/typescript/integrations/inngest-agent-kit/`.
2. Public surface: `wrapWithSpendGuard(stepAi, client, options)` factory; returns a `step.ai`-shaped object whose `infer()` / `wrap()` calls execute reserve → provider → commit. Callers swap one line.
3. Behaviour parity with D04: PRE → throw to halt; POST `SUCCESS` → commit with extracted usage; provider error → commit `PROVIDER_ERROR`.
4. **Retry dedup is the headline feature.** Derive SpendGuard's `idempotencyKey` deterministically from Inngest's `step.idempotencyKey` (or `step.id`) so retries produce the same key and D05's `DecisionCache` returns the cached outcome.
5. Demo mode `agent_real_inngest_agent_kit`: `examples/inngest-agent-kit/` Node script runs an AgentKit function via Inngest's in-memory dev runtime against the sidecar UDS, proving (a) reserve fires before the OpenAI HTTP call leaves the process, (b) denied budget short-circuits, (c) a forced retry yields one reservation and one commit.
6. ESM-only, Node 20.10+; peer-deps `@spendguard/sdk@^0.1.0`, `@inngest/agent-kit@^0.1`.

## 3. Non-goals

- Wrapping non-`step.ai` Inngest steps (raw `step.run`).
- Mid-stream gating — PRE + COMMIT only.
- Function-level middleware — wrap is per-step.
- Cross-step budget enforcement — contract-layer concern.
- Approval-resume UI — `ApprovalRequired` propagates into Inngest's failure handler; documented only.

## 4. Public surface — LOCKED

```ts
import { wrapWithSpendGuard, type WrapOptions }
  from "@spendguard/inngest-agent-kit";

inngest.createFunction({ id: "agent-fn" }, { event: "agent/run" },
  async ({ step }) => {
    const sgStep = wrapWithSpendGuard(step.ai, client, {
      budgetId, windowInstanceId, unit, pricing,
      claimEstimator: ({ input }) => [{ scopeId, amountAtomic: "1000000", unit }],
    });
    return await sgStep.infer("call-openai", {
      model: openai({ model: "gpt-4o-mini" }),
      body: { messages: [{ role: "user", content: "hi" }] },
    });
  });
```

`WrapOptions` mirrors `SpendGuardCallbackHandlerOptions` (D04 §4) field-for-field minus `route` (defaults to `"llm.call.inngest"`). `ClaimEstimatorInput` carries: `stepId`, `attempt`, `inngestIdempotencyKey`, `model`, `body`, `eventId`, `runId`. Estimator type: `(input) => readonly BudgetClaim[]`.

`wrapWithSpendGuard` returns an object whose `infer(name, opts)` and `wrap(name, fn, ...args)` signatures match `step.ai` exactly. Type-preserving: TS infers return types from the wrapped `step.ai`.

## 5. Architecture

The wrap returns new `infer`/`wrap` that call the original with a body augmented to: (1) compute `idempotencyKey` from step identity; (2) call `client.reserve(...)`; (3) `await` the provider; (4) call `client.commitEstimated(...)`. If reserve throws, the step body throws — Inngest records the step as failed, no provider call leaves the process. On retry, same `step.id` + same `inngestIdempotencyKey` produce the same SpendGuard key; D05's `DecisionCache` returns the prior outcome.

Key facts:

- **Inngest step identity drives SpendGuard identity.** `llmCallId = step.id`, `runId = ctx.runId`, `stepId = step.id`. `decisionId = deriveUuidFromSignature(step.id, { scope: "decision_id" })` — attempt-invariant.
- `idempotencyKey = deriveIdempotencyKey({tenantId, sessionId, runId, stepId, llmCallId, trigger: "LLM_CALL_PRE"})` — byte-identical across retries.
- No inflight Map — PRE and POST are local-variable scoped within one `await`.
- Token-usage + `provider_event_id` extraction mirrors D04 `extract.ts`.

## 6. Locked design decisions

1. **Factory over class** — matches AgentKit's `step.ai`-as-namespace shape.
2. **`step.id` is the `llmCallId`** — durable, attempt-invariant.
3. **Inngest idempotency reused.** Adapter reads `ctx.step.idempotencyKey` if present; falls back to `step.id`.
4. **Retry dedup is contractual and tested.** Demo gate proves one reserve / one commit across N retries.
5. **Demo mode is `agent_real_inngest_agent_kit`** — Inngest dev runtime in-process.
6. **PRE+POST only.** AgentKit's `step.ai.infer` is non-streaming.
7. **Peer-deps, not deps.**
8. **No re-export of `@spendguard/sdk`.**

## 7. Slice plan

| Slice | Title | Size |
|---|---|---|
| `COV_D29_01_pkg_init` | package.json, tsconfig, tsup, biome, vitest, peer-dep wiring vs D05 | S |
| `COV_D29_02_wrap_factory` | `wrapWithSpendGuard(stepAi, client, opts)` factory; options types; type-preserving `infer`/`wrap` | S |
| `COV_D29_03_reserve_commit_retry_dedup` | reserve/commit wiring; Inngest idempotency reuse; retry dedup; PROVIDER_ERROR branch | M |
| `COV_D29_04_tests_mock_agent_kit` | vitest vs mock `@inngest/agent-kit` step harness; ≥ 22 tests including retry dedup | M |
| `COV_D29_05_demo_agent_real_inngest_agent_kit` | `examples/inngest-agent-kit/` Node script + run_demo.py dispatch + compose service + retry demo gate | M |
| `COV_D29_06_docs_publish` | docs page, README adapter row, npm OIDC publish workflow | S |

Total: **6 slices**, 3 S + 3 M. Acceptance in `acceptance.md`.
