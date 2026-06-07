# `@spendguard/openai-agents` Changelog

All notable changes to the OpenAI Agents SDK (TypeScript) adapter for the
SpendGuard SDK.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This package adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

The Python sibling — [`spendguard-sdk[openai-agents]`](https://pypi.org/project/spendguard-sdk/)
on PyPI, currently v0.5.1 — and this TypeScript adapter are kept in
lockstep on the bracket semantics. See
[`docs/specs/coverage/D08_openai_agents_ts/design.md`](https://github.com/m24927605/agentic-spendguard/blob/main/docs/specs/coverage/D08_openai_agents_ts/design.md)
for the cross-language behaviour parity matrix and
[`sdk/python/src/spendguard/integrations/openai_agents.py`](https://github.com/m24927605/agentic-spendguard/blob/main/sdk/python/src/spendguard/integrations/openai_agents.py)
for the Python source the bracket is mirrored from.

---

## [0.1.0] — 2026-06-07

First public release. TS counterpart of `spendguard-sdk[openai-agents]`
(Python) v0.5.1; closes deliverable D08 (OpenAI Agents TS adapter).

### Added

#### Factory + class form (SLICE 2)

- `withSpendGuard(inner, opts): Model` — primary surface. Wraps any
  `@openai/agents` `Model` (typically `OpenAIChatCompletionsModel` /
  `OpenAIResponsesModel` from `@openai/agents-openai`, or any custom
  `Model` impl) and runs every `getResponse(request)` through the SLICE 2
  PRE/POST bracket from `./core.ts`.
- `SpendGuardAgentsModel` — class form. Implements the same `Model`
  interface, delegates to the same shared `bracketedGetResponse(...)`. No
  bracket drift between the two surfaces (review-standards.md §1.2
  reviewer gate).
- `SpendGuardAgentsOptions` — LOCKED v0.1.0 options surface: `client`,
  `tenantId`, `budgetId?`. Additive optional fields will land in future
  minors per the design.md §4 superset (`windowInstanceId` / `unit` /
  `pricing` / `claimEstimator`) — see **Known limitations** below.

#### Shared run context (SLICE 2)

- `runContext({ runId }, async () => …)` — Node `AsyncLocalStorage`-backed
  per-invocation context. The storage is keyed on
  `Symbol.for("@spendguard/run-context/v1")` so D04 / D06 / D08 / D29 all
  observe the SAME `AsyncLocalStorage` slot at runtime — a multi-framework
  agent shares ONE trace across LangChain.js + Vercel AI + Agents SDK
  without manual plumbing.
- `currentRunContext(): RunContext` — read-side. Throws a typed error
  outside any active context so the adapter's PRE hook fails loud rather
  than silently fabricating a run id.

#### Signature + UUID derivation (SLICE 2)

- `deriveAgentSignature(input, systemInstructions)` — BLAKE2b-128 hex
  digest. Mirrors the Python `_signature(...)` byte shape; the
  cross-language fixture extension at
  `sdk/fixtures/cross-language/v1.json#FXOA*` rows gates byte-equivalence.
- `(decisionId, llmCallId)` minted via the substrate's
  `deriveUuidFromSignature(...)` with distinct scope tags. The same
  signature feeds both UUIDs — review-standards.md §2.2 / §2.3.
- `idempotencyKey` minted via `deriveIdempotencyKey(...)` from
  `(tenantId, sessionId=runId, runId, stepId, llmCallId, trigger)` —
  byte-identical to the Python adapter (review-standards.md §2.4).

#### PRE / INNER / POST bracket (SLICE 2)

- `client.reserve(...)` fires BEFORE `inner.getResponse(...)` — reviewer
  gate 1.1. DENY / STOP / SKIP / APPROVAL_REQUIRED rejections from
  `reserve` propagate as their substrate-typed errors UNCHANGED; the inner
  Model is NEVER reached on these paths (reviewer gate 1.3).
- On CONTINUE / DEGRADE outcomes the bracket forwards the request to the
  inner verbatim. The DEGRADE-mutation-apply path is anti-scope for v0.1.x
  (design §3 non-goals); a future minor lifts it when the substrate ships
  the patch-apply hook.
- On a non-empty `outcome.reservationIds` array, the bracket fires
  `client.commitEstimated(...)` with `outcome="SUCCESS"` on inner success
  or `outcome="PROVIDER_ERROR"` on inner throw. Commit-side failures are
  warned-and-returned (NOT swallowed silently); the inner response is
  preserved — review-standards.md §10.2.

#### Default claim estimator (SLICE 3)

- `MODEL_BASELINE_TOKENS` — design.md §11 literal table mapping known
  OpenAI model ids to per-call baseline token counts (`gpt-4o-mini`:
  500, `gpt-4o`: 1500, `gpt-4.1-mini`: 500, `gpt-4.1`: 1500, `o1`: 3000,
  `o3-mini`: 1500, `o3`: 3000).
- `DEFAULT_BASELINE_TOKENS` = 800 — unknown-model fallback (reviewer
  gate 9.5).
- `defaultClaimEstimator(...)` — internal helper used by the bracket's
  default projection. A caller-supplied `claimEstimator` (lands additive
  in a future minor on the public options surface) overrides this
  default — Python parity "explicit non-null wins".

#### Cross-language fixture extension (SLICE 3)

- `sdk/fixtures/cross-language/v1.json` extended with the `openai_agents`
  section (`FXOA*` rows): 5 `derive_agent_signature` vectors covering
  string + list-of-message + empty + multi-byte-UTF-8 + sys-instructions
  shapes, plus a tripled `(idempotencyKey, decisionId, llmCallId)`
  derivation chain for the FXOA1 seed. The integration test suite at
  `tests/integration.test.ts` reads and asserts byte-equality against
  these rows — review-standards.md §2.5 / §2.6 / §2.7.

#### Errors

- Re-exports `SpendGuardError`, `DecisionDenied`, `DecisionStopped`,
  `ApprovalRequired`, `SidecarUnavailable` directly from `@spendguard/sdk`
  (preserves class identity so `err instanceof DecisionDenied` works
  across the adapter ↔ substrate boundary). Other substrate errors
  (`DecisionSkipped`, `MutationApplyFailed`, …) remain importable from
  `@spendguard/sdk` directly.

#### Tests (SLICE 3)

- vitest suite: 40 unit tests (`factory.test.ts`) + 21 integration tests
  (`integration.test.ts`) + 5 locked-surface tests covering:
  - ALLOW lifecycle (reserve → inner → commit SUCCESS).
  - DENY / STOP / APPROVAL_REQUIRED → inner NEVER reached (reviewer gate
    1.3 enforced on every path).
  - SIDECAR_UNAVAILABLE propagates unchanged (no degrade in v0.1.x).
  - PROVIDER_ERROR path: commit fires with `outcome="PROVIDER_ERROR"`
    + `estimatedAmountAtomic="0"`, then the error rethrows.
  - Commit-side failure does NOT corrupt the inner response.
  - Stream pass-through with no PRE/POST gating (reviewer gate 1.5).
  - Default-estimator dispatch (`gpt-4o-mini` → 500, `gpt-4o` → 1500,
    unknown → 800).
  - Cross-language fixture parity (`FXOA*` rows in v1.json).
  - Multiple wraps independent (no cross-tenant leakage).

#### Demo (SLICE 4 + 5)

- `examples/openai-agents-ts-composite/` — runnable Node demo with
  `--mock` (laptop, no sidecar) and `--real` (sidecar UDS + real
  `@openai/agents` Agent + Runner). 3-step ALLOW + DENY + STREAM matrix
  with the locked success line
  `[demo] openai_agents_ts ALL 3 steps PASS (ALLOW + DENY + STREAM)`.
- `deploy/demo/openai_agents_ts/` — docker-compose overlay layered on
  the base demo stack. `make demo-up DEMO_MODE=openai_agents_ts`
  brings up `postgres + sidecar + outbox-forwarder + counting-stub +
  openai-agents-runner` and runs the demo's `--real` mode end-to-end.
- `deploy/demo/verify_step_openai_agents_ts.sql` — ledger-side gates:
  `reserve >= 2`, `commit_estimated >= 2`, `denied_decision >= 1`,
  INV-2 strict-order proof.

### Known limitations

- **Stream-per-chunk gating** is anti-scope for v0.1.x (design.md §3
  non-goals). `withSpendGuard`'s `getStreamedResponse` is pass-through:
  the stream forwards verbatim with no PRE/POST around it. A future
  minor lifts this when the substrate's `LLM_STREAM_DELTA` trigger
  ships.
- **DEGRADE mutation patch application** is also anti-scope. Substrate
  DEGRADE outcomes flow through as CONTINUE — the bracket does NOT
  rewrite the request. `MutationApplyFailed` surfaces parity with the
  Python sibling's v0.5.1 stance.
- **Substrate dependency on `UnitRef` broadening.** Per design.md §4
  superset, the v0.1.0 options surface omits `windowInstanceId`,
  `unit`, `pricing`, and `claimEstimator`. The TS substrate's public
  `UnitRef` does not yet expose `unit_id` — a future hardening slice
  picks up the SDK-side broadening + adapter wire-through together
  (parity with D04 / D06 SLICE 3 surface-narrowing deviations).
- **Default estimator is the literal `MODEL_BASELINE_TOKENS` table** —
  per-model tokenizer dispatch (Strategy A) is the Python sibling's
  v0.5.x extension and lands in a future TS minor as additive optional
  (caller-supplied `claimEstimator` already covers the escape hatch).
  Documented inline in `defaultEstimator.ts` Deviation #1.
- **Browser is unsupported** (D05 §6 UDS-only).

### Compatibility

- Node 20.10+ (the substrate uses `await using` + stable
  `AsyncLocalStorage`).
- ESM-only — `"type": "module"` in `package.json`. Consumers must use
  `import` / `await import`.
- Peer dependencies: `@openai/agents` `>=0.11 <1`, `@spendguard/sdk`
  workspace-pinned (the substrate is pre-1.0 — the publish workflow
  rewrites the workspace dep to `^x.y.z` at publish time).
