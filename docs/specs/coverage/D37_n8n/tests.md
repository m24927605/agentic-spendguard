# D37 — Tests

## 1. Test layout

```
sdk/typescript/integrations/n8n/tests/
├── node.test.ts            # SpendGuardChatModel.supplyData behaviour (mock n8n context)
├── credential.test.ts      # SpendGuardApi schema + property validation
├── clientPool.test.ts      # acquireClient singleton + FIFO eviction
├── runIdentity.test.ts     # executionId / nodeName / custom run-id derivation
├── errors.test.ts          # SpendGuardError → NodeApiError mapping
├── _support/
│   ├── mockN8nContext.ts   # ISupplyDataFunctions fake (getCredentials, getNode, getNodeParameter, etc.)
│   ├── mockUpstreamModel.ts # tiny BaseChatModel sub that fires real RunManager events
│   └── mockSidecar.ts      # re-exports the mock from sdk/typescript/tests/_support
└── e2e/
    └── selfHostedN8n.test.ts # docker-compose self-hosted n8n + mock sidecar
```

## 2. Coverage targets

| Module | Statement / branch target |
|---|---|
| `nodes/SpendGuardChatModel/SpendGuardChatModel.node.ts` | ≥ 90 % stmt, ≥ 85 % branch |
| `credentials/SpendGuardApi.credentials.ts` | n/a (declarative schema; covered by `credential.test.ts` parity check) |
| `src/clientPool.ts` | 100 % stmt + branch |
| `src/runIdentity.ts` | 100 % stmt + branch |
| `src/errors.ts` | 100 % stmt + branch |

Overall package floor: **≥ 90 % statements, ≥ 85 % branches**.

## 3. Tests by module

### 3.1 `node.test.ts` — `SpendGuardChatModel.supplyData`

| # | Test | Verifies |
|---|---|---|
| N-01 | `supplyData` returns `{ response: <upstream> }` when an upstream model is connected | wiring contract |
| N-02 | `supplyData` throws when `getInputConnectionData(AiLanguageModel, 0)` returns `undefined` | guard |
| N-03 | `supplyData` calls `getCredentials("spendGuardApi")` exactly once | credential lookup |
| N-04 | A `SpendGuardCallbackHandler` is pushed onto `upstream.callbacks` (array-shape) | injection contract |
| N-05 | A second `supplyData` call on the SAME node instance does NOT double-register the handler | idempotent injection |
| N-06 | `upstream.callbacks` was `undefined` → becomes an array of length 1 | defensive init |
| N-07 | `upstream.callbacks` was a single handler (object, not array) → normalised to array of length 2 | LangChain accepts both shapes |
| N-08 | Handler receives `budgetId` from credential when `budgetIdOverride` is empty | param resolution |
| N-09 | Handler receives `budgetIdOverride` when set | override wins |
| N-10 | Handler receives `route = "llm.call"` by default | default |
| N-11 | Handler receives the consumer-provided `route` when overridden | param resolution |
| N-12 | `claimEstimator` callback returns a single `BudgetClaim` with the configured `amountAtomic` + `unit` | claim shape |
| N-13 | `claimEstimator` invocation reuses the resolved `budgetId` as `scopeId` | budget pinning |
| N-14 | `acquireClient` invoked once per credential per process; not once per `supplyData` call | singleton contract |
| N-15 | `mapToNodeApiError` is invoked when `acquireClient` throws | error path coverage |
| N-16 | Returned model retains its original prototype chain (`response instanceof BaseChatModel`) | no-proxy guarantee |

### 3.2 `credential.test.ts` — `SpendGuardApi`

| # | Test | Verifies |
|---|---|---|
| C-01 | `name === "spendGuardApi"` | n8n credential ID lock |
| C-02 | `displayName === "SpendGuard API"` | UI string |
| C-03 | `documentationUrl` resolves to the docs site integrations page | doc link present |
| C-04 | `properties` contains `tenantId`, `socketPath`, `budgetId`, `windowInstanceId`, `runtimeKind` in that order | schema order |
| C-05 | `tenantId` and `budgetId` and `windowInstanceId` are `required: true` | required-field invariant |
| C-06 | `socketPath` default is `/var/run/spendguard/sidecar.sock` | demo-compat default |
| C-07 | `runtimeKind` default is `"n8n"` | telemetry attribution |
| C-08 | No `test` function exported (handshake is lazy) | scope check |

### 3.3 `clientPool.test.ts` — `acquireClient`

