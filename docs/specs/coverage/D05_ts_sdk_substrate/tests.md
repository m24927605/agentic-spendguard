# D05 — Tests

## 1. Test layout

```
sdk/typescript/tests/
├── client.test.ts                 # SpendGuardClient lifecycle + RPCs (vs mock sidecar)
├── errors.test.ts                 # Exception hierarchy, instanceof checks, statusCode
├── ids.test.ts                    # newUuid7 shape, deriveIdempotencyKey determinism
├── promptHash.test.ts             # Local determinism + tenant canonicalization
├── pricing.test.ts                # USD-micros math, round-up semantics, demo snapshot
├── runPlan.test.ts                # AsyncLocalStorage propagation, nested wins
├── retry.test.ts                  # Bounded retry, idempotency-key guard
├── decisionCache.test.ts          # LRU eviction, TTL, key isolation
├── crossLanguage.test.ts          # Byte-for-byte parity vs Python + Rust
├── treeShaking.test.ts            # Subpath imports do not pull grpc
├── env.test.ts                    # Env-var resolution precedence
├── otel.test.ts                   # onSpan callback fires, mutual-exclusion error
├── _support/
│   ├── mockSidecar.ts             # @grpc/grpc-js server over ephemeral UDS
│   ├── fixtures.ts                # Reusable BudgetClaim / DecisionResponse builders
│   └── crossLanguageFixtures.ts   # Loads sdk/fixtures/cross-language/*.json
└── e2e/
    └── reserveCommitRelease.test.ts  # Full reserve → commit → release happy path
```

## 2. Unit coverage targets

| Module | Branch coverage target | Notes |
|---|---|---|
| `errors.ts` | 100 % | Tiny module, all branches simple. |
| `ids.ts` | 95 % | Skip the unreachable `randomBytes` failure branch. |
| `promptHash.ts` | 100 % | Tenant canonicalisation + trim + HMAC must be exhaustive. |
| `pricing.ts` | 95 % | One floating-point edge case branch is acceptable to skip. |
| `runPlan.ts` | 100 % | Nested + sync + async all exercised. |
| `retry.ts` | 90 % | Skip the unreachable `Math.random < 0` branch. |
| `decisionCache.ts` | 90 % | LRU eviction order, TTL expiry. |
| `client.ts` | 80 % | RPC error paths covered; some `disabled` no-op branches deferred to integration. |
| `otel.ts` | 80 % | Run with + without `@opentelemetry/api` peer dep installed. |

Overall package floor: **≥ 85 % statements, ≥ 80 % branches**.

## 3. Tests by module

### 3.1 `client.test.ts`

