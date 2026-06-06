# D08 — Tests

## 1. Test layout

```
sdk/typescript/packages/openai-agents/tests/
├── withSpendGuard.test.ts             # factory entry, ALLOW/DENY/STOP/APPROVAL paths
├── model.test.ts                      # SpendGuardAgentsModel subclass form
├── runContext.test.ts                 # AsyncLocalStorage propagation + shared key
├── signature.test.ts                  # blake2b16 determinism + Python parity
├── usage.test.ts                      # totalTokens extraction (number/string/undef)
├── defaultEstimator.test.ts           # model→baseline table + Python parity
├── crossLanguageSignature.test.ts     # consumes sdk/fixtures/cross-language/v1.json
└── _support/
    ├── mockClient.ts                  # in-process SpendGuardClient double
    ├── mockInnerModel.ts              # in-process @openai/agents Model double
    └── runScope.ts                    # helper: await withRun(runId, fn)
```

Plus the demo regression in slice S08_05:

```
deploy/demo/tests/
└── test_demo_modes.py                 # extends existing parametrized matrix
                                       # with agent_real_openai_agents_ts
```

## 2. Unit coverage targets

| Module | Branch coverage target | Notes |
|---|---|---|
| `core.ts` | 95 % | The shared bracketing logic — must exercise PRE-only, PRE+POST, retry-skip, no-reservation paths. |
| `withSpendGuard.ts` | 95 % | Factory composition — getResponse, streamResponse, close, getRetryAdvice. |
| `model.ts` | 95 % | Class form parity with factory. |
| `signature.ts` | 100 % | All input types (string, list, object, null) covered. |
| `usage.ts` | 100 % | All response.usage.totalTokens shapes. |
| `runContext.ts` | 100 % | inside/outside, nested, shared-key cross-module verification. |
| `defaultEstimator.ts` | 100 % | Known model + fallback model + claim shape. |