| # | Test | Verifies |
|---|---|---|
| CP-01 | First `acquireClient(creds)` constructs a `SpendGuardClient`, calls `connect` then `handshake`, returns it | happy path |
| CP-02 | Second `acquireClient(creds)` with same `tenantId + socketPath` returns the SAME instance | singleton |
| CP-03 | `acquireClient(creds')` with different `tenantId` returns a DIFFERENT instance | per-credential keyed |
| CP-04 | 17th unique credential triggers FIFO eviction; oldest client's `close()` is called | bounded pool |
| CP-05 | Failed handshake → pool entry is deleted; next call retries from scratch | error recovery |
| CP-06 | Concurrent first-call requests (same creds) share a single in-flight promise — only one `connect` runs | dedup |
| CP-07 | `beforeExit` handler calls `close()` on every cached client | clean shutdown |
| CP-08 | `key()` is identical for `tenantId: "A", socketPath: "/s"` regardless of property iteration order on the input object | deterministic key |
| CP-09 | Two creds differing only in `runtimeKind` share a client (key only uses `tenantId + socketPath`) | key scope |

### 3.4 `runIdentity.test.ts` — `resolveRunIdentity`

| # | Test | Verifies |
|---|---|---|
| RI-01 | `runIdSource = "executionId"` → `runId = "${executionId}:${nodeName}"`, `sessionId = executionId`, `stepId = nodeName` | default mode |
| RI-02 | `runIdSource = "nodeName"` → `runId = nodeName`, sessionId + stepId unchanged | alt mode |
| RI-03 | `runIdSource = "custom"` with non-empty `customRunId` → `runId = customRunId` | override |
| RI-04 | `runIdSource = "custom"` with empty `customRunId` → falls back to `executionId:nodeName` | fallback |
| RI-05 | `sessionId` always equals `executionId` regardless of `runIdSource` | session invariant |
| RI-06 | `stepId` always equals `nodeName` regardless of `runIdSource` | step invariant |
| RI-07 | n8n-style `executionId` UUIDs are forwarded byte-identical (no trimming) | identity preservation |

### 3.5 `errors.test.ts` — `mapToNodeApiError`

| # | Test | Verifies |
|---|---|---|
| ER-01 | `DecisionStopped` → `NodeApiError(httpCode: "403")` with reason codes joined in message | deny mapping |
| ER-02 | `DecisionDenied` → `NodeApiError(httpCode: "403")` | denied mapping |
| ER-03 | `DecisionSkipped` → `NodeApiError(httpCode: "403")` | skipped mapping |
| ER-04 | `ApprovalRequired` → `NodeApiError(httpCode: "428")` with `approvalRequestId` in description | approval mapping |
| ER-05 | `SidecarUnavailable` → `NodeApiError(httpCode: "503")` | unavailable mapping |
| ER-06 | `HandshakeError` → `NodeApiError(httpCode: "502")` | handshake mapping |
| ER-07 | Generic `Error` → `NodeApiError(node, err)` passthrough | fallback |
| ER-08 | `null` / `undefined` input → `NodeApiError` constructed without throwing | defensive |
| ER-09 | `reasonCodes` empty array → message reads `"SpendGuard denied: decision_denied"` (default reason) | empty-array path |
| ER-10 | `auditDecisionEventId` undefined → description reads `"Audit event: (pending)"` | optional-field formatting |

### 3.6 `e2e/selfHostedN8n.test.ts`

This is a Vitest test that drives a docker-compose self-hosted n8n + a mock sidecar (the same gRPC mock D05 ships in `sdk/typescript/tests/_support`). Compose spec lives in `tests/e2e/compose.yml`.

| # | Test | Verifies |
|---|---|---|
| EE-01 | Importing the `n8n_real.workflow.json` fixture into a fresh n8n instance succeeds | workflow JSON is well-formed |
| EE-02 | Triggering the workflow with budget available results in execution `status: "success"` | happy path |
| EE-03 | After EE-02: the mock sidecar received exactly one `Reserve` call with `trigger=LLM_CALL_PRE`, `route=llm.call`, and `runId = "${executionId}:AI Agent"` | wire shape |
| EE-04 | After EE-02: the mock sidecar received exactly one `CommitEstimated` call with `outcome=SUCCESS` | post-call commit |
| EE-05 | Triggering with the mock sidecar configured to deny (returns `DecisionDenied`) yields execution `status: "error"`, `data.resultData.error.httpCode === "403"` | deny path |
| EE-06 | After EE-05: zero outbound Anthropic HTTP requests were observed by the mock provider | gating proof |
| EE-07 | The SpendGuard credential is created via `POST /api/v1/credentials` and saved without error | credential API integration |
| EE-08 | Two sequential workflow runs share a single `SpendGuardClient` instance (verified by mock sidecar handshake count = 1) | singleton client crosses executions |
| EE-09 | A second workflow with the same credential reuses the same client (handshake count stays at 1) | per-credential singleton |
| EE-10 | n8n's execution log records the `NodeApiError` with `description` mentioning the SpendGuard `decisionId` | debuggability |

