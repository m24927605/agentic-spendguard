# `@spendguard/inngest-agent-kit` Changelog

All notable changes to the Inngest AgentKit adapter for the SpendGuard
SDK.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This package adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

See [`docs/specs/coverage/D29_inngest_agent_kit/design.md`](https://github.com/m24927605/agentic-spendguard/blob/main/docs/specs/coverage/D29_inngest_agent_kit/design.md)
for the locked design and the headline retry-dedup contract.

---

## [0.1.0] — 2026-06-07

First public release. Closes deliverable D29 (Inngest AgentKit adapter).

### Added

#### Factory (SLICE 2 + 3)

- `wrapWithSpendGuard(stepAi, client, options): StepAi` — primary
  surface. Wraps an Inngest AgentKit `step.ai` namespace and threads
  every `infer()` / `wrap()` call through the SpendGuard PRE/POST
  bracket: `reserve` → provider → `commitEstimated`. Returns a
  type-preserving `StepAi`-shaped object so callers swap one line.
- `WrapWithSpendGuardOptions` — LOCKED v0.1.0 options surface:
  `tenantId` (required), `budgetId?`, `windowInstanceId?`, `unit?`,
  `pricing?`, `claimEstimator?`, `route?` (defaults to
  `"llm.call.inngest"`), `callSignatureFn?`, `claimEstimate?`,
  `onApprovalRequired?`, `idempotencyCache?`. Mirrors D04 / D06 / D08
  field-for-field minus `route` (Inngest-specific default).

#### Retry-dedup headline (SLICE 3)

- **The headline contract.** `wrapWithSpendGuard` derives the SpendGuard
  `idempotencyKey` from Inngest's own step identity — `step.id` and (when
  supplied) `step.idempotencyKey`. `attempt` is intentionally NOT part of
  the seed. N retry attempts of the same step body produce a
  byte-identical `idempotencyKey`, so D05's `DecisionCache` (and / or the
  sidecar's own idempotency dedup) returns the cached outcome and the
  adapter records ONE `LLM_CALL_PRE` audit row across N attempts.
- Layered defence: when the consumer supplies an `idempotencyCache`
  (typically `InMemoryIdempotencyCache` from `@spendguard/sdk`), the
  in-process cache absorbs the duplicate `reserve` without crossing the
  sidecar UDS. When not, the sidecar's own idempotency dedup catches the
  duplicate `idempotencyKey`. Both layers verified by R-03 / R-06.
- A NEW Inngest function invocation (new `ctx.runId`) for the same step
  name produces a DIFFERENT `idempotencyKey` — fresh runs are NOT
  deduped against prior runs (R-08 / IT-09 / IT-12).

#### Identity derivation (SLICE 2)

- `deriveIdentity({ tenantId, input }): DerivedIdentity` — Inngest step
  identity → SpendGuard identity tuple. Deterministic, attempt-invariant
  (review-standards §4.1 / §6.5).
- `deriveStepIdempotencyKey({ tenantId, runId, stepId,
  inngestIdempotencyKey? }): string` — convenience for callers who want
  to probe the dedup contract without constructing a full identity.
  Byte-identical to D05's `deriveIdempotencyKey` for the same canonical
  tuple.

#### PRE / INNER / POST bracket (SLICE 3)

- `client.reserve(...)` fires BEFORE the wrapped `step.ai.infer` body
  runs (review-standards §3.1). `DecisionDenied` / `DecisionStopped` /
  `DecisionSkipped` / `ApprovalRequired` from `reserve` propagate as
  their substrate-typed errors UNCHANGED; the inner body NEVER fires on
  these paths (review-standards §5).
- `SidecarUnavailable` propagates as-is (strict-mode default,
  review-standards §5.2 / §5.7). The future `degrade=auto` mode is
  LOCKED OUT of v0.1.x.
- On success the bracket fires `commitEstimated(...)` with
  `outcome="SUCCESS"`, `estimatedAmountAtomic=String(extractTotalTokens(result))`,
  and `providerEventId=extractProviderEventId(result)`.
- On provider-side throw the bracket fires `commitEstimated(...)` with
  `outcome="PROVIDER_ERROR"` and `estimatedAmountAtomic="0"`, then
  re-throws. Commit failure is logged but does NOT mask the original
  provider error (review-standards §5.10).

#### Token-usage extraction (SLICE 3)

- `extractTotalTokens(result)` — probe order:
  `result.usage.total_tokens` (OpenAI) →
  `result.usage_metadata.total_tokens` (Anthropic / Gemini) →
  `result.response_metadata.token_usage.total_tokens` (legacy) → `0`.
  Tolerates camelCase + snake_case + non-object `usage`
  (review-standards §7).
