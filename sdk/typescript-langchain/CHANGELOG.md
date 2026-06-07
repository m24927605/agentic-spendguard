# `@spendguard/langchain` Changelog

All notable changes to the LangChain.js adapter for the SpendGuard SDK.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This package adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

The Python sibling — [`spendguard-sdk[langchain]`](https://pypi.org/project/spendguard-sdk/)
on PyPI, currently v0.5.1 — and this TypeScript adapter are kept in lockstep
on the callback semantics. See
[`docs/specs/coverage/D04_langchain_ts/design.md`](../../docs/specs/coverage/D04_langchain_ts/design.md)
for the cross-language behaviour parity matrix.

---

## [0.1.0-pre] — 2026-06-07

First public release. TS counterpart of `spendguard-sdk[langchain]` (Python)
v0.5.1; callback-handler shape (LangChain.js prefers callback handlers; the
Python wrapper is a Python idiom). Closes deliverable D04 (LangChain TS
adapter).

### Added

#### Handler (SLICE 2-3)

- `SpendGuardCallbackHandler` — `BaseCallbackHandler` subclass; drop-in
  via `callbacks: [handler]` on any `BaseChatModel` / `BaseLLM`.
  - `static lc_name()` returns `"SpendGuardCallbackHandler"` for
    `@langchain/core`'s serialization registry.
  - `raiseError = true` + `awaitHandlers = true` pinned on the instance —
    throw propagates through `CallbackManager` to halt `model.invoke()`
    (review-standards §1.3 P0 LOCK).
- `SpendGuardCallbackHandlerOptions` — LOCKED v0.1.0 options surface:
  `client`, `tenantId?`, `defaultBudgetMicrosCap?`, `budgetId?`. Additive
  optional fields will land in future minors per the design.md §4
  superset (`unitId` / `windowInstanceId` / `unit` / `pricing` /
  `claimEstimator` / `route` / `callSignatureFn` / `claimEstimate` /
  `onApprovalRequired`) when the substrate broadens `UnitRef` —
  see **Known limitations** below.

#### Hooks (SLICE 3)

- `handleChatModelStart` — derives the canonical idempotency key from
  `(tenantId, runId, parentRunId)` via the substrate's
  `deriveIdempotencyKey`, builds a `ReserveRequest` with
  `trigger="LLM_CALL_PRE"`, projects a coarse claim from the chat messages,
  and dispatches `client.reserve(...)`. On `DecisionDenied` (or subclasses
  `DecisionStopped` / `ApprovalRequired`) the error rethrows so the
  LangChain `RunManager` halts the run BEFORE the provider HTTP call. On
  `SidecarUnavailable` (or any other substrate error) the handler logs +
  swallows so a sidecar outage does NOT block the LLM call — "operational
  degradation, not enforcement" (design.md §3.6).
- `handleLLMStart` — symmetrically gates `BaseLLM` (completions-shape)
  invocations against the same reserve path.
- `handleLLMEnd` — reads + deletes the inflight entry keyed by `runId`,
  extracts `(promptTokens, completionTokens)` from `output.llmOutput.tokenUsage`
  (camelCase OR `token_usage` snake_case — accepts both LangChain-canonical
  AND OpenAI-passthrough shapes), and emits a SUCCESS commit via
  `client.commitEstimated(...)` with `outcomeKind="SUCCESS"`.
- `handleLLMError` — emits a `PROVIDER_ERROR` / FAILURE commit with the
  error's `.message` threaded onto `actualErrorMessage`.

#### IDs (SLICE 3)

- `deriveIdempotencyKey` — adapter-side helper that maps LangChain's
  `(runId, parentRunId)` shape onto the substrate's canonical
  `(tenantId, sessionId, runId, stepId, llmCallId, trigger)` key. The
  same `(tenantId, runId, parentRunId)` triple — invoked from any number
  of retry attempts within a single LangChain run — produces the same
  key. Cross-language byte-equivalent with the Python SDK's
  `derive_idempotency_key()`.

#### Errors

- Re-exports `SpendGuardError`, `DecisionDenied`, `SidecarUnavailable`
  directly from `@spendguard/sdk` (preserves class identity so
  `err instanceof DecisionDenied` works across the adapter ↔ substrate
  boundary). Other substrate errors (`DecisionStopped`,
  `ApprovalRequired`, `MutationApplyFailed`, …) remain importable from
  `@spendguard/sdk` directly.

#### Tests (SLICE 4)

- Mock-sidecar test suite covering: reserve / commit happy path, throw
  propagation under `raiseError = true`, deny → 0 provider fetches,
  unknown `runId` no-op on POST, token-usage extraction parity for
  camelCase + snake_case + missing shapes.
- Cross-language fixture parity: `deriveIdempotencyKey` output for the
  same `(tenantId, runId, parentRunId)` triple is byte-identical to the
  Python adapter's `derive_idempotency_key(...)` against the shared
  fixture corpus.

#### Demo (SLICE 5)

- `examples/langchain-ts/` — runnable Node demo. Drives `ChatOpenAI`
  against the in-network counting-stub upstream with a 3-step
  ALLOW + DENY + STREAM matrix. `make demo-up DEMO_MODE=langchain_ts`
  exits 0 with the success line
  `[demo] langchain_ts ALL 3 steps PASS (ALLOW + DENY + STREAM)`.

#### Documentation + publish pipeline (SLICE 6)

- Standalone docs page at
  [`/docs/integrations/langchain-ts/`](https://agenticspendguard.dev/docs/integrations/langchain-ts/)
  on the Astro Starlight site — install / quick start / configuration /
  limitations / demo / troubleshooting.
- Root `README.md` adapter integrations table now includes the
  `@spendguard/langchain` row pointing at `examples/langchain-ts/`.
- `.github/workflows/sdk-langchain-publish.yml` — npm Trusted Publisher
  OIDC pipeline triggered on `release` events tagged `langchain-v*`.
  Provenance + `--access public` enforced at the publish call.
- `scripts/size-budget.sh` enforces a 50 KB gzipped tarball ceiling
  (larger than D05's 250 KB only by margin — the adapter is thin glue;
  the budget headroom covers the `@langchain/core` import surface).
- `scripts/version-check.sh` asserts `package.json#version` matches
  `src/version.ts#VERSION` so the wire-reported `sdk_version` matches
  what npm published.

### Locked invariants

- **Public surface is `SpendGuardCallbackHandler` +
  `SpendGuardCallbackHandlerOptions` ONLY**. `errors.ts` re-exports
  `SpendGuardError` / `DecisionDenied` / `SidecarUnavailable` for
  pattern-matching ergonomics. No `default` export — review-standards
  §1.7.
- **`raiseError = true` + `awaitHandlers = true` ARE LOAD-BEARING.**
  Throw propagation through `CallbackManager` depends on both. Do not
  override either on the handler instance.
- **`runId` is `llmCallId`** — exact equality. LangChain's
  `RunManager` UUID is the deterministic call ID across retries.
- **No re-export of `@spendguard/sdk` symbols** beyond the three
  pattern-matchable errors. Consumers import everything else
  (`SpendGuardClient`, idempotency helpers) directly.

### Compatibility

- Node 20.10+ (peer-aligned with `@spendguard/sdk`).
- `@langchain/core@>=0.3` (declared as peer; adapter pins NEITHER —
  consumer's lockfile wins).
- `@spendguard/sdk@^0.1.0` (declared as peer).

### Known limitations (deferred to future slices)

- **`UnitRef` broadening.** The TS SDK substrate's public `UnitRef` does
  not currently expose `unit_id` — `sdk/typescript/src/client.ts::mapUnitRef`
  hardcodes empty. The v0.1.0 handler options surface therefore omits the
  fuller `windowInstanceId` / `unit` / `pricing` / `claimEstimator` /
  `route` / `callSignatureFn` / `claimEstimate` / `onApprovalRequired`
  field set that `design.md` §4 anticipates. The adapter projects sensible
  defaults instead (route `"langchain-llm"`, unit `USD_MICROS`, empty
  pricing freeze, char/4-token heuristic). The next D04 hardening slice
  picks up the SDK-side broadening + adapter wire-through together.
- **No mid-stream cap on streamed responses.** `handleLLMNewToken` is
  intentionally NOT wired in v0.1.0 — PRE + POST only. Overruns land in
  the audit chain but the tokens were already emitted.
- **DEGRADE patch application surfaces as `MutationApplyFailed`.**
  Matches the Python `spendguard-sdk[langchain]` v0.5.1 stance. Built-in
  claim mutation lands in a later slice.
- **No tool-call mid-loop gating.** Each tool call is its own LangChain
  event; cross-tool budget enforcement is the contract layer's job.

### Verification

- Tests pass under vitest 2.x (Node 20).
- `npm pack` tarball ≤ 50 KB gzipped (`scripts/size-budget.sh`).
- `package.json#version` == `src/version.ts#VERSION`
  (`scripts/version-check.sh`).
- Cross-language idempotency fixture parity holds between this adapter
  and `spendguard-sdk[langchain]` (Python).

---

## Future versions

The lockstep contract with `spendguard-sdk[langchain]` (Python) v0.5.1
means the next minor (v0.2.x) will additively extend the options surface
toward the design.md §4 superset when the TS substrate broadens `UnitRef`.
Every post-0.1.0 addition is backward-compatible (new optional fields
only) so the v0.1.0 type lock holds.

[0.1.0-pre]: https://github.com/m24927605/agentic-spendguard/releases/tag/langchain-v0.1.0-pre
