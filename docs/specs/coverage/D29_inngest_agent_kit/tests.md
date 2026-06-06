# D29 — Tests

## 1. Test layout

```
sdk/typescript/integrations/inngest-agent-kit/tests/
├── wrap.test.ts             # wrapWithSpendGuard reserve/commit unit tests
├── retryDedup.test.ts       # the headline retry-dedup guarantee
├── errors.test.ts           # throw propagation, ApprovalRequired, PROVIDER_ERROR
├── identity.test.ts         # identity-derivation invariants
├── extract.test.ts          # usage parsers per provider shape
├── treeShaking.test.ts      # bundle does not pull @grpc/grpc-js directly
├── _support/
│   ├── mockSidecar.ts       # re-exports the @spendguard/sdk mock
│   └── mockAgentKit.ts      # tiny step.ai shim that fires real-shape events
└── e2e/
    └── inngestDev.test.ts   # runs Inngest dev runtime in-memory + stubbed fetch
```

## 2. Coverage targets

| Module | Statement / branch target |
|---|---|
| `wrap.ts` | ≥ 92 % stmt, ≥ 88 % branch |
| `identity.ts` | 100 % stmt + branch |
| `extract.ts` | 100 % stmt, ≥ 90 % branch |
| `options.ts` (types only) | n/a |

Overall package floor: **≥ 92 % statements, ≥ 88 % branches** (slightly tighter than D04 because the surface is smaller).

## 3. Tests by module

### 3.1 `wrap.test.ts`

| # | Test | Verifies |
|---|---|---|
| W-01 | `wrapWithSpendGuard(stepAi, client, opts).infer(name, opts)` fires `client.reserve` with `trigger=LLM_CALL_PRE` | wire-level mock assertion |
| W-02 | `reserve` is called BEFORE the inner `stepAi.infer` body runs | recorded `(ts, op)` sequence: reserve < provider |
| W-03 | `llmCallId` on the reserve request equals `ctx.step.id` | identity correctness |
| W-04 | `stepId` on the reserve request equals `ctx.step.id` | identity correctness |
| W-05 | `decisionId` equals `deriveUuidFromSignature(seed, {scope:"decision_id"})` where `seed = inngestIdempotencyKey ?? step.id` | golden value |
| W-06 | `idempotencyKey` byte-matches `deriveIdempotencyKey({...})` with attempt-invariant inputs | byte-equality vs. fixture |
| W-07 | `claimEstimator(input)` invoked exactly once per `infer` call | spy call count |
| W-08 | `claimEstimator` receives `{stepId, attempt, inngestIdempotencyKey, runId, model, body, eventId}` | shape assertion |
| W-09 | When `runtimeCtx` is undefined, adapter falls back to `name` as `stepId` and empty `runId` (documented degradation) | graceful fallback |
| W-10 | `route` defaults to `"llm.call.inngest"`; consumer override propagates | sanity |
| W-11 | `commitEstimated` fires AFTER `stepAi.infer` resolves; `outcome="SUCCESS"`; reads usage via `extractTotalTokens` | wire-level |
| W-12 | `providerEventId` on commit comes from `extractProviderEventId(result)` | wire-level |
| W-13 | `wrapWithSpendGuard(stepAi).wrap(name, fn, ...args)` also fires reserve before `fn` runs and commit after | wrap parity with infer |
| W-14 | Type-preservation: TS infers `infer()` return type from the original `stepAi.infer` overload (compile-time gate) | type-only test via `expectType` |
| W-15 | Two concurrent `infer` calls with different step.ids do not cross-correlate | concurrency isolation |
| W-16 | Adapter does NOT mutate the `options` object passed in | input hygiene |
| W-17 | Reserve request includes `attempt=0` for first execution (passed through as part of `claimEstimator` input only — not on the wire) | input audit |

### 3.2 `retryDedup.test.ts` — the headline guarantee