Overall package floor: **≥ 90 % statements, ≥ 85 % branches** (substantially higher than D05's 85/80 because the surface is small).

## 3. Tests by module

### 3.1 `withSpendGuard.test.ts`

| # | Test | Verifies |
|---|---|---|
| WS-01 | ALLOW path: `reserve` returns CONTINUE → inner called → `commitEstimated` fires | reservation id flows; estimated_amount_atomic = totalTokens; outcome="SUCCESS" |
| WS-02 | DENY path: `reserve` throws `DecisionDenied` → inner NEVER called | mockInnerModel.callCount === 0 — the load-bearing invariant |
| WS-03 | STOP path: throws `DecisionStopped` → inner NEVER called | mockInnerModel.callCount === 0 |
| WS-04 | SKIP path: throws `DecisionSkipped` → inner NEVER called, no POST | commitEstimated not invoked |
| WS-05 | APPROVAL_REQUIRED path: throws `ApprovalRequired`; caller `.resume(client)` returns CONTINUE; inner runs on retry | typed error surfaces; client.resumeAfterApproval called once |
| WS-06 | Empty `reservationIds[]` (sidecar elected single-row commit-only) | `commitEstimated` not invoked; inner result still returned |
| WS-07 | Default estimator used when `claimEstimator` omitted | claim amount = MODEL_BASELINE_TOKENS[inner.model] |
| WS-08 | Explicit `claimEstimator` wins over default | custom estimator output flows verbatim |
| WS-09 | Outside `runContext` → throws `Error` containing "active runContext" | helpful error message; inner not reached |
| WS-10 | `streamResponse` pass-through — no PRE call, no POST call | mockClient.reserve.callCount === 0 |
| WS-11 | `close()` delegates to inner.close() | mockInnerModel.closeCount === 1 |
| WS-12 | `getRetryAdvice` delegates to inner | identity of returned advice object preserved |
| WS-13 | Same `(runId, signature)` produces same `idempotencyKey` across two calls | retry-safe; verified against `deriveIdempotencyKey` |
| WS-14 | Different `runId` produces different `idempotencyKey` for same `signature` | per-run isolation |
| WS-15 | `inner` lacks `close`/`getRetryAdvice` (optional methods) | wrapper does not throw on `await inner.close?.()` |

### 3.2 `model.test.ts`

Mirrors WS-01..WS-15 against `SpendGuardAgentsModel` subclass form. Same assertions, different construction style. Validates both entry points share `core.ts` behaviour. ~15 tests.

### 3.3 `runContext.test.ts`

| # | Test | Verifies |
|---|---|---|
| RC-01 | `runContext({ runId }, fn)` makes `currentRunContext()` return `{ runId }` inside fn | basic propagation |
| RC-02 | `currentRunContext()` outside any context throws | with the documented error message |
| RC-03 | Nested `runContext` — inner wins inside, outer restored after | AsyncLocalStorage stack semantics |
| RC-04 | Visible across `await Promise.resolve()` | async hook propagation |
| RC-05 | Visible across `Promise.all([...])` | both branches see the same ctx |
| RC-06 | Visible across `setImmediate` / `process.nextTick` | full async-hook coverage |
| RC-07 | Shared-key: import `runContext` from `@spendguard/openai-agents/run-context` and from `@spendguard/openai-agents` — both reference the same storage | `Symbol.for("@spendguard/run-context/v1")` deduplication |
| RC-08 | Forward-compat: a second consumer (sketch of D04 LangChain TS) sees the same `runId` set by D08's `runContext` | cross-package shared storage works |

### 3.4 `signature.test.ts`

| # | Test | Verifies |
|---|---|---|
| SIG-01 | string input → deterministic blake2b16 hex | length === 32, lowercase |
| SIG-02 | Same `(input, sys)` produces same output across N invocations | determinism |
| SIG-03 | Different `sys` produces different signature | tenant-context isolation |
| SIG-04 | null `sys` treated as empty string | parity with Python `or ""` |
| SIG-05 | Object input (list-of-message-dict) → canonical JSON.stringify | covers Agents SDK Item[] input shape |
| SIG-06 | Empty string vs `null` input distinct | edge case |

### 3.5 `usage.test.ts`

| # | Test | Verifies |
|---|---|---|
| US-01 | `usage.totalTokens` is number → returned as-is | `42 → 42` |
| US-02 | `usage.totalTokens` is string → parsed | `"42" → 42` |
| US-03 | `usage.totalTokens` is undefined → 0 | safe fallback |
| US-04 | `usage` undefined → 0 | safe fallback |
| US-05 | `usage.totalTokens` is `NaN` / `Infinity` → 0 | finiteness guard |
| US-06 | `usage.totalTokens` is `""` → 0 | string→Number returns NaN, caught |

### 3.6 `defaultEstimator.test.ts`

| # | Test | Verifies |
|---|---|---|
| DE-01 | `gpt-4o-mini` → 500-baseline BudgetClaim | known model |
| DE-02 | `gpt-4o` → 1500-baseline BudgetClaim | known model |
| DE-03 | unknown model → 800-baseline fallback | resilience |
| DE-04 | Returned claim has expected shape: `budgetId`, `unit`, `amountAtomic`, `direction: "DEBIT"`, `windowInstanceId` | proto-aligned |
| DE-05 | Python parity: read both Python `_default_estimator.MODEL_BASELINE_TOKENS` and TS table; assert equal | cross-language drift gate |
| DE-06 | Estimator ignores `input` argument | predictability — only `model` matters |

### 3.7 `crossLanguageSignature.test.ts` (P0)

Reads `sdk/fixtures/cross-language/v1.json` `openai_agents` section. Each fixture:

```json
{ "input": "...", "system": "...",
  "expected_signature":   "<16-byte hex>",
  "expected_decision_id": "<UUID>",
  "expected_llm_call_id": "<UUID>" }
```

Tests:

| # | Test | Verifies |
|---|---|---|
| CL-01 | ≥ 32 fixture vectors loaded | minimum coverage |
| CL-02 | For every vector: `signatureOf(input, system) === expected_signature` | byte-for-byte parity with Python |
| CL-03 | For every vector: `deriveUuidFromSignature(sig, { scope: "decision_id" }) === expected_decision_id` | parity |
| CL-04 | For every vector: `deriveUuidFromSignature(sig, { scope: "llm_call_id" }) === expected_llm_call_id` | parity |
| CL-05 | A failing vector prints expected/actual diff, not `false === true` | diagnosibility |

The fixture is extended in slice S08_03; the Python suite (`sdk/python/tests/integrations/test_openai_agents.py`) consumes the same JSON.

## 4. Mock infrastructure (`tests/_support/`)

### 4.1 `mockClient.ts`

```ts
export class MockSpendGuardClient {
  readonly tenantId  = "tenant-test";
  readonly sessionId = "session-test";

  reserveCalls:  ReserveRequest[]         = [];
  commitCalls:   CommitEstimatedRequest[] = [];
  releaseCalls:  ReleaseRequest[]         = [];
  nextOutcome:   DecisionOutcome | Error  = /* configured per test */;

  async reserve(req: ReserveRequest): Promise<DecisionOutcome> {
    this.reserveCalls.push(req);
    if (this.nextOutcome instanceof Error) throw this.nextOutcome;
    return this.nextOutcome;
  }
  async commitEstimated(req: CommitEstimatedRequest) { this.commitCalls.push(req); }
  async resumeAfterApproval() { return /* configured outcome */ }
}
```

### 4.2 `mockInnerModel.ts`

```ts
export class MockInnerModel implements Model {
  readonly model = "gpt-4o-mini";
  callCount  = 0;
  closeCount = 0;
  lastArgs?: any;

  async getResponse(...args) {
    this.callCount += 1;
    this.lastArgs = args;
    return {
      responseId: "resp-mock-001",
      usage:      { totalTokens: 42, inputTokens: 30, outputTokens: 12 },
      output:     [{ type: "message", content: "mock-reply" }],
    };
  }
  streamResponse() { return /* mock async iterator */ }
  async close() { this.closeCount += 1; }
  getRetryAdvice() { return { retry: false }; }
}
```

## 5. Integration / e2e

### 5.1 `examples/openai-agents-ts-composite/demo.ts --mock`

A round-trip test that:
1. Builds a `MockSpendGuardClient` configured to ALLOW the first call, DENY the second.
2. Wraps a `MockInnerModel`.
3. Runs both paths through `withSpendGuard`.
4. Asserts: PRE 2 / inner 1 / commit 1 invariant — same invariant the Python demo asserts.

Wired as a Make target: `make demo-ts-composite-mock` runs the built `dist/demo.js --mock` and exits 0 on pass. Added to `make test-ts`.

### 5.2 `agent_real_openai_agents_ts` demo regression

`deploy/demo/tests/test_demo_modes.py` parametrize entry:

```python
@pytest.mark.parametrize("mode", [
    ..., "agent_real_openai_agents_ts",
])
def test_demo_mode(mode, demo_stack):
    proc = run_demo(mode, env={"OPENAI_API_KEY": os.environ["OPENAI_API_KEY_TEST"]})
    assert proc.returncode == 0
    assert "Runner.run OK" in proc.stdout
```

Predicated on the existing `OPENAI_API_KEY_TEST` env var the demo CI shard already exports. Falls back to skip when missing (parity with `agent_real_openai_agents`).

### 5.3 Audit-chain verification

Post-run, the test queries the demo Postgres `audit_outbox` table:

```sql
SELECT trigger, run_id, decision, outcome
  FROM audit_outbox
  WHERE run_id = :runId
  ORDER BY emitted_at ASC;
```

Asserts: exactly 2 rows (`LLM_CALL_PRE` + `LLM_CALL_POST`), both linked by the same `decision_id`, and the chain HMAC verifies. Same invariant the Python `agent_real_openai_agents` mode already asserts.

## 6. Negative tests

| # | Test | Verifies |
|---|---|---|
| NEG-01 | Calling `getResponse` without `client.handshake()` first | inner SDK error surfaces clearly; wrapper does not silently no-op |
| NEG-02 | `client` field omitted in options | TS compile error (type-level) AND runtime `TypeError` |
| NEG-03 | `inner.getResponse` throws → PRE has succeeded → POST not called | reservation is left for TTL sweep (sidecar handles); wrapper does not double-emit |
| NEG-04 | `commitEstimated` throws after inner returned | wrapper does not swallow — caller sees the commit error, has the inner response on `.cause` (added to error) |
| NEG-05 | `client.reserve` resolves with empty `reservationIds[]` | no `commitEstimated` call; inner response returned |

## 7. Linting + typecheck gates

| # | Gate | Pass condition |
|---|---|---|
| LT-01 | `pnpm run lint` | biome zero diagnostics |
| LT-02 | `pnpm run typecheck` | `tsc --noEmit` exit 0 |
| LT-03 | `pnpm run typecheck` with `@openai/agents` 0.3.0 in node_modules | typecheck passes |
| LT-04 | `pnpm run typecheck` with `@openai/agents` 0.4.x simulated (vendored declarations) | typecheck passes — peer range honoured |

## 8. Subpath export tree-shaking

| # | Test | Verifies |
|---|---|---|
| TS-01 | `import { runContext } from "@spendguard/openai-agents/run-context"` → built bundle does NOT contain `core.ts` or `signature.ts` | subpath isolation |
| TS-02 | Full surface import → minified bundle ≤ 60 KB, gzipped ≤ 18 KB | budget |
| TS-03 | `@openai/agents` is NOT in the published tarball's `dependencies` | peer-dep correctness |
