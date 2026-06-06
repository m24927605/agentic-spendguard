# D04 — LangChain TypeScript adapter (`@spendguard/langchain`)

**Status:** Spec — Tier 1 (build plan `framework-coverage-build-plan-2026-06.md` §2.1).
**Parent strategy:** [`framework-coverage-2026-06.md`](../../../strategy/framework-coverage-2026-06.md), Pattern 1 (framework callback middleware).
**Owner sub-agent:** Frontend Developer.
**Upstream contract:** [`D05_ts_sdk_substrate/design.md`](../D05_ts_sdk_substrate/design.md) §4. D04 imports — does not re-derive — every symbol locked there.
**Python equivalent:** `sdk/python/src/spendguard/integrations/langchain.py` (378 LOC, shipped). D04 mirrors its semantics; it does NOT re-translate its line shape.

## 1. Problem

LangChain.js (`@langchain/core@^0.3`) is the dominant TS agent stack. Shipping only the Python adapter leaves the JS/TS ecosystem unguarded — `ChatOpenAI` / `ChatAnthropic` / any `BaseChatModel` today calls the provider with zero pre-call refusal. The customer is left with two unacceptable options: hand-roll a `BaseCallbackHandler` (and re-derive idempotency / prompt-hash, breaking audit-chain determinism) or fall back to the egress proxy and lose per-call adapter context (route, run_id, step_id).

D04 ships the TS per-call adapter so a TS agent gets the same pre-call reservation + post-call commit lifecycle the Python adapter already provides.

## 2. Goals

1. Publish `@spendguard/langchain` npm package, version `0.1.0`, Apache-2.0, in-tree at `sdk/typescript/integrations/langchain/`.
2. Public surface: `SpendGuardCallbackHandler` extends `@langchain/core/callbacks/base.BaseCallbackHandler`. Drop-in via `callbacks: [handler]` — no model subclassing, works with every `BaseChatModel` / `BaseLLM`.
3. Behaviour parity with Python's `SpendGuardChatModel`: `handleChatModelStart` (and `handleLLMStart`) → `client.reserve(...)` (throw to halt); `handleLLMEnd` → `client.commitEstimated(outcome="SUCCESS")`; `handleLLMError` → `client.commitEstimated(outcome="PROVIDER_ERROR")`.
4. Optional `@spendguard/sdk` `withRunPlan` integration (D05 §4.7); LangChain `RunManager`-issued `runId` is the deterministic call ID across retries.
5. Demo mode `agent_real_langchain_ts`: Node script under `examples/langchain-ts/` drives `ChatOpenAI` + `SpendGuardCallbackHandler` against the sidecar UDS, proving (a) reservation fires BEFORE the OpenAI HTTP call leaves the process and (b) denied budget short-circuits without contacting OpenAI.
6. ESM-only, Node 20.10+; peer-deps `@spendguard/sdk@^0.1.0`, `@langchain/core@^0.3`.

## 3. Non-goals

- Streaming mid-stream gating — pre-stream PRE + post-stream COMMIT only (Python POC parity).
- Tool-call mid-loop gating — each tool call is its own LangChain event; substrate handles per-call PRE/POST; cross-tool budget enforcement is the contract layer's job.
- `BaseChatModel` subclass wrapper — LangChain.js prefers callback handlers; the subclass-wrap is a Python idiom.
- Approval-resume UI — `ApprovalRequired` is raised inside `handleChatModelStart`; v0.1.0 documents the pattern, no built-in UI helper.
- Separate `@spendguard/langgraph` package — LangGraph builds on `BaseChatModel`, so the same handler covers it via `RunnableConfig.callbacks`.

## 4. Public surface — LOCKED

```ts
import { SpendGuardCallbackHandler, type SpendGuardCallbackHandlerOptions }
  from "@spendguard/langchain";

const handler = new SpendGuardCallbackHandler({
  client,                // SpendGuardClient from @spendguard/sdk
  budgetId, windowInstanceId, unit, pricing,
  claimEstimator: (input) => [{ scopeId, amountAtomic: "1000000", unit }],
  // Optional: callSignatureFn, route ("llm.call"), onApprovalRequired
});
const model = new ChatOpenAI({ model: "gpt-4o-mini", callbacks: [handler] });
await model.invoke([new HumanMessage("hi")]);
```

`SpendGuardCallbackHandlerOptions` mirrors the Python `SpendGuardChatModel` constructor field-for-field (camelCase) but drops `inner`. `ClaimEstimatorInput` carries the same data both `handleLLMStart` and `handleChatModelStart` receive: `messages` (or `prompts`), `runId`, `parentRunId`, `tags`, `metadata`, `invocationParams`, `extraParams`. Estimator type: `(input) => readonly BudgetClaim[]`.

## 5. Architecture

LangChain `Runnable.invoke()` → `RunManager` dispatches `handleChatModelStart` → handler calls `SpendGuardClient.reserve` → records the outcome in an in-memory `Map<runId, InflightReservation>` → provider HTTP call → `handleLLMEnd` (or `handleLLMError`) reads + deletes the entry and calls `commitEstimated`.

Key facts:

- `runId` is the correlation key. PRE writes; POST/ERROR reads + deletes. Capacity bounded at 10k entries with FIFO eviction so a forgotten POST cannot leak memory.
- Throw inside `handleChatModelStart` propagates through `CallbackManager` because the handler sets `awaitHandlers = true` and `raiseError = true` — verified in slice 3 against `@langchain/core@0.3`.
- Idempotency key: `deriveIdempotencyKey({tenantId, sessionId, runId, stepId, llmCallId, trigger: "LLM_CALL_PRE"})` with `llmCallId = runId` (LangChain's `RunManager` UUID is the deterministic call ID).

## 6. Locked design decisions

1. **Callback handler over model wrapper** — TS idiom; Python's wrapper is Python idiom. No re-litigation.
2. **`handleChatModelStart` AND `handleLLMStart` both wired** — chat vs. completions both gated. Same reserve path.
3. **`runId` is the `llmCallId`** — deterministic across retries, comes from LangChain's `RunManager`.
4. **Streaming = PRE-only gate in v0.1.0**. POST commits after final chunk. Matches Python POC.
5. **Demo mode is `agent_real_langchain_ts`** — distinct name from Python's `agent_real_langchain`; both modes coexist.
6. **No DEGRADE patch application** — surface as `MutationApplyFailed`. Matches Python pydantic_ai integration parity.
7. **Peer-deps not deps** — `@spendguard/sdk` AND `@langchain/core` are peerDeps. Adapter pins NEITHER; consumer's lock wins.

## 7. Slice plan

| Slice | Title | Size |
|---|---|---|
| `COV_D04_01_pkg_init` | package.json, tsconfig, tsup, biome, vitest, peer-dep wiring vs D05 | S |
| `COV_D04_02_handler_skeleton` | `SpendGuardCallbackHandler` + options + inflight Map; PRE/POST stubbed | S |
| `COV_D04_03_reserve_commit_wiring` | reserve/commit wiring; PROVIDER_ERROR path; throw-on-deny verified | M |
| `COV_D04_04_tests_mock_sidecar` | vitest vs mock sidecar + `@langchain/openai` fetch stub; ≥ 20 tests | M |
| `COV_D04_05_demo_agent_real_langchain_ts` | `examples/langchain-ts/` Node script + run_demo.py dispatcher + compose service | M |
| `COV_D04_06_docs_publish` | docs page, README adapter row, npm OIDC publish workflow | S |

Total: **6 slices**, 3 S + 3 M. Acceptance in `acceptance.md`.