| # | Test | Verifies |
|---|---|---|
| R-01 | Two `infer` calls with the SAME `ctx.step.id` + SAME `inngestIdempotencyKey` derive the SAME `idempotencyKey` | byte equality |
| R-02 | Two `infer` calls with the SAME `ctx.step.id` + SAME `inngestIdempotencyKey` BUT different `attempt` values still derive the SAME `idempotencyKey` | attempt-invariance |
| R-03 | When the step body throws on attempt 0 and succeeds on attempt 1, the mock sidecar records exactly ONE `reserve` round-trip and ONE `commit` round-trip | retry dedup via D05 DecisionCache (cached decision returned on attempt 1) |
| R-04 | Attempt 0 commits `PROVIDER_ERROR`; attempt 1 succeeds — net audit shape is one PRE row, one PROVIDER_ERROR post, then a SUCCESS post deduped against the SAME decision_id | wire-level |
| R-05 | When `ctx.step.idempotencyKey` is absent but `ctx.step.id` is stable, dedup still works (falls back to step.id seed) | fallback path |
| R-06 | Without D05's DecisionCache enabled (`SPENDGUARD_DISABLE_DECISION_CACHE=1`), the second attempt's reserve hits the sidecar — the test still passes because the sidecar's own audit dedup catches it via `idempotencyKey` | layered defence proof |
| R-07 | Three retries on the same step result in exactly ONE PRE audit row in the mock-sidecar journal | bound-check |
| R-08 | A NEW Inngest function invocation (new `ctx.runId`) for the same step name produces a DIFFERENT `idempotencyKey` (not deduped against the prior run) | scope correctness — retries dedupe, fresh runs don't |

R-03 is the most important test in the package: it proves the headline goal.

### 3.3 `errors.test.ts`

| # | Test | Verifies |
|---|---|---|
| E-01 | `DecisionStopped` thrown from `client.reserve` propagates out of `sgStep.infer` | end-to-end throw |
| E-02 | `DecisionStopped` short-circuits the call: inner `stepAi.infer` is NEVER invoked | spy assertion `callCount=0` |
| E-03 | `DecisionDenied` propagates identically | parity |
| E-04 | `ApprovalRequired` without `onApprovalRequired` propagates | default path |
| E-05 | `ApprovalRequired` with `onApprovalRequired` returning resumed `DecisionOutcome` continues the call | resume path |
| E-06 | `ApprovalRequired` with `onApprovalRequired` returning null propagates the original error | passthrough |
| E-07 | `SidecarUnavailable` propagates as-is (no auto-suppress) | strict mode |
| E-08 | `claimEstimator` throwing → error propagates through `infer` | not swallowed |
| E-09 | Inner `stepAi.infer` throwing (provider error) → `commitEstimated(outcome="PROVIDER_ERROR", estimatedAmountAtomic="0")` fires THEN error re-throws | post-error commit |
| E-10 | `commitEstimated` itself failing does NOT mask the original provider error — both are surfaced (provider error wins, commit failure logged) | error precedence |

### 3.4 `identity.test.ts`

| # | Test | Verifies |
|---|---|---|
| I-01 | `deriveIdentity({tenantId, sessionId, input})` returns deterministic output for the same input | determinism |
| I-02 | Same `stepId` + different `attempt` → same `idempotencyKey` | attempt-invariance |
| I-03 | Same `stepId` + different `inngestIdempotencyKey` → DIFFERENT `idempotencyKey` | seed precedence |
| I-04 | Missing `inngestIdempotencyKey` falls back to `stepId` as seed | fallback |
| I-05 | Different `runId` → different `idempotencyKey` (per D05 derivation rule) | scope |
| I-06 | `decisionId` is a valid UUIDv7-shape string | format |
| I-07 | `idempotencyKey` matches `sg-[0-9a-f]{32}` pattern | format |

### 3.5 `extract.test.ts`

| # | Test | Verifies |
|---|---|---|
| X-01 | OpenAI shape: `result.usage.total_tokens` read | primary path |
| X-02 | Anthropic shape: `result.usage_metadata.total_tokens` read | secondary path |
| X-03 | Fallback: `result.response_metadata.token_usage.total_tokens` | tertiary path |
| X-04 | Returns `0` when none present | safety |
| X-05 | `providerEventId`: `result.id` first | order |
| X-06 | Falls back to `result.response_metadata.id` | order |
| X-07 | Returns `""` if absent | safety |
| X-08 | Robust to non-object `usage` field | drift tolerance |

### 3.6 `treeShaking.test.ts`

| # | Test | Verifies |
|---|---|---|
| T-01 | `import { WrapOptions } from "@spendguard/inngest-agent-kit"` pulls 0 KB of runtime — type only | esbuild metafile check |
| T-02 | `dist/index.js` does not statically `import` `@grpc/grpc-js` (substrate owns that) | grep + bundle inspect |
| T-03 | Minified bundle ≤ 35 KB; gz ≤ 10 KB | size gate |

### 3.7 `e2e/inngestDev.test.ts`

End-to-end against the `inngest` package's in-memory dev runner with `global.fetch` stubbed:

| # | Test | Verifies |
|---|---|---|
| EE-01 | Happy-path: a single-step Inngest function with `sgStep.infer` reserves before the OpenAI fetch fires | event ordering |
| EE-02 | Reserve denied → 0 OpenAI `fetch` calls | budget short-circuit |
| EE-03 | `commitEstimated` ack AFTER `fetch` resolves with the OpenAI response | event ordering |
| EE-04 | Inngest function configured with `retries: 2`, body fails on attempts 0 + 1, succeeds on attempt 2 → mock sidecar journal contains exactly 1 `RequestDecision` ack | retry dedup E2E |
| EE-05 | Inngest function with `retries: 2`, body fails on ALL attempts → mock sidecar journal contains 1 PRE + N PROVIDER_ERROR commits sharing the same `decision_id` | error path E2E |

## 4. Mock AgentKit support

`tests/_support/mockAgentKit.ts` exports `makeMockStepAi(behavior)` which returns a `{ infer, wrap }` shaped object whose body is configurable. The mock simulates Inngest's retry semantics: when `behavior.throwsOnAttempt(n)` is set, calling `infer` with `ctx.step.attempt = n` throws; the test harness re-invokes with `attempt = n+1` and the SAME `step.id` + SAME `idempotencyKey`. This isolates D29's behaviour from the actual Inngest runtime for unit tests.

## 5. Demo-mode regression

Slice 5 adds:

| Gate | Command | Pass condition |
|---|---|---|
| D-01 | `make demo-up DEMO_MODE=agent_real_inngest_agent_kit` | exit 0; "result:" printed |
| D-02 | `make demo-up DEMO_MODE=agent_real_inngest_agent_kit SPENDGUARD_DEMO_DENY=1` | exit reflects denied call; `/tmp/openai-fetch-log.jsonl` contains 0 OpenAI request lines |
| D-03 | The demo log shows reserve event timestamp BEFORE first OpenAI HTTP call timestamp | proof of pre-call gating |
| D-04 | `compose.yml` `demo-inngest-agent-kit` service Node 20 base image; npm install on cold-start succeeds | image gate |
| D-05 | `make demo-up DEMO_MODE=agent_real_inngest_agent_kit SPENDGUARD_DEMO_RETRIES=2` (forces 3 attempts via a body that throws on attempts 0 + 1) yields exactly 1 `LLM_CALL_PRE` row in `audit_outbox` for that run | retry dedup proven against the real sidecar |

D-05 is the headline demo gate.

## 6. Cross-language consistency check

For Inngest there is no Python counterpart (Inngest's TS-first SDK is the only one we adapt). But the substrate-level `deriveIdempotencyKey` byte-equality vs. Python is already gated by D05 §11. D29 reuses D05's fixture vectors via a thin smoke test:

| Gate | Command | Pass condition |
|---|---|---|
| CL-01 | `pnpm run test tests/idempotencyParity.test.ts` (per D29 — picks the 8 vectors with `inngest_*` keys) | TS derivation matches the fixture |

The fixture (`sdk/fixtures/cross-language/inngest_agent_kit_v1.json`) is created in slice 4 of D29 with 8 vectors covering: with/without `inngestIdempotencyKey`, attempt-0/attempt-1/attempt-N invariance, distinct `runId`s.

## 7. Bench

`tests/wrap.bench.ts` (vitest bench):

- `wrapWithSpendGuard` construction: < 0.05 ms
- `sgStep.infer` overhead vs. raw `stepAi.infer` (sidecar mocked): < 0.5 ms
- `deriveIdentity`: < 0.1 ms

Advisory in v0.1.0; promoted to blocking in v0.2 once D05 A6.1/A6.2 lands.

## 8. Slice → test mapping

| Slice | Tests added |
|---|---|
| `COV_D29_01_pkg_init` | sanity `import { wrapWithSpendGuard } from "../src"` import test |
| `COV_D29_02_wrap_factory` | wrap.test.ts W-09, W-13, W-14, W-16; identity.test.ts I-06, I-07 |
| `COV_D29_03_reserve_commit_retry_dedup` | wrap.test.ts W-01..W-08, W-10..W-12, W-15, W-17; retryDedup.test.ts R-01..R-08; errors.test.ts E-01..E-10; identity.test.ts I-01..I-05; extract.test.ts X-01..X-08 |
| `COV_D29_04_tests_mock_agent_kit` | e2e/inngestDev.test.ts EE-01..EE-05; CL-01 |
| `COV_D29_05_demo_agent_real_inngest_agent_kit` | D-01..D-05 |
| `COV_D29_06_docs_publish` | treeShaking.test.ts T-01..T-03; publish dry-run |