- `extractProviderEventId(result)` — probe order: `result.id` →
  `result.response_metadata.id` → `""`. NEVER throws.

#### Errors

- Re-exports `SpendGuardError`, `DecisionDenied`, `DecisionStopped`,
  `DecisionSkipped`, `ApprovalRequired`, `SidecarUnavailable` directly
  from `@spendguard/sdk` (preserves class identity so
  `err instanceof DecisionDenied` works across the adapter ↔ substrate
  boundary).

#### Tests (SLICE 4)

- vitest suite: 53 unit tests (`wrap.test.ts`) + 23 integration tests
  (`integration.test.ts`) including:
  - W-01..W-17 factory + reserve/commit unit tests.
  - R-01..R-08 retry-dedup contract (the headline gate).
  - E-01..E-10 error propagation.
  - I-01..I-07 identity-derivation invariants.
  - X-01..X-08 token-usage extract probe order.
  - IT-01..IT-23 end-to-end retry-replay matrices through an enhanced
    `runStepUntil(...)` harness that simulates Inngest's
    deterministic-retry semantics: same `(runId, stepId,
    idempotencyKey)` across attempts, `attempt` advances each iteration,
    fatal substrate errors stop replay (NonRetriable semantics).
- 76 tests total; all green.

#### Demo (SLICE 5)

- `examples/inngest-agent-kit/` — runnable Node demo with `--mock`
  (laptop, no sidecar) and `--real` (sidecar UDS + counting-stub or real
  OpenAI). 3-step ALLOW + DENY + RETRY_DEDUP matrix with the locked
  success line
  `[demo] inngest_agent_kit ALL 3 steps PASS (ALLOW + DENY + RETRY_DEDUP)`.
  `RETRY_DEDUP` replaces D04 / D06 / D08's `STREAM` step because
  `step.ai.infer` is non-streaming (design.md §3 non-goal).
- `deploy/demo/inngest_agent_kit/` — docker-compose overlay layered on
  the base demo stack. `make demo-up DEMO_MODE=inngest_agent_kit`
  brings up `postgres + sidecar + outbox-forwarder + counting-stub +
  inngest-agent-kit-runner` and runs the demo's `--real` mode
  end-to-end.
- `deploy/demo/verify_step_inngest_agent_kit.sql` — ledger-side gates
  including the headline `COV_D29_DEDUP_GATE`: `reserve == 2` (ALLOW
  produces 1 + RETRY_DEDUP collapses 3 attempts into 1; total must stay
  at 2). If the dedup contract is broken the gate trips with
  `RETRY_DEDUP must dedup 3 attempts → 1 reservation`.

### Known limitations

- **Stream-per-chunk gating** is anti-scope for v0.1.x — `step.ai.infer`
  is non-streaming by Inngest design (design.md §3 non-goal).
- **Cross-step budget enforcement** is contract-layer scope, not
  adapter scope (design.md §3 non-goal).
- **Approval-resume UI** — `ApprovalRequired` flows through to the
  caller's `onApprovalRequired` callback, but the adapter does NOT ship
  an approval UI. Resume semantics documented inline in the README.
- **Substrate dependency on `UnitRef` broadening.** The TS substrate's
  public `UnitRef` does not yet expose `unit_id`
  (`sdk/typescript/src/client.ts::mapUnitRef` hardcodes empty); the
  `make demo-up DEMO_MODE=inngest_agent_kit` `--real` flow surfaces the
  substrate-side `claim[0].unit.unit_id empty` error from the sidecar's
  `ReserveSet` validator. This is the known cross-slice D05 UnitRef gap
  — same surface as D04 / D06 / D08. The adapter's wire shape and
  retry-dedup contract are independently verified by the 76 vitest
  cases; the `--mock` mode exercises the bracket end-to-end without
  going through the substrate validator. A future hardening slice picks
  up the SDK-side broadening + adapter wire-through together.
- **Browser is unsupported** (D05 §6 UDS-only).

### Compatibility

- Node 20.10+ (the substrate uses stable `AsyncLocalStorage`).
- ESM-only — `"type": "module"` in `package.json`. Consumers must use
  `import` / `await import`.
- Peer dependencies: `@inngest/agent-kit` `>=0.13 <1`, `@spendguard/sdk`
  workspace-pinned (the substrate is pre-1.0 — the publish workflow
  rewrites the workspace dep to `^x.y.z` at publish time).
