# `@spendguard/vercel-ai` Changelog

All notable changes to the Vercel AI SDK middleware for the SpendGuard SDK.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This package adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

The Python sibling — [`spendguard-sdk[pydantic-ai]`](https://pypi.org/project/spendguard-sdk/)
on PyPI, currently v0.5.1 — and this TypeScript middleware are kept in
lockstep on the reserve / commit semantics. See
[`docs/specs/coverage/D06_vercel_ai_sdk/design.md`](../../docs/specs/coverage/D06_vercel_ai_sdk/design.md)
for the cross-language behaviour parity matrix.

---

## [0.1.0] — 2026-06-07

First public release. TS-side Vercel AI SDK `LanguageModelV1Middleware`
factory; transitively covers Mastra Agents via the
`@spendguard/vercel-ai/mastra` subpath alias. Closes deliverable D06
(Vercel AI SDK middleware).

### Added

#### Package init (SLICE 1)

- `package.json` + `tsconfig.json` + `tsup.config.ts` + `vitest.config.ts`
  + `biome.json` substrate. ESM-only output, Node 20.10+ target floor,
  workspace-link against `@spendguard/sdk`.
- Public `VERSION` constant + `index.ts` barrel skeleton.

#### Factory + WeakMap stash (SLICE 2)

- `createSpendGuardMiddleware(opts): LanguageModelV1Middleware` — the
  public factory. Validates `opts.client` + `opts.tenantId` at
  construction; returns a `{ middlewareVersion: "v1", transformParams,
  wrapGenerate, wrapStream }` shape conformant with AI SDK v4's
  `LanguageModelV1Middleware`.
- `SpendGuardMiddlewareOptions` — LOCKED v0.1.0 options surface:
  `client`, `tenantId`, `budgetId?`. Additive optional fields will land
  in future minors per the design.md §4 superset (`windowInstanceId` /
  `unit` / `pricing` / `claimEstimator` / `route` / `callSignature` /
  `runIdProvider` / `providerEventIdExtractor`) when the substrate
  broadens `UnitRef` — see **Known limitations** below.
- Module-level `WeakMap<LanguageModelV1CallOptions, StashEntry>` — the
  WeakMap stash keyed by params reference. GC-safe, no manual cleanup;
  review-standards §8 P0 LOCK.

#### `transformParams` reserve wiring (SLICE 3)

- `transformParams({ params })` — derives a stable per-call
  `(runId, idempotencyKey)` pair via `deriveUuidFromSignature` +
  `deriveIdempotencyKey`, projects a coarse pre-call `BudgetClaim` from
  the flattened prompt text, and calls
  `client.reserve({ trigger: "LLM_CALL_PRE", ... })`.
  - On `DecisionDenied` (or its `DecisionStopped` / `ApprovalRequired`
    subclasses), the middleware rethrows so the AI SDK caller halts
    BEFORE `doGenerate` fires.
  - On `SidecarUnavailable` (or any other substrate error), the call
    passes through without a stash entry — "operational degradation,
    not enforcement" stance, mirrors D04 SLICE 3.

#### `wrapGenerate` commit/rollback (SLICE 4)

- `wrapGenerate({ doGenerate, params })` — non-streaming commit path.
  Looks up the stash entry, runs `doGenerate()` in try/catch:
  - SUCCESS → `client.commitEstimated({outcomeKind:"SUCCESS"})` with the
    provider-reported `(promptTokens, completionTokens)` tuple. Accepts
    BOTH AI SDK v4 canonical camelCase (`{promptTokens, completionTokens}`)
    AND OpenAI-passthrough snake_case
    (`{prompt_tokens, completion_tokens}`) shapes.
  - FAILURE → `client.commitEstimated({outcomeKind:"FAILURE"})` with the
    error's `.message` threaded onto `actualErrorMessage`, then re-throws
    the original error.
  - No stash entry → passthrough (sidecar-unavailable degraded mode).
- Commit-side failures DO NOT corrupt the result — `safeCommit` swallows
  commit RPC errors and falls back to `console.warn`. Sidecar TTL
  reconciles via the audit chain.

#### `wrapStream` TransformStream commit (SLICE 5)

- `wrapStream({ doStream, params })` — streaming commit path. Looks up
  the stash, calls `doStream()` to get `{ stream, ...rest }`, replaces
  `stream` with a `TransformStream` that:
  1. Forwards every `LanguageModelV1StreamPart` downstream unmodified.
  2. Watches each part for the terminal `finish` event and snapshots its
     `usage` payload.
  3. On `flush()` (consumer drained), emits a `SUCCESS` commit
     asynchronously.
  4. On a stream-side error or upstream throw, emits a `FAILURE` commit
     and propagates the error downstream.
  5. Single `terminal` boolean ensures exactly-once commit emission
     across the finish/error race window.

#### Provider integration matrix (SLICE 6)

- `tests/_support/mockSidecar.ts` — in-process `MockSpendGuardClient`
  implementing the `SpendGuardClient` interface surface the middleware
  touches. Per-call axes: `DecisionPlan` queue, simulated latency,
  simulated reserve/commit errors. Mirrors D04 SLICE 4 langchain mock
  shape.
- `tests/_support/mockProvider.ts` — `MockOpenAIModel` +
  `MockAnthropicModel` implementing `LanguageModelV1` with recorded
  fixtures (`OPENAI_FIXTURES` / `ANTHROPIC_FIXTURES`). Both providers
  expose their real provider-specific `provider` string + `modelId` so
  middleware tests assert per-provider routing.
- `tests/providers.test.ts` — 14 distinct tests spanning: OpenAI happy
  path / rate-limit error / denial short-circuit; Anthropic happy path /
  overloaded_error; streaming across both providers; cross-provider
  parity (commit-shape equality with different token counts); empty
  stream case (0 completion tokens); idempotency key determinism across
  calls; passthrough integrity of `rawCall` / `rawResponse` /
  `finishReason` / `warnings`.

#### Mastra integration (SLICE 7)

- `src/mastra.ts` — function-reference subpath alias. Re-exports
  `createSpendGuardMiddleware` as `createSpendGuardLanguageMiddleware`.
  Strict `===` equality with the root export (asserted at runtime in the
  demo + at unit-test time in
  [`tests/locked-surface.test.ts`](./tests/locked-surface.test.ts)).
- `package.json#exports["./mastra"]` — declares the subpath in the
  package's exports map; `tsup.config.ts` adds the `mastra` entry so the
  build emits `dist/mastra.js` + `dist/mastra.d.ts`.
- `examples/vercel-ai-mastra/` — runnable Node demo. Drives
  `generateText` + `streamText` via `wrapLanguageModel` + the SpendGuard
  middleware against an in-network counting-stub `LanguageModelV1`. 3
  steps: ALLOW + DENY + STREAM. Asserts the Mastra alias parity at boot.
- `deploy/demo/vercel_ai_mastra/{docker-compose.yaml, README.md}` —
  docker-compose overlay declaring `counting-stub` +
  `vercel-ai-mastra-runner` services. Mirrors the
  `deploy/demo/langchain_ts/` shape.
- `deploy/demo/verify_step_vercel_ai_mastra.sql` — ledger-DB +
  cross-DB canonical_events assertions: `reserve >= 2`,
  `commit_estimated >= 2`, `denied_decision >= 1`, INV-2 strict-order
  (earliest reserve precedes earliest outcome), `audit_outbox.decision
  >= 2`.
- `deploy/demo/Makefile` extended with `DEMO_MODE=vercel_ai_mastra`
  branch + `demo-verify-vercel-ai-mastra` target.
- `deploy/demo/demo/run_demo.py::run_vercel_ai_mastra_mode` — Python
  counting-stub verifier (parity with `run_langchain_ts_mode`).
- Docs page at
  [`/docs/integrations/vercel-ai/`](https://agenticspendguard.dev/docs/integrations/vercel-ai/)
  on the Astro Starlight site — install / quick start / Mastra walkthrough /
  configuration / streaming semantics / limitations / demo /
  troubleshooting.
- Root `README.md` adapter integrations table now includes the
  `@spendguard/vercel-ai` row pointing at `examples/vercel-ai-mastra/`.

#### Publish pipeline (SLICE 8)

- `.github/workflows/sdk-vercel-ai-publish.yml` — npm Trusted Publisher
  OIDC pipeline triggered on `release` events tagged `vercel-ai-v*`.
  Provenance + `--access public` enforced at the publish call. Mirrors
  the D04 / D05 publish pipeline shape field-for-field.
- `scripts/size-budget.sh` enforces a 50 KB gzipped tarball ceiling
  (same as D04 — the middleware is thin glue; the budget headroom
  covers the `ai` peer-import surface + the Mastra subpath alias
  entry).
- `scripts/version-check.sh` asserts `package.json#version` matches
  `src/version.ts#VERSION` so the wire-reported `sdk_version` matches
  what npm published.
- `scripts/prepublish.sh` runs version-check → tsup build (clean).
  Idempotent.
- `package.json#scripts.prepublishOnly` invokes
  `scripts/prepublish.sh` so `npm publish` gates correctly even when
  run outside the CI workflow.

### Locked invariants

- **Public surface is `createSpendGuardMiddleware` +
  `SpendGuardMiddlewareOptions` + `VERSION` + the three
  pattern-matchable error classes (`SpendGuardError`,
  `DecisionDenied`, `SidecarUnavailable`).** `index.ts` re-exports
  exactly those names. No `default` export — review-standards §1.5
  LOCK.
- **`/mastra` subpath alias is a function-reference re-export.**
  `createSpendGuardLanguageMiddleware === createSpendGuardMiddleware`
  via strict equality. Locked by `tests/locked-surface.test.ts` AND
  by the runtime assertion at the top of
  `examples/vercel-ai-mastra/index.mjs`.
- **WeakMap stash keyed by params reference.** Module-level
  `WeakMap<LanguageModelV1CallOptions, StashEntry>`; review-standards
  §8 P0 LOCK. The GC collects the entry the moment the AI SDK drops
  the params reference; no manual cleanup, no leak.
- **`runId` is `llmCallId`** — exact equality. The middleware does not
  mint a separate `llmCallId` from `runId`. Both fields share the same
  value on the reserve / commit wire shape.
- **No re-export of `@spendguard/sdk` symbols** beyond the three
  pattern-matchable errors + the `VERSION` constant. Consumers import
  everything else (`SpendGuardClient`, idempotency helpers,
  `deriveUuidFromSignature`) directly.

### Compatibility

- Node 20.10+ (peer-aligned with `@spendguard/sdk`).
- `ai@^4.0.0` (declared as peer; middleware pins NEITHER —
  consumer's lockfile wins). AI SDK v5 migration is additive; will land
  in a future minor when AI SDK v5 ships as a stable peer.
- `@spendguard/sdk@^0.5.0` (declared as peer).
- `zod@>=3.22.0` (optional peer; the middleware does NOT import `zod`
  directly — declared optional so consumers who do not use AI SDK's
  structured outputs can omit it).

### Known limitations (deferred to future slices)

- **AI SDK v5 (`LanguageModelV2Middleware`) is anti-scope for v0.1.0.**
  The current peer is `ai@^4`, so the middleware targets
  `LanguageModelV1Middleware`. The v5 migration is additive — same
  options surface, same factory name; only the inner type
  parameterisation changes.
- **`UnitRef` broadening.** The TS SDK substrate's public `UnitRef`
  does not currently expose `unit_id` —
  `sdk/typescript/src/client.ts::mapUnitRef` hardcodes empty. The
  v0.1.0 options surface therefore omits the fuller
  `windowInstanceId` / `unit` / `pricing` / `claimEstimator` /
  `route` / `callSignature` / `runIdProvider` /
  `providerEventIdExtractor` field set that `design.md` §4
  anticipates. The middleware projects sensible defaults instead (route
  `"vercel-ai-llm"`, unit `USD_MICROS`, empty pricing freeze, char/4-token
  heuristic). The next D06 hardening slice picks up the SDK-side
  broadening + middleware wire-through together.
- **No mid-stream cap on streamed responses.** PRE + POST commit only
  in v0.1.0. Overruns land in the audit chain but the tokens were
  already emitted.
- **DEGRADE patch application surfaces as `MutationApplyFailed`.**
  Matches the Python `pydantic_ai.py::SpendGuardModel` v0.5.1 stance.
  Built-in claim mutation lands in a later slice.
- **No tool-call mid-loop gating.** Each tool call is its own AI SDK
  event; cross-tool budget enforcement is the contract layer's job.
- **Mastra `Workflow` step gating is anti-scope.** D06 covers Mastra
  `Agent` paths (which resolve to `generateText` / `streamText`); a
  separate adapter targets the workflow surface.

### Verification

- 54 tests pass under vitest 2.x (Node 20). 14 of the 54 are SLICE 6
  provider integration tests (OpenAI + Anthropic LanguageModelV1
  doubles); 3 are SLICE 7 locked-surface tests including the Mastra
  alias parity check; remaining 37 are SLICE 2-5 factory + reserve +
  commit + stream tests.
- `npm pack` tarball ≤ 50 KB gzipped (`scripts/size-budget.sh`).
- `package.json#version` == `src/version.ts#VERSION`
  (`scripts/version-check.sh`).
- Mastra alias parity:
  `createSpendGuardLanguageMiddleware === createSpendGuardMiddleware`
  (strict equality) under both unit test AND demo runtime assertion.

---

## Future versions

The lockstep contract with `spendguard-sdk[pydantic-ai]` (Python) v0.5.1
means the next minor (v0.2.x) will additively extend the options
surface toward the design.md §4 superset when the TS substrate broadens
`UnitRef`. Every post-0.1.0 addition is backward-compatible (new
optional fields only) so the v0.1.0 type lock holds.

[0.1.0]: https://github.com/m24927605/agentic-spendguard/releases/tag/vercel-ai-v0.1.0
