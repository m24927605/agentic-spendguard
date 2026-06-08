# `@spendguard/sdk` Changelog

All notable changes to the TypeScript half of the SpendGuard SDK.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This package adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

The Python SDK ([spendguard-sdk on PyPI](https://pypi.org/project/spendguard-sdk/),
currently v0.5.1) and this TypeScript SDK are kept in lockstep on the public
surface — see
[`docs/specs/coverage/D05_ts_sdk_substrate/design.md`](../../docs/specs/coverage/D05_ts_sdk_substrate/design.md)
§9 for the lockstep contract.

---

## [Unreleased]

### Added

- `UnitRef.unitId` — optional canonical-truth UUID of the ledger unit row.
  Adapters that issue ledger-backed reserve calls now thread this through to
  `BudgetClaim.unit.unit_id` on the wire. Closes the HARDEN_D05_UR substrate
  gap that previously blocked DENY+STREAM full assertion across ~14 adapter
  demos. Backward-compat: omitting `unitId` matches prior behavior.

---

## [0.1.0] — 2026-06-07

First public release. Mirrors `spendguard-sdk` (Python) v0.5.1 public surface.
Closes deliverable D05 (TypeScript SDK substrate).

### Added

#### Client (SLICE 3-5)

- `SpendGuardClient` — gRPC-over-UDS client for the SpendGuard sidecar.
  - `handshake()` — protocol-version + capabilities negotiation.
  - `reserve()` — pre-flight budget decision (the canonical method name).
  - `requestDecision()` — `reserve()` alias for migration ergonomics; `===`
    identity is asserted as a P0 surface invariant
    (review-standards §1.5).
  - `commitEstimated()` — observed-output commit (cost rounded up).
  - `release()` — terminal reservation release with explicit
    `releaseReason`.
  - `queryBudget()` — wire stub (throws "not yet wired" until cross-component
    slice; documented in JSDoc per review-standards §11.3).
- `SpendGuardClientOptions`, `HandshakeOutcome`, `DecisionOutcome`,
  `ReleaseOutcome` types.
- Default deadlines: `DEFAULT_DECISION_TIMEOUT_MS`,
  `DEFAULT_HANDSHAKE_TIMEOUT_MS`, `DEFAULT_PUBLISH_TIMEOUT_MS`,
  `DEFAULT_TRACE_TIMEOUT_MS`.
- `unix:` URI handling matches Python (sets
  `grpc.default_authority: "localhost"` so `tonic` does not 400 with
  `PROTOCOL_ERROR` on the URL-encoded path).

#### IDs (SLICE 6)

- `newUuid7()` — UUIDv7 generator (sortable, embedded ms timestamp).
- `deriveIdempotencyKey()` — deterministic `sg-…` key. **P0 cross-language
  byte-equivalent with Python `spendguard.derive_idempotency_key()` and the
  Rust sidecar's `audit_chain::derive_key`** (sidecar-side dedup correctness).
- `deriveUuidFromSignature()` — BLAKE2b-based UUID derivation from an
  application signature + scope. **Byte-equivalent with Python.** Uses
  `@noble/hashes` BLAKE2b implementation.
- `defaultCallSignature()` — framework-agnostic signature helper.
- `workloadInstanceId()` — process-stable identifier.

#### Pricing (SLICE 6)

- `PricingLookup` — typed lookup with provider+model+kind keys.
- `USD_MICROS_PER_USD` — wire-unit conversion constant.
- `DEMO_PRICING` — embedded pricing snapshot for the demo seed
  (`pricing_version: v2026.05.09-1`, regenerated at release time from
  `deploy/demo/init/pricing/seed.yaml`).

#### Prompt hash (SLICE 6)

- `computePromptHash()` — HMAC-SHA256 lowercase hex. **P0 cross-language
  byte-equivalent with Python `spendguard.compute_prompt_hash()` and the
  sidecar `prompt_hash::compute`**. Tenant ID is canonicalised to lowercase
  before hashing — the `crossLanguage` test suite pins this.

#### Run plan / Signal 3 (SLICE 7)

- `withRunPlan()` + `currentRunPlan()` — `AsyncLocalStorage`-scoped
  budget-hint context. Adapters set the plan once at run boundary; nested
  `reserve()` calls read it transparently.

#### OTel (SLICE 8)

- `otelTracer` config field — optional `@opentelemetry/api` `Tracer`.
- Per-RPC spans named `spendguard.<rpc>` with attributes documented in
  `design.md` §6.4 + `SPENDGUARD_OTEL_ATTR` constant export.
- `@opentelemetry/api` is a peer dep, marked optional in
  `peerDependenciesMeta`.

#### Retry (SLICE 8)

- Bounded retry for the `UNAVAILABLE` / `DEADLINE_EXCEEDED` / `CANCELLED`
  cluster. Max 2 attempts, constant 25 ms + jitter backoff. Idempotency-key
  required (the substrate enforces).

#### Idempotency cache (SLICE 8)

- In-process LRU keyed by `(tenantId, idempotencyKey)` so re-attempts after
  retry observation surface the cached decision rather than re-RPC'ing.

#### Cross-language fixtures (SLICE 9)

- `sdk/fixtures/cross-language/v1.json` — the SINGLE SOURCE OF TRUTH for
  byte-equivalence between the Rust sidecar, Python SDK, and TS SDK. v1.json
  ships with ≥20 fixtures spanning `compute_prompt_hash`,
  `derive_idempotency_key`, and `derive_uuid_from_signature` (see
  `sdk/fixtures/cross-language/README.md` for the add-a-fixture / mint-v2
  runbook).
- The fixture file is **included in the published npm tarball** under
  `fixtures/cross-language/v1.json` so consumer-side conformance suites can
  pin against the exact same vectors.

#### Errors

- `SpendGuardError`, `HandshakeError`, `SidecarUnavailable`,
  `DecisionDenied`, `DecisionStopped`, `DecisionSkipped`, `ApprovalRequired`,
  `ApprovalDeniedError`, `ApprovalLapsedError`,
  `ApprovalBundleHotReloadedError`, `MutationApplyFailed`,
  `SpendGuardConfigError` — full hierarchy mirror of the Python SDK.

### Locked invariants

- **P0 cross-language byte-equivalence** with the Python SDK (and the Rust
  sidecar) on `computePromptHash`, `deriveIdempotencyKey`, and
  `deriveUuidFromSignature`. Drift breaks audit-chain dedup and the
  idempotency replay collapse contract. Enforced by the
  `tests/crossLanguage.test.ts` suite consuming
  `sdk/fixtures/cross-language/v1.json` (also consumed by
  `sdk/python/tests/test_cross_language_fixtures.py`).
- **`reserve()` === `requestDecision()` reference identity** (P0 surface
  invariant, `tests/locked-surface.test.ts`).
- **ESM-only.** No CJS shim. `"type": "module"`.
- **camelCase on the public surface, snake_case on the wire** — every wire
  conversion is centralised in `src/client.ts` so adapter authors never see
  snake_case.

### Compatibility

- Node 20.10+ (uses `using` / `await using`, stable `AsyncLocalStorage`,
  Web Crypto).
- Bun 1.1+ tested as secondary target.
- Deno 1.46+ tested as secondary target.
- **Browser is NOT supported in v0.1.x.** UDS gRPC is server-only. A future
  v0.x ASP HTTP gateway transport is forward-reserved in the config type but
  not built.

### Known limitations (deferred to future slices)

- `queryBudget()` wire body — stub-throws "not yet wired". Cross-component
  slice unblocks.
- LLM_CALL_OUTCOME proto bump — deferred to a cross-component slice that
  spans D05 + Rust sidecar + Python SDK simultaneously.
- Per-framework `defaultCallSignature()` defaults for LangChain / Vercel AI
  / OpenAI Agents / Inngest — each adapter package (D04 / D06 / D08 / D29)
  provides its own message-type-specific signature.
- Identity-propagation `RunContext` (parentRunId / traceparent threading) —
  deferred per SLICE 7 R2. Use the explicit `ReserveRequest` fields for now.

### Verification

- 366 tests pass under vitest 2.x (Node 20).
- `npm pack` tarball ≤ 250 KB (`scripts/size-budget.sh`).
- Minified `dist/index.js` ≤ 120 KB (design.md §10 budget).
- `package.json#version` == `src/version.ts#VERSION` (`scripts/version-check.sh`).
- All P0 byte-equivalence fixtures match the Python SDK.

---

## Future versions

The lockstep contract with `spendguard-sdk` (Python) v0.5.1 means the next
minor (v0.2.x) will track whatever Python's next minor mints. Any change to
the v0.1.0 public surface listed under "Added" requires a coordinated
v0.minor bump on both packages — see
[`docs/specs/coverage/D05_ts_sdk_substrate/design.md`](../../docs/specs/coverage/D05_ts_sdk_substrate/design.md)
§§4 / 9 for the contract.

[0.1.0]: https://github.com/m24927605/agentic-spendguard/releases/tag/ts-sdk-v0.1.0
