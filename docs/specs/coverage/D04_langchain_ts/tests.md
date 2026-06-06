# D04 — Tests

## 1. Test layout

```
sdk/typescript/integrations/langchain/tests/
├── handler.test.ts          # SpendGuardCallbackHandler lifecycle vs. mock sidecar
├── streaming.test.ts        # streaming PRE + POST behaviour
├── errors.test.ts           # throw propagation, ApprovalRequired, PROVIDER_ERROR commit
├── extract.test.ts          # token-usage + provider_event_id parsers
├── inflight.test.ts         # InflightMap eviction + take semantics
├── treeShaking.test.ts      # bundle does not pull @grpc/grpc-js when only types imported
├── _support/
│   ├── mockSidecar.ts       # re-exports the mock from sdk/typescript/tests/_support
│   └── mockLangchain.ts     # tiny BaseChatModel sub that fires real RunManager events
└── e2e/
    └── chatOpenAI.test.ts   # @langchain/openai with stubbed global fetch
```

## 2. Coverage targets

| Module | Statement / branch target |
|---|---|
| `handler.ts` | ≥ 90 % stmt, ≥ 85 % branch |
| `inflight.ts` | 100 % stmt + branch |
| `extract.ts` | 100 % stmt, ≥ 90 % branch |
| `options.ts` (types only) | n/a |

Overall package floor: **≥ 90 % statements, ≥ 85 % branches**.

## 3. Tests by module

### 3.1 `handler.test.ts`

| # | Test | Verifies |
|---|---|---|
| H-01 | `handleChatModelStart` fires `client.reserve` with `trigger=LLM_CALL_PRE` | wire-level mock assertion |
| H-02 | `handleChatModelStart` passes `runId` AS `llmCallId` on the reserve request | identical strings |
| H-03 | `handleChatModelStart` derives `decisionId` via `deriveUuidFromSignature(sig, {scope:"decision_id"})` | golden value |
| H-04 | `handleChatModelStart` derives `idempotencyKey` via `deriveIdempotencyKey({tenantId, sessionId, runId, stepId, llmCallId, trigger})` | byte-equality vs. fixture |
| H-05 | `handleChatModelStart` calls `claimEstimator(input)` once per event | spy call count |
| H-06 | `claimEstimator` receives `kind="chat"` for chat-model events | kind discriminant |
| H-07 | `handleLLMStart` calls `claimEstimator` with `kind="llm"` and `prompts` array set | kind discriminant |
| H-08 | `handleLLMStart` derives the SAME `idempotencyKey` as `handleChatModelStart` for matching inputs | parity gate |
| H-09 | `route` defaults to `"llm.call"`; consumer override propagates | sanity |
| H-10 | `parentRunId` from LangChain forwarded to reserve | run-tree correctness |
| H-11 | `handleLLMEnd` reads inflight entry for `runId` and fires `commitEstimated` once | spy assertion |
| H-12 | `handleLLMEnd` after a NOT-OURS `runId` (e.g. ignored event) is a no-op | guard branch |
| H-13 | `handleLLMEnd` extracts `total_tokens` from `usage_metadata` (LangChain 0.3 path) | golden output |
| H-14 | `handleLLMEnd` falls back to `response_metadata.token_usage.total_tokens` | older path |
| H-15 | `handleLLMEnd` extracts `provider_event_id` from `response_metadata.id` then `response_id` | order |
| H-16 | Two concurrent invocations with different `runId`s do not cross-correlate | inflight isolation |
| H-17 | Handler is reusable across runs (no per-run construction needed) | lifecycle |
| H-18 | `awaitHandlers = true` and `raiseError = true` are set on the instance | LangChain 0.3 requirement for throw-on-deny |

### 3.2 `streaming.test.ts`