| # | Test | Verifies |
|---|---|---|
| C-01 | constructor rejects missing `socketPath` and `SPENDGUARD_SIDECAR_UDS` | `SpendGuardConfigError` with clear message. |
| C-02 | constructor rejects missing `tenantId` | `SpendGuardConfigError`. |
| C-03 | constructor rejects `otelTracer` + `onSpan` both set | mutually-exclusive `SpendGuardConfigError`. |
| C-04 | `disabled: true` short-circuits `reserve` / `commitEstimated` / `release` | no UDS contact, returns no-op outcomes. |
| C-05 | `handshake()` is idempotent under concurrent callers | only one Handshake RPC issued; both callers see same outcome. |
| C-06 | `handshake()` capability-mismatch raises `HandshakeError` | server returns `capability_required > capability_level`. |
| C-07 | `handshake()` protocol-version mismatch raises `HandshakeError` | server returns `protocolVersion=2`, adapter sent 1. |
| C-08 | `reserve()` on STOP raises `DecisionStopped` | reason codes propagated, audit decision event id captured. |
| C-09 | `reserve()` on STOP_RUN_PROJECTION raises `DecisionStopped` | run_code_triggered preserved. |
| C-10 | `reserve()` on SKIP raises `DecisionSkipped` | tagged subclass. |
| C-11 | `reserve()` on REQUIRE_APPROVAL raises `ApprovalRequired` | carries `tenantId` for `resume()` round-trip. |
| C-12 | `reserve()` CONTINUE returns `DecisionOutcome` | reservation IDs as readonly tuple, `ttlExpiresAtSeconds` non-zero. |
| C-13 | `reserve()` DEGRADE returns `DecisionOutcome` with `mutationPatchJson` | json string non-empty. |
| C-14 | `reserve()` with `promptText` populates `runtime_metadata.prompt_hash` | hash matches `computePromptHash(text, tenant)`. |
| C-15 | `reserve()` with `decisionContextJson` folds into runtime_metadata | prompt_hash wins on collision. |
| C-16 | `reserve()` repeated `idempotencyKey` hits in-process cache | second call does not contact server. |
| C-17 | `reserve()` inside `withRunPlan` sends `plannedStepsHint` | `plannedCalls + plannedTools` on wire. |
| C-18 | `reserve()` outside any plan sends `plannedStepsHint=0` | default. |
| C-19 | `release()` happy path | `ReleaseOutcome.auditEventSignature` non-empty. |
| C-20 | `release()` idempotent same key returns same outcome | server emits one `audit.release` per `(reservation_id, idempotency_key)`. |
| C-21 | `release()` fencing/state conflict surfaces as `SpendGuardError` with gRPC status detail | maps `FailedPrecondition` cluster. |
| C-22 | `commitEstimated()` SUCCESS path | one trace event sent + one ack consumed. |
| C-23 | `commitEstimated()` server returns REJECTED ack → throws `SpendGuardError` | ack status surfaced. |
| C-24 | `confirmPublishOutcome()` `APPLY_FAILED` records audit | server-returned event id non-empty. |
| C-25 | `safeConfirmApplyFailed()` swallows transport error | original exception path preserved. |
| C-26 | `resumeAfterApproval()` returns `DecisionOutcome` on `approved` | typed outcome. |
| C-27 | `resumeAfterApproval()` raises `ApprovalDeniedError` on `denied` | approver subject/reason carried. |
| C-28 | `resumeAfterApproval()` raises `ApprovalLapsedError` on non-actionable state | state parsed from `[APPROVAL_NON_TERMINAL]` tag. |
| C-29 | `resumeAfterApproval()` raises `ApprovalBundleHotReloadedError` on bundle rotation | both hashes parsed from `[BUNDLE_HOT_RELOADED]` message. |
| C-30 | `queryBudget()` throws "not yet wired" in v0.1.0 | matches the docstring. |
| C-31 | `[Symbol.asyncDispose]` closes UDS channel | second close is a no-op. |
| C-32 | `tenantId` getter is stable after handshake | matches constructor. |
| C-33 | `sessionId` throws before handshake | clear `HandshakeError`. |
| C-34 | `unix:` URI uses `grpc.default_authority=localhost` | channel option asserted via mock transport. |
| C-35 | Connection retry on `UNAVAILABLE` succeeds within 2 attempts | bounded retry verified. |
| C-36 | Bare `UNAVAILABLE` without idempotency key does NOT retry | safety guard verified. |

### 3.2 `errors.test.ts`

| # | Test | Verifies |
|---|---|---|
| E-01 | All error classes extend `SpendGuardError` and `Error` | `instanceof` chain. |
| E-02 | `SidecarUnavailable.statusCode === 503` | const literal. |
| E-03 | `DecisionDenied.statusCode === 403` | const literal. |
| E-04 | `ApprovalRequired.resume(client)` delegates to `client.resumeAfterApproval` with right args | spy. |
| E-05 | `ApprovalLapsedError` reason_codes includes `approval_lapsed_<state>` | per spec. |
| E-06 | Error subclasses preserve `name` after `JSON.stringify` | name field is enumerable. |
| E-07 | `SidecarUnavailable` `cause` is forwarded when constructed with one | for error chaining. |
| E-08 | `MutationApplyFailed` and `SpendGuardConfigError` extend `SpendGuardError` | adapter routing works. |

### 3.3 `ids.test.ts`

