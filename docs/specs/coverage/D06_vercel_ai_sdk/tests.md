# D06 — Tests

Coverage map: unit + integration + provider matrix + Mastra + demo regression. Coverage target: ≥ 85% statements / 80% branches / 85% functions / 85% lines.

## 1. Unit tests — `tests/middleware.test.ts`

Validates the factory and option plumbing.

| # | Case | Assertion |
|---|---|---|
| 1.1 | `createSpendGuardMiddleware(opts)` returns an object with `transformParams`, `wrapGenerate`, `wrapStream` keys | All three are functions |
| 1.2 | Missing `client` → throws `Error("client is required")` | exact message |
| 1.3 | Missing `budgetId` / `windowInstanceId` / `unit` / `pricing` → individual `Error` thrown | per-field assertion |
| 1.4 | `transformParams` stashes a `StashEntry` reachable via the WeakMap | Stash holds `identity`, `outcome`, `runId`, `traceparent`, `tracestate`, `route` |
| 1.5 | `wrapGenerate` called without prior `transformParams` → throws `Error` mentioning `wrapLanguageModel()` | exact substring match |
| 1.6 | `wrapStream` called without prior `transformParams` → same `Error` shape | exact substring match |
| 1.7 | When `runIdProvider` is not set AND no `currentRunPlan()` binding → throws `SpendGuardConfigError` on first `transformParams` | one-shot config error |
| 1.8 | `runIdProvider` wins over `currentRunPlan()` if both present | identity-derivation uses provider's runId |

## 2. Identity derivation — `tests/identity.test.ts`

| # | Case | Assertion |
|---|---|---|
| 2.1 | Bit-identical params produce identical `idempotencyKey` across two calls | byte equality |
| 2.2 | Changing `temperature` produces a different signature | byte inequality |
| 2.3 | Changing `prompt` content but keeping settings equal produces different signature | byte inequality |
| 2.4 | `stepId` uses `${runId}:call:${signature.slice(0,16)}` shape | regex match |
| 2.5 | `llmCallId` and `traceDecisionId` are RFC 9562 UUIDv5-shape strings (36 chars, dashed) | UUID regex |
| 2.6 | Custom `callSignature` override is honoured | identity uses caller's signature |
| 2.7 | Cross-language parity: a Python pydantic-ai run with same `(messages, settings)` produces the same `idempotencyKey` (run against the D05 cross-language fixture) | byte equality vs `sdk/fixtures/cross-language/v1.json` |

## 3. Streaming instrumentation — `tests/streaming.test.ts`

| # | Case | Assertion |
|---|---|---|
| 3.1 | Stream emits 3 deltas + 1 finish → `onFinish` is called exactly once with usage totals from the finish part | call count = 1; tokens = sum |
| 3.2 | Stream emits parts then throws → `onError` called once; `onFinish` never called | mutual exclusion |
| 3.3 | Consumer calls `stream.cancel()` mid-stream → `onError` called once with the cancel reason | one-shot |
| 3.4 | `onFinish` throws internally → consumer of the stream still sees `finish` part forwarded (commit failure does not corrupt stream) | downstream sees finish |
| 3.5 | Race: finish part lands AND consumer cancels simultaneously → exactly one of `onFinish` / `onError` fires (first-wins) | terminal-state machine respected |
| 3.6 | Empty stream (provider returns no parts) → `onFinish` called with `totalTokens=0` | zero-tokens commit allowed |

## 4. Provider matrix — `tests/providers/openai.test.ts` + `tests/providers/anthropic.test.ts`

Each provider gets 6 cases. Mock sidecar via `tests/_support/mockSidecar.ts` (UDS `@grpc/grpc-js` server). Provider responses use recorded JSON fixtures under `tests/_support/recordedResponses/{openai,anthropic}/`.

| # | Case | Provider | Assertion |
|---|---|---|---|
| 4.1 | Reserve CONTINUE → `generateText` succeeds → COMMIT emitted | openai + anthropic | mock sidecar logs `reserve` then `emitLlmCallPost` then `confirmPublishOutcome` |
| 4.2 | Reserve STOP → `generateText` throws `DecisionStopped` | openai + anthropic | exception type + `doGenerate` never called |
| 4.3 | Reserve DENY → `generateText` throws `DecisionDenied` | openai + anthropic | exception type + denial flows up to caller |
| 4.4 | Reserve CONTINUE → provider throws → ROLLBACK (`release`) | openai + anthropic | mock sidecar logs `reserve` then `release` with `reasonCode=PROVIDER_ERROR` |
| 4.5 | `streamText` happy path → commit after `finish` part lands | openai + anthropic | commit time-ordering: all stream parts consumed BEFORE commit RPC |
| 4.6 | `streamText` cancelled mid-stream → release fires | openai + anthropic | mock sidecar logs `release` once; commit never |

Recorded provider responses are committed under `tests/_support/recordedResponses/` — no live API key needed. The mock is wrapped with vitest mocks for `fetch` so `@ai-sdk/openai` / `@ai-sdk/anthropic` see realistic provider behaviour without network.