| # | Test | Verifies |
|---|---|---|
| S-01 | Streaming chat call (`stream()`) fires `handleChatModelStart` once, `handleLLMEnd` once at stream completion | PRE+POST count |
| S-02 | `handleLLMNewToken` events do NOT fire `reserve` | streaming chunks are not gated |
| S-03 | A stream that errors mid-flight triggers `handleLLMError` and PROVIDER_ERROR commit | error path |
| S-04 | A stream aborted by `AbortController` triggers `handleLLMError` once | abort path |

### 3.3 `errors.test.ts`

| # | Test | Verifies |
|---|---|---|
| E-01 | `DecisionStopped` thrown from `client.reserve` propagates out of `model.invoke` | end-to-end throw |
| E-02 | `DecisionStopped` short-circuits the call: no LLM HTTP fetch fires | bound a `fetch` spy and assert 0 calls |
| E-03 | `DecisionDenied` (non-Stopped) propagates same way | parity |
| E-04 | `ApprovalRequired` without `onApprovalRequired` propagates | default path |
| E-05 | `ApprovalRequired` with `onApprovalRequired` that returns a resumed `DecisionOutcome` continues the call | resume path |
| E-06 | `ApprovalRequired` with `onApprovalRequired` returning `null` propagates the original error | passthrough |
| E-07 | `SidecarUnavailable` propagates as-is (no auto-suppress) | strict mode |
| E-08 | `claimEstimator` throwing → error propagates through `handleChatModelStart` | not swallowed |
| E-09 | An LLM error (provider 500) calls `handleLLMError` → `commitEstimated(outcome="PROVIDER_ERROR", estimatedAmountAtomic="0")` | post-error commit |
| E-10 | Concurrent invocations: one errors, the other succeeds → exactly one PROVIDER_ERROR commit and one SUCCESS commit | inflight isolation |

### 3.4 `extract.test.ts`

| # | Test | Verifies |
|---|---|---|
| X-01 | Returns `0` for empty `output.generations` | safety branch |
| X-02 | Reads `usage_metadata.total_tokens` when present | primary path |
| X-03 | Reads `response_metadata.token_usage.total_tokens` fallback | older path |
| X-04 | Returns `0` when neither shape present | safety |
| X-05 | `provider_event_id` reads `response_metadata.id` first | order |
| X-06 | Falls back to `response_metadata.response_id` | order |
| X-07 | Returns `""` if both absent | safety |
| X-08 | Robust to `usage_metadata` being a non-object | LangChain 0.3.x minor drift tolerance |

### 3.5 `inflight.test.ts`

| # | Test | Verifies |
|---|---|---|
| IF-01 | `put(runId, entry)` then `take(runId)` returns entry | basic |
| IF-02 | Second `take(runId)` returns `undefined` | one-shot |
| IF-03 | At capacity (10 k), oldest entry is evicted on `put` | FIFO bound |
| IF-04 | `take` of a non-existent key returns `undefined` (no throw) | safety |

### 3.6 `treeShaking.test.ts`

| # | Test | Verifies |
|---|---|---|
| T-01 | `import { SpendGuardCallbackHandlerOptions } from "@spendguard/langchain"` pulls 0 KB of runtime — type only | esbuild metafile check |
| T-02 | `dist/index.js` does not statically `require/import` `@grpc/grpc-js` (it goes through the substrate) | grep + bundle inspect |
| T-03 | Minified bundle ≤ 40 KB; gz ≤ 12 KB | size gate |

### 3.7 `e2e/chatOpenAI.test.ts`

End-to-end against `@langchain/openai`'s `ChatOpenAI` with `global.fetch` stubbed:

| # | Test | Verifies |
|---|---|---|
| EE-01 | Happy-path invoke: `reserve` ack BEFORE first `fetch` call to OpenAI | event ordering — mock sidecar records timestamp, `fetch` records timestamp; assert reserve < fetch |
| EE-02 | Reserve denied → 0 OpenAI `fetch` calls | budget short-circuit proof |
| EE-03 | `commitEstimated` ack AFTER `fetch` resolves with the OpenAI response | event ordering |
| EE-04 | OpenAI 500 → `commitEstimated(outcome="PROVIDER_ERROR")` ack | error path |
| EE-05 | Streaming `stream()` happy-path: reserve before first SSE chunk; commit after final | event ordering |