| # | Test | Verifies |
|---|---|---|
| I-01 | `newUuid7()` matches RFC 9562 §5.7 (version=7, variant=10) | byte 6 high nibble = 7, byte 8 high bits = 10. |
| I-02 | `newUuid7()` is monotonic-ish in fast succession | timestamp prefix non-decreasing. |
| I-03 | `newUuid7()` is parseable as UUID | round-trip through `crypto.randomUUID` regex. |
| I-04 | `deriveIdempotencyKey()` is deterministic | same args twice → same key. |
| I-05 | `deriveIdempotencyKey()` `tenant` rotation changes key | sanity. |
| I-06 | `deriveIdempotencyKey()` `trigger` rotation changes key | sanity. |
| I-07 | `deriveIdempotencyKey()` output shape | `sg-` + 32 hex chars exactly. |
| I-08 | `deriveUuidFromSignature("hello", { scope: "decision_id" })` matches a fixed vector | golden value. |
| I-09 | `defaultCallSignature([], undefined)` is a 32-hex string | shape gate. |
| I-10 | `defaultCallSignature` stable across runs with identical inputs | determinism. |
| I-11 | `workloadInstanceId()` honors `SPENDGUARD_WORKLOAD_INSTANCE_ID` env var | env stub. |

### 3.4 `promptHash.test.ts`

| # | Test | Verifies |
|---|---|---|
| P-01 | `computePromptHash("hello world", "tenant-a")` matches fixed vector | golden. |
| P-02 | `computePromptHash(" hello ", "t")` trims ASCII whitespace | leading + trailing. |
| P-03 | `computePromptHash("hello\n", "t")` trims `\n`, `\t`, `\f`, `\r` | full set. |
| P-04 | Same prompt + different tenants → different hashes | tenant-keyed. |
| P-05 | UUID-shaped tenant is canonicalised to lowercase | "AAAA…" == "aaaa…". |
| P-06 | Non-UUID tenant uses raw string verbatim | per spec. |
| P-07 | Output is 64-char lowercase hex | regex gate. |

### 3.5 `pricing.test.ts`

| # | Test | Verifies |
|---|---|---|
| PR-01 | `usdMicrosForCall` rounds up to nearest µUSD | never undercharge. |
| PR-02 | Missing-kind falls back to default ("output") | per spec. |
| PR-03 | Zero / negative token counts skip | no negative charges. |
| PR-04 | `pricePerMillion` returns `null` when unknown | not 0. |
| PR-05 | `DEMO_PRICING` exposes ≥ 12 (provider, model, kind) keys | snapshot health gate. |
| PR-06 | `DEMO_PRICING_VERSION` matches `seed.yaml`'s `pricing_version` field at build time | scripts/embedPricingSnapshot.ts. |

### 3.6 `runPlan.test.ts`

| # | Test | Verifies |
|---|---|---|
| R-01 | `currentRunPlan()` returns `null` outside `withRunPlan` | default. |
| R-02 | `withRunPlan({plannedCalls: 8}, fn)` propagates plan into async fn | AsyncLocalStorage. |
| R-03 | Nested `withRunPlan` — outer wins | per spec. |
| R-04 | Plan visible across `await` and `Promise.all` | ALS context. |
| R-05 | `plannedCalls` non-int / negative → `TypeError` at decorator time | input validation. |
| R-06 | `plannedTools` non-int / negative → `TypeError` at decorator time | symmetric. |
| R-07 | `fn` non-callable raises | clear error. |
| R-08 | `plannedSteps` getter on `RunPlan` = `plannedCalls + plannedTools` | sanity. |

### 3.7 `retry.test.ts`

| # | Test | Verifies |
|---|---|---|
| RT-01 | First-try success → no backoff | fast path. |
| RT-02 | `UNAVAILABLE` once → succeeds on second attempt | 1 retry. |
| RT-03 | `UNAVAILABLE` twice → throws `SidecarUnavailable` | exhausted. |
| RT-04 | `DEADLINE_EXCEEDED` → `SidecarUnavailable` | classification. |
| RT-05 | `INVALID_ARGUMENT` not retried | non-retriable code. |
| RT-06 | Backoff time in [25, 50] ms | jitter clamp. |
| RT-07 | Request without `idempotencyKey` does NOT retry on `UNAVAILABLE` | safety. |
| RT-08 | Error has `cause` linking to original | for adapter routing. |