E2E tests run against `n8nio/n8n:1.50.1` pinned image. The mock sidecar runs as a sidecar container exposing UDS via a shared docker volume.

## 4. Run-time matrix

| Matrix shard | Floor | Pass condition |
|---|---|---|
| n8n 1.50.x | 1.50.0 (floor) | full unit + e2e suite green |
| n8n 1.55.x | 1.55.0 (mid) | full unit + e2e suite green |
| n8n latest | latest at release tag time | full unit + e2e suite green |
| Node 20.10 | floor | unit suite green |
| Node 22 LTS | matrix default | unit + e2e suite green |

n8n 1.50 is the floor because (a) it's the first release where `NodeConnectionType` is the documented public enum (earlier releases used string literals), and (b) it's the floor of the `@n8n/n8n-nodes-langchain` AI Agent's `ai_languageModel` connection wiring.

## 5. Cross-deliverable parity gates

| Gate | Pass condition |
|---|---|
| P-01 | The `runId` chosen by `resolveRunIdentity` is the value that lands on `Reserve.runId` AND on the `idempotencyKey` derivation — round-trip verified in EE-03 | identity end-to-end |
| P-02 | `deriveIdempotencyKey({tenantId, sessionId, runId, stepId, llmCallId: runId, trigger: "LLM_CALL_PRE"})` byte-equals the value the mock sidecar receives | D05 invariant holds when D37 supplies the inputs |
| P-03 | If D04 v0.1.x exposes `runIdOverride / sessionIdOverride / stepId`, D37 uses them; otherwise D37 fails CI at typecheck — forces D04 to ship the additive | spec coordination |

## 6. Anti-tests (must NOT exist)

| # | Anti-test | Rationale |
|---|---|---|
| AT-01 | Test that calls a real Anthropic / OpenAI API key | no live provider calls in CI; that's demo-mode territory |
| AT-02 | Test that requires `process.env.SPENDGUARD_TENANT_ID` to be set | unit tests pass credentials explicitly |
| AT-03 | Test that monkey-patches `BaseChatModel.prototype` | global mutation forbidden; per-instance `callbacks.push` only |
| AT-04 | Test that asserts the n8n UI renders a specific DOM | DOM-level UI tests are out of scope; n8n eslint handles UI lint |
| AT-05 | Test that reaches across `@spendguard/sdk` internals (e.g. asserts proto field names) | substrate's contract; tested in D05's suite |

## 7. Fixture inventory

| Fixture | Path | Purpose |
|---|---|---|
| `n8n_real.workflow.json` | `examples/n8n/workflows/` | EE-01..EE-06 use this as the imported workflow |
| `cross_lang/n8n_v1.json` | `sdk/fixtures/cross-language/` | (Optional, S-tier) ≥ 8 vectors of `(executionId, nodeName, tenantId) → idempotencyKey` for parity vs Python — added in slice 4 |
| `compose.yml` | `tests/e2e/` | docker-compose spec for self-hosted n8n + mock sidecar |
| `n8n_init.json` | `tests/e2e/` | n8n owner-setup payload for headless first-boot |

## 8. CI shape

- Unit tests run on every push (`pnpm run test`).
- E2E tests run only on the release branch + PR-to-main + nightly schedule (compose spin-up is ~90 s; not gating every PR).
- E2E shard is allowed to flake retry once; persistent flake escalates per `feedback_hardening_workflow`.
- Coverage report uploaded to repo's existing coverage artifact path.

## 9. Notes for the reviewer

1. The handler-injection path is the highest-value test surface — six tests (N-04..N-09) verify it because it's where adapter bugs hide.
2. `clientPool` is the only piece of D37 with shared state. Every test in §3.3 resets the module via `vi.resetModules()` to avoid cross-test bleed.
3. E2E tests are pinned to a specific n8n image SHA; bumping the SHA requires a PR review (per `feedback_hardening_workflow` reproducibility rule).
4. The deny-path test (EE-05/EE-06) is the contract acceptance gate; if it ever fails silently (workflow executes despite deny), the entire deliverable is broken.