The fetch stub intercepts every `https://api.openai.com/v1/chat/completions` request. The mock sidecar records `(ts, op)` tuples; ordering assertions are done by comparison on the recorded sequence.

## 4. Mock LangChain support

`tests/_support/mockLangchain.ts` exports `FakeChatModel`, a minimal `BaseChatModel` whose `_generate` returns a hard-coded `LLMResult` with a configurable `usage_metadata`. This gives unit tests deterministic input/output without needing the `@langchain/openai` HTTP fetch — fastpath for H-01..H-18, S-01..S-04, E-01..E-10.

## 5. Demo-mode regression

Slice 5 adds:

| Gate | Command | Pass condition |
|---|---|---|
| D-01 | `make demo-up DEMO_MODE=agent_real_langchain_ts` | exit 0; OpenAI reply printed |
| D-02 | `make demo-up DEMO_MODE=agent_real_langchain_ts SPENDGUARD_DEMO_DENY=1` | exit code reflects denied call; `0` calls to OpenAI captured in the egress-proxy log |
| D-03 | The demo log shows reserve event timestamp BEFORE first OpenAI HTTP call timestamp | proof of pre-call gating |
| D-04 | `compose.yml` `demo-langchain-ts` service Node 20 base image; npm install on cold-start succeeds | image gate |

`D-03` is verified post-run with `psql` against `audit_outbox`: there must be exactly one `LLM_CALL_PRE` row with a `created_at` earlier than the OpenAI request's logged timestamp.

## 6. Cross-language consistency check (lightweight)

The Python `SpendGuardChatModel` and TS `SpendGuardCallbackHandler` MUST produce the same `idempotencyKey` for the same `(tenantId, sessionId, runId, stepId, llmCallId)` tuple. This is asserted by:

| Gate | Command | Pass condition |
|---|---|---|
| CL-01 | `pnpm run test tests/idempotencyParity.test.ts` | TS computes the key, compares against a fixed-vector JSON file at `sdk/fixtures/cross-language/langchain_v1.json` |
| CL-02 | `make -C sdk/python test PYTHONPATH=src TEST=tests/test_langchain_idempotency_parity.py` | Python computes the same keys, compares against the same fixture |

The fixture is created in slice 4 of D04; the Python parity test is added at the same time (a small Python-side patch to the existing langchain test module).

## 7. Bench

A `tests/handler.bench.ts` micro-benchmark (vitest bench) asserts:

- Construction: < 0.1 ms
- `handleChatModelStart` overhead vs. a no-op handler: < 0.5 ms (sidecar mocked, in-memory)
- `handleLLMEnd` overhead: < 0.3 ms

Bench is advisory in v0.1.0; promoted to blocking once the substrate's runtime matrix gate (D05 A6.1/A6.2) is green.

## 8. Slice → test mapping

| Slice | Tests added |
|---|---|
| `COV_D04_01_pkg_init` | A sanity `import { SpendGuardCallbackHandler } from "../src"` import test; no real assertions |
| `COV_D04_02_handler_skeleton` | inflight.test.ts (IF-01..IF-04); handler.test.ts H-17, H-18 |
| `COV_D04_03_reserve_commit_wiring` | handler.test.ts H-01..H-16; errors.test.ts E-01..E-10; extract.test.ts X-01..X-08 |
| `COV_D04_04_tests_mock_sidecar` | streaming.test.ts S-01..S-04; e2e/chatOpenAI.test.ts EE-01..EE-05; CL-01, CL-02 |
| `COV_D04_05_demo_agent_real_langchain_ts` | D-01..D-04 |
| `COV_D04_06_docs_publish` | treeShaking.test.ts T-01..T-03; publish dry-run |