### 3.8 `decisionCache.test.ts`

| # | Test | Verifies |
|---|---|---|
| DC-01 | Get-set round-trip | basic. |
| DC-02 | LRU eviction beyond `maxEntries` | per spec. |
| DC-03 | TTL expiry returns `undefined` after window | per spec. |
| DC-04 | Empty key never matches | safety. |
| DC-05 | Different tenants → different cache namespaces | isolation (cache is per-client). |

### 3.9 `crossLanguage.test.ts` — P0 invariant

This is the critical gate. The cross-language fixture file lives at `sdk/fixtures/cross-language/v1.json` (created by slice S05_09) and is shared by Python (`sdk/python/tests/cross_language_test.py`) + Rust (`services/sidecar/tests/cross_language_test.rs`).

Each fixture entry has the shape:

```json
{
  "computePromptHash": [
    { "promptText": "hello", "tenantId": "tenant-a", "expectedHex": "..." },
    { "promptText": "  ", "tenantId": "01900000-...-...-...", "expectedHex": "..." }
  ],
  "deriveIdempotencyKey": [
    { "args": { "tenantId": "...", "sessionId": "...", "runId": "...",
                "stepId": "...", "llmCallId": "...", "trigger": "LLM_CALL_PRE" },
      "expected": "sg-<32hex>" }
  ],
  "defaultCallSignature": [
    { "messages": [...canonicalised...], "modelSettings": {...}, "expectedHex": "..." }
  ]
}
```

| # | Test | Verifies |
|---|---|---|
| CL-01 | Every `computePromptHash` vector in `v1.json` matches | exhaustive sweep. |
| CL-02 | Every `deriveIdempotencyKey` vector matches | exhaustive sweep. |
| CL-03 | Every `defaultCallSignature` vector matches | exhaustive sweep. |
| CL-04 | Tenant-canonicalisation vectors match (mixed case UUID → identical hash) | gate. |
| CL-05 | Empty-string prompt + valid tenant → fixed-vector match | gate (handles "" specially per `client.py` r1 P3). |
| CL-06 | Test fails loudly with a diff if any vector drifts | not silently passing on hash mismatch. |

Volume requirement: **≥ 64 vectors** across all three functions. The fixture is committed; the Python + Rust suites already consume it. Slice S05_09 adds the TS suite.

### 3.10 `treeShaking.test.ts`

| # | Test | Verifies |
|---|---|---|
| TS-01 | `import { newUuid7 } from "@spendguard/sdk/ids"` esbuild metadata shows NO `@grpc/grpc-js` dep pulled | tree-shaking gate. |
| TS-02 | `import { PricingLookup } from "@spendguard/sdk/pricing"` excludes proto tree | gate. |
| TS-03 | `import { computePromptHash } from "@spendguard/sdk/prompt-hash"` is < 10 KB bundled | bundle-size gate. |
| TS-04 | `import {} from "@spendguard/sdk"` (full surface) is < 120 KB minified, < 35 KB gzipped | hard budget. |

The test uses esbuild's `metafile` output to inspect dependency graph.

### 3.11 `env.test.ts`

| # | Test | Verifies |
|---|---|---|
| EN-01 | Explicit `socketPath` overrides `SPENDGUARD_SIDECAR_UDS` | precedence. |
| EN-02 | Env `SPENDGUARD_TENANT_ID` honored when `tenantId` omitted | fallback. |
| EN-03 | `SPENDGUARD_DECISION_TIMEOUT_MS` parsed as integer; non-integer rejected | safety. |
| EN-04 | `SPENDGUARD_DISABLE=1` or `=true` sets `disabled` | per spec; documented as test-only. |
| EN-05 | Missing both explicit + env → `SpendGuardConfigError` at constructor | per spec. |

### 3.12 `otel.test.ts`

| # | Test | Verifies |
|---|---|---|
| O-01 | `onSpan` callback receives one record per RPC | call count + name. |
| O-02 | `onSpan` records `error` on failed RPC | exception forwarded. |
| O-03 | `otelTracer.startActiveSpan` called with correct name + attributes | OTel parity. |
| O-04 | Both `onSpan` + `otelTracer` set → constructor throws `SpendGuardConfigError` | mutual exclusion. |
| O-05 | Neither set → zero-cost (no span constructed) | budget gate. |
| O-06 | `@opentelemetry/api` not installed → import still works | peerDep optional. |

