# D35 — Tests

## 1. Test layout

```
sdk/typescript/integrations/flowise/tests/
├── wrapper.test.ts            # SpendGuardChatModelWrapper.init() lifecycle vs. mock sidecar
├── clientCache.test.ts        # cache hit/miss, separate UDS paths get separate clients
├── claimEstimator.test.ts     # JSON parsing, default fallback, error paths
├── flowiseManifest.test.ts    # INode contract (label/name/type/inputs) frozen against design.md §4
├── _support/
│   ├── mockSidecar.ts         # re-exports D04's mock helper
│   └── mockChatModel.ts       # tiny BaseChatModel sub that fires real RunManager events
└── e2e/
    └── flowiseContainer.test.ts   # testcontainers vs flowiseai/flowise:2.x
```

## 2. Coverage targets

| Module | Statement / branch target |
|---|---|
| `nodes/SpendGuardChatModelWrapper.ts` | ≥ 90 % stmt, ≥ 85 % branch |
| `clientCache.ts` | 100 % stmt + branch |
| `claimEstimator.ts` | 100 % stmt + branch |

Overall package floor: **≥ 90 % statements, ≥ 85 % branches**.

## 3. Tests by module

### 3.1 `wrapper.test.ts`

| # | Test | Verifies |
|---|---|---|
| W-01 | `init()` with all required inputs returns the SAME `chatModel` object reference | identity (not a clone / proxy) |
| W-02 | After `init()`, the returned model's `callbacks` array contains exactly one `SpendGuardCallbackHandler` | handler attached |
| W-03 | `init()` preserves pre-existing handlers in `chatModel.callbacks` (appends, does not replace) | non-destructive |
| W-04 | `init()` throws if `chatModel` input is missing | required-input guard |
| W-05 | `init()` throws if `tenantId` is empty / missing | required-input guard |
| W-06 | `init()` throws if `budgetId` is empty / missing | required-input guard |
| W-07 | `init()` throws if `windowInstanceId` is empty / missing | required-input guard |
| W-08 | `init()` reads `sidecarUds` from env `SPENDGUARD_SIDECAR_UDS` when input is omitted | env fallback |
| W-09 | `init()` throws if neither input nor env supplies `sidecarUds` | required-input guard |
| W-10 | `init()` calls `getOrCreateClient` once per distinct (uds, tenantId) tuple across multiple invocations | cache works |
| W-11 | Triggering a chat call on the returned `chatModel` fires `client.reserve` with `trigger=LLM_CALL_PRE` and `route=llm.call` | D04 wiring proven through the wrapper |
| W-12 | `claimEstimatorJson` empty → default $1 USD-micros claim | default path |
| W-13 | `claimEstimatorJson` populated → the parsed claim is forwarded to `reserve.projectedClaims` | override path |
| W-14 | `route` input override propagates to `reserve.route` | route override |
| W-15 | `DecisionStopped` thrown from sidecar during a chat call propagates out of the LangChain `invoke()` | throw propagation through wrapper |
| W-16 | Two distinct `tenantId`s into the wrapper yield two distinct `SpendGuardClient`s in the cache | cache key correctness |

### 3.2 `clientCache.test.ts`

| # | Test | Verifies |
|---|---|---|
| C-01 | First `getOrCreateClient` call constructs + connects + handshakes | first-call path |
| C-02 | Second call with same (uds, tenantId) returns cached instance — no second connect | cache hit |
| C-03 | Different `sidecarUds` → different instance | key sensitivity |
| C-04 | Different `tenantId` (same uds) → different instance | key sensitivity |
| C-05 | `_resetCacheForTests()` clears state between cases | test hygiene |
| C-06 | Handshake failure during first call propagates and does NOT poison the cache (next call retries) | failure semantics |

### 3.3 `claimEstimator.test.ts`

| # | Test | Verifies |
|---|---|---|
| CE-01 | Empty JSON returns the default $1_000_000 USD-micros claim | default path |
| CE-02 | Whitespace-only JSON same as empty | default path |
| CE-03 | Valid JSON `{"amountAtomic":"5000000","scopeId":"prod"}` parses correctly | happy path |
| CE-04 | Valid JSON missing `amountAtomic` throws with a clear error message | guard |
| CE-05 | Invalid JSON syntax throws with `not valid JSON` in the message | guard |
| CE-06 | JSON `unit` override wins over the constructor `unit` argument | precedence |
| CE-07 | Estimator function returns the SAME claim on repeated calls (no state) | purity |

### 3.4 `flowiseManifest.test.ts`

This file enforces the public-canvas surface (design.md §4) so a slice author cannot accidentally rename `tenantId` to `tenant_id` or shuffle inputs without breaking a test.