## 5. Mastra integration — `tests/mastra/agent.test.ts`

| # | Case | Assertion |
|---|---|---|
| 5.1 | `Agent({ model: wrapLanguageModel(...) })` with `createSpendGuardLanguageMiddleware` produces a working agent | `agent.generate(...)` returns text |
| 5.2 | Reserve CONTINUE → Agent.generate → COMMIT emitted | mock sidecar reserve+commit ordering |
| 5.3 | Reserve STOP → Agent.generate throws `DecisionStopped` | denial propagates through Mastra |
| 5.4 | Mastra streaming via `agent.stream(...)` → commit after stream completes | reserve→stream→commit time-order |
| 5.5 | Mastra Agent retried twice (e.g. via `experimental_repairText`) → same `idempotencyKey` collapses on sidecar | reserve called twice; sidecar dedup returns identical decision both times |
| 5.6 | The `@spendguard/vercel-ai/mastra` re-export is an alias (same function reference) | `createSpendGuardMiddleware === createSpendGuardLanguageMiddleware` strict equality |

## 6. Retry / idempotency — `tests/retry.test.ts`

| # | Case | Assertion |
|---|---|---|
| 6.1 | v5 SDK's internal retry (maxRetries=2) on transient provider error → middleware re-runs `transformParams` with identical params → same `idempotencyKey` produced | byte equality |
| 6.2 | After successful retry → exactly ONE commit emitted to sidecar (sidecar dedupes the reserve) | commit count = 1 |
| 6.3 | After failed retry exhaustion → exactly ONE release emitted | release count = 1 |

## 7. RunPlan binding — `tests/runPlan.test.ts`

| # | Case | Assertion |
|---|---|---|
| 7.1 | `await withRunPlan({ runId, plannedCalls: 3 }, async () => generateText(...))` → middleware picks up runId from AsyncLocalStorage | runId reaches sidecar reserve call |
| 7.2 | Nested `generateText` calls inside the same `withRunPlan` block use the SAME runId but distinct stepIds | step-id divergence + run-id stability |
| 7.3 | Two concurrent `withRunPlan` blocks (`Promise.all`) do not leak runIds between each other | strict isolation |

## 8. Demo regression — `tests/demo/agent_real_vercel_ai_ts.test.ts`

| # | Case | Assertion |
|---|---|---|
| 8.1 | `make demo MODE=agent_real_vercel_ai_ts` exits 0 | exit code |
| 8.2 | Demo emits reserve → generate → commit sequence visible in audit log | grep `LLM_CALL_PRE` + `LLM_CALL_POST` events |
| 8.3 | Demo's Mastra variant also exits 0 | exit code (sub-test `agent_real_mastra`) |
| 8.4 | Streaming variant of the demo (`agent_real_vercel_ai_ts_stream`) commits AFTER stream completes — verified by event timestamps | `commit.timestamp > last_stream_part.timestamp` |

## 9. Tree-shaking — `tests/treeShaking.test.ts`

| # | Case | Assertion |
|---|---|---|
| 9.1 | `import { createSpendGuardMiddleware } from "@spendguard/vercel-ai"` bundles to ≤ 30 KB minified (excluding `@spendguard/sdk` + `ai` peer deps) | esbuild metadata diff |
| 9.2 | Importing the Mastra subpath does NOT pull additional code beyond the alias | bundle size equality vs main entry |
| 9.3 | `@spendguard/sdk` and `ai` are correctly externalised in the bundle | dist contains no inlined peer-dep code |

## 10. Type-level — `tests/types.test-d.ts` (vitest + `expectTypeOf`)

| # | Case | Assertion |
|---|---|---|
| 10.1 | `createSpendGuardMiddleware` returns `LanguageModelV2Middleware` exactly | typeof equality |
| 10.2 | `SpendGuardMiddlewareOptions` requires `client`, `budgetId`, `windowInstanceId`, `unit`, `pricing` | omit any → compile error |
| 10.3 | `claimEstimator`, `callSignature`, `runIdProvider`, `route` are optional | omit all → compiles |
| 10.4 | `wrapLanguageModel({ model: openai("..."), middleware: createSpendGuardMiddleware(...) })` typechecks against `ai@5` | green tsc |

## 11. Coverage thresholds — `vitest.config.ts`

```ts
coverage: {
  provider: "v8",
  thresholds: {
    statements: 85,
    branches:   80,
    functions:  85,
    lines:      85,
  },
  exclude: ["src/_proto/**", "tests/**", "scripts/**"],
}
```

## 12. CI matrix

| Shard | Runtime | Suite |
|---|---|---|
| 12.1 | Node 22 LTS | full test suite + type-level + tree-shaking + demo regression |
| 12.2 | Node 20.10 | full test suite (no demo regression) |
| 12.3 | Bun 1.1+ | unit-only subset (`middleware`, `identity`, `streaming`, `claim`); advisory |
| 12.4 | Provider matrix | `openai.test.ts` + `anthropic.test.ts` against recorded fixtures |
| 12.5 | Mastra | `tests/mastra/agent.test.ts` against `@mastra/core@^0.x` dev-dep |