## 4. Integration / e2e

### 4.1 `e2e/reserveCommitRelease.test.ts`

Runs a full lifecycle against `_support/mockSidecar.ts`:

1. `await client.handshake()` — verifies session id is non-empty.
2. `await client.reserve({...})` with a fake budget claim and `idempotencyKey` derived via `deriveIdempotencyKey(...)`.
3. Assert `DecisionOutcome.decision === "CONTINUE"` and `reservationIds.length === 1`.
4. `await client.commitEstimated({...})` — assert no throw.
5. `await client.release({reservationId, idempotencyKey: deriveIdempotencyKey(...)})` — assert `releasedReservationIds.length === 1`.
6. `await client.close()`.

Repeat the same flow with `disabled: true`: every method should succeed with no-op outcomes and zero UDS bytes (mock sidecar accept counter is 0).

### 4.2 Mock sidecar implementation notes

The mock sidecar is a real `@grpc/grpc-js` server that binds an ephemeral UDS path (`/tmp/spendguard-test-<random>.sock`). It implements every RPC the v0.1.0 client uses with deterministic happy paths. Each test that needs different behavior (STOP, SKIP, …) constructs the server with a custom `behaviour` config.

- **No external deps.** The mock lives inside `tests/_support/` and uses the same `_proto` codegen as the SUT. Slice S05_09 ships the mock.
- **Per-test isolation.** `pool: "forks"` in `vitest.config.ts` plus a unique socket path per test ensures parallel runs don't collide.
- **Cleanup.** `afterEach` closes the server and `fs.rm`'s the socket file. Failure to clean leaks /tmp entries; the cleanup hook is asserted.

## 5. Coverage report invariant

`pnpm run test --coverage` must produce:

- Statements ≥ 85 %
- Branches ≥ 80 %
- Functions ≥ 85 %
- Lines ≥ 85 %

CI gates the publish workflow on these floors. Below the floor → workflow fails.

## 6. Runtime matrix tests

| Runtime | Tests | When |
|---|---|---|
| Node 20.10 (minimum) | full suite | PR + main |
| Node 22 LTS (primary) | full suite + e2e | PR + main |
| Bun 1.1+ | unit (skip e2e — `@grpc/grpc-js` UDS under Bun has rough edges; smoke test only) | nightly |
| Deno 1.46+ | unit subset (no `@grpc/grpc-js` — only pure-TS modules: ids, errors, pricing, promptHash, runPlan) | nightly |

Bun + Deno failures are advisory in v0.1.0; promoted to blocking in v0.2.

## 7. Verification commands (operator quick-reference)

```bash
cd sdk/typescript

# Codegen + lint + typecheck + test in one shot.
pnpm install --frozen-lockfile
pnpm run proto
pnpm run lint
pnpm run typecheck
pnpm run test

# Just the cross-language gate (P0 invariant).
pnpm run test tests/crossLanguage.test.ts

# Bundle size budget.
pnpm run build
pnpm run size
```

All commands return exit code 0 on success. Anything red → R1 fails, reviewer remediates per `review-standards.md`.

## 8. Regression / demo gate

The substrate is consumed by `make demo` only transitively (the Python SDK still drives the canonical demo). To prove the TS substrate works end-to-end, slice S05_09 adds a Makefile target:

```make
demo-ts-substrate: ## End-to-end TS substrate against real sidecar
	cd sdk/typescript && pnpm install --frozen-lockfile && pnpm run proto && pnpm run build
	cd sdk/typescript && node tests/e2e/reserveCommitRelease.test.ts \
		--socket-path=/tmp/spendguard/sidecar.sock \
		--tenant-id=$(SPENDGUARD_DEMO_TENANT_ID)
```

This target runs against the same `docker-compose up -d` topology used for the Python demo. Adding the target is the visible evidence that the substrate is wired to a real sidecar, not just to its own mock.