| # | Test | Verifies |
|---|---|---|
| M-01 | `label === "SpendGuard ChatModel Wrapper"` | locked label |
| M-02 | `name === "spendGuardChatModelWrapper"` | locked name (Flowise URL slug) |
| M-03 | `type === "BaseChatModel"` | downstream-compatibility |
| M-04 | `category === "Spend Guard"` | side-panel grouping |
| M-05 | `baseClasses` contains exactly `["BaseChatModel", "BaseLanguageModel"]` | downstream-compatibility |
| M-06 | `inputs` field names == exact set `{chatModel, tenantId, budgetId, windowInstanceId, unit, sidecarUds, route, claimEstimatorJson}` | locked input schema |
| M-07 | Required inputs (`chatModel`, `tenantId`, `budgetId`, `windowInstanceId`, `unit`) have `optional !== true` | locked required-vs-optional split |
| M-08 | `module.exports.nodeClass` exists and is the wrapper class | Flowise loader contract |

### 3.5 `e2e/flowiseContainer.test.ts`

Runs against a real Flowise via `testcontainers` (network available CI only — gated by env `D35_E2E=1` so dev machines without Docker do not fail). Skipped automatically when the env flag is unset.

| # | Test | Verifies |
|---|---|---|
| F-01 | Spin up `flowiseai/flowise:2.x` container with `@spendguard/flowise-nodes` mounted at `/root/.flowise/nodes/spendguard/` | container starts, `/api/v1/ping` responds 200 |
| F-02 | The wrapper node appears in `GET /api/v1/nodes` response | node loader picked it up |
| F-03 | POST a chatflow JSON containing the wrapper at `/api/v1/chatflows`; receive 200 + a chatflow id | persistence works |
| F-04 | POST a prediction to `/api/v1/prediction/<id>` with `question: "hi"` | 200; response body has chat completion shape; **sidecar logs ONE `RequestDecision` with `trigger=LLM_CALL_PRE`, `route=llm.call`** |
| F-05 | The reserve is fired BEFORE the OpenAI HTTP request (timestamps compared against the mock OpenAI server's request log) | pre-call gating proven |
| F-06 | Deny variant: budget seeded with `amountAtomic="0"`; POST prediction; receive 4xx OR a body containing `STOP`/`DecisionStopped`; mock OpenAI server gets 0 requests | deny path proven |
| F-07 | After F-04 the audit_outbox table in the demo Postgres has exactly one `LLM_CALL_PRE` row for the tenant | DB-side wiring proven |

The E2E suite uses the same demo Postgres + sidecar that the `flowise_real` demo compose uses, just driven from vitest instead of `run_demo.py`.

## 4. Fixtures

### 4.1 `tests/_fixtures/chatflow_minimal.json`

A minimal Flowise chatflow JSON used by `F-03`/`F-04`. Schema follows Flowise 2.x's `chatflowAddNodes` payload shape — three nodes (`ChatOpenAI` → `SpendGuardChatModelWrapper` → `Conversation Chain`) with edges between them. Committed verbatim; not generated.

### 4.2 `tests/_fixtures/chatflow_deny.json`

Variant of `chatflow_minimal.json` where the wrapper's `tenantId` points at the seeded "no-budget" tenant. Used by `F-06`.

## 5. Mock OpenAI server

The E2E suite spins up a tiny Node HTTP server (port 0, random) that responds to `/v1/chat/completions` with a canned chat completion. The Flowise container's `OPENAI_API_HOST` env is set to the server URL. The server records every request with a timestamp; tests assert against the recorded sequence.

This mirrors D04's `tests/e2e/chatOpenAI.test.ts` pattern — slice 4 reuses the same mock-server helper from `sdk/typescript/integrations/langchain/tests/_support/mockOpenAi.ts`.

## 6. CI matrix

| Matrix shard | Node | Flowise | Skip when |
|---|---|---|---|
| ts-flowise-unit | 20.10, 22 LTS | n/a | never |
| ts-flowise-e2e | 22 LTS | 2.x | `D35_E2E` env unset OR docker socket missing |

The E2E shard is allowed to take up to 6 minutes (Flowise's first-boot is ~30 s, model loader ~15 s on container init).

## 7. Cross-language parity (P0)

D35 inherits D04's `idempotencyKey` derivation, so the cross-language parity test in `sdk/typescript/integrations/langchain/tests/idempotencyParity.test.ts` covers D35 transitively. D35 adds one assertion:

| # | Test | Verifies |
|---|---|---|
| P-01 | A chat call routed through `SpendGuardChatModelWrapper` produces the SAME `idempotencyKey` (for identical inputs) as a direct `SpendGuardCallbackHandler` attached to the same model | wrapper does not alter the key derivation |

The fixture is `sdk/fixtures/cross-language/flowise_v1.json` — 4 vectors covering chat with + without `parentRunId`, with + without `claimEstimatorJson` override.
