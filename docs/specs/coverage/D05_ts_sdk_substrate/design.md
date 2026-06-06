# D05 — TypeScript SDK Substrate (`@spendguard/sdk` npm package)

**Status:** Spec — Tier 2 (build plan `framework-coverage-build-plan-2026-06.md` §2.2).
**Parent strategy:** [`docs/strategy/framework-coverage-2026-06.md`](../../../strategy/framework-coverage-2026-06.md), Pattern 1 (model-abstraction middleware, TS half).
**Build plan:** [`docs/strategy/framework-coverage-build-plan-2026-06.md`](../../../strategy/framework-coverage-build-plan-2026-06.md) §2.2, §2.4.
**Owner sub-agent:** Frontend Developer.
**Dependency edges (downstream blockers):** D04 (LangChain TS), D06 (Vercel AI SDK + Mastra), D08 (OpenAI Agents TS), D29 (Inngest AgentKit).

> **Contract notice.** Sections 4 / 5 / 6 / 9 lock the public surface that D04 / D06 / D08 / D29 build against. Changes to those sections after this spec is merged require a v0.minor bump and a coordinated update of every downstream deliverable's spec. Bug-fix-only changes (clarification, doc, internal refactor) may land without coordination.

## 1. Problem

The Python SDK (`spendguard-sdk` on PyPI, currently v0.5.1) ships a complete `SpendGuardClient` + per-framework `integrations/*` substrate. The TS half of the ecosystem is empty: D04 (LangChain TS), D06 (Vercel AI SDK + Mastra), D08 (OpenAI Agents TS), D29 (Inngest AgentKit) cannot ship without (a) a generated proto client targeting the v1 sidecar UDS wire, (b) a typed `Client` mirroring the Python public surface, (c) the supporting helpers (idempotency-key derivation, prompt-hash, pricing table, run-plan context, error hierarchy), and (d) a publish pipeline.

A TS adapter SHOULD NOT carry its own proto codegen, idempotency derivation, or pricing helper — that yields four divergent copies in six months and breaks audit-chain determinism (the Rust sidecar's `prompt_hash::compute` must equal both the Python and TS `compute()`). D05 builds the single shared substrate.

## 2. Goals

1. Ship a published `@spendguard/sdk` npm package, version `0.1.0` (first TS release), licence Apache-2.0, repository `m24927605/agentic-spendguard`, located in-tree at `sdk/typescript/`.
2. Public surface mirrors `spendguard-sdk` (Python) one-to-one in name and semantic — see §4. Same docstring intent so a customer reading Python docs can call TS without surprise.
3. Codegen proto types + service stubs from `proto/spendguard/` via a deterministic, lockfile-checked `pnpm run proto` script. The generated tree is committed (not gitignored) so the wheel and consumer builds do not need `protoc` at install time.
4. Module format: ESM-only, tree-shakeable, `"type": "module"`, dual TS declaration files. No CJS shim.
5. Runtime target: Node 20 LTS as the primary, Bun 1.1+ and Deno 1.46+ as documented secondary targets. Browser is **explicitly out of scope** for the substrate (UDS + gRPC are server-only); however, a `runtime: "fetch"` mode is forward-reserved in the config type for future ASP HTTP gateway transport (§9.3).
6. Publish pipeline: GitHub Actions Trusted Publisher OIDC to npm (mirrors `sdk-publish.yml` PyPI flow), triggered on `ts-sdk-v*` git tags. Provenance attestation enabled (`npm publish --provenance`).
7. Per-framework adapter packages (D04 / D06 / D08 / D29) live in **separate npm packages** under `@spendguard/*` scope and depend on `@spendguard/sdk` as a peer dep — they are NOT folded into this package. This spec only delivers the substrate.

## 3. Non-goals

- D04 / D06 / D08 / D29 adapter implementations: each gets its own spec set and slice plan. D05 publishes only the core.
- Browser runtime: out of scope. UDS-only transport in v0.1.x. A future v0.x ASP HTTP gateway transport (`runtime: "fetch"`) is a forward-reserved type slot, not built here.
- Replacing the Python SDK: both ship and stay in lockstep. Public-surface lock-step is enforced by the contract notice at the top of this file.
- `@spendguard/cli` (matches D02 Rust `spendguard install`): D02 ships the install tooling in Rust. The TS SDK only exposes a programmatic API.
- Code-generating per-framework helpers (auto-build `wrapLanguageModel` from a contract): the four adapters are hand-written.

## 4. Public surface — LOCKED

This is the contract D04 / D06 / D08 / D29 import. The four adapter specs MAY assume every symbol listed here is exported from `@spendguard/sdk` at v0.1.0, with the documented type and semantic.

### 4.1 Package layout (consumer-facing)

```ts
import {
  // Client
  SpendGuardClient,
  type SpendGuardClientOptions,
  type HandshakeOutcome,
  type DecisionOutcome,
  type ReleaseOutcome,
  // Errors
  SpendGuardError,
  HandshakeError,
  SidecarUnavailable,
  DecisionDenied,
  DecisionStopped,
  DecisionSkipped,
  ApprovalRequired,
  ApprovalDeniedError,
  ApprovalLapsedError,
  ApprovalBundleHotReloadedError,
  MutationApplyFailed,
  SpendGuardConfigError,
  // IDs
  newUuid7,
  deriveIdempotencyKey,
  deriveUuidFromSignature,
  defaultCallSignature,
  workloadInstanceId,
  // Pricing
  PricingLookup,
  USD_MICROS_PER_USD,
  type PriceKey,
  type PriceTable,
  // Prompt hash
  computePromptHash,
  // Run plan (Signal 3)
  withRunPlan,
  currentRunPlan,
  type RunPlan,
  // Constants
  DEFAULT_DECISION_TIMEOUT_MS,
  DEFAULT_HANDSHAKE_TIMEOUT_MS,
  DEFAULT_PUBLISH_TIMEOUT_MS,
  DEFAULT_TRACE_TIMEOUT_MS,
  VERSION,
} from "@spendguard/sdk";
```

Subpath exports (tree-shaking + smaller adapter bundles):

| Subpath | Purpose |
|---|---|
| `@spendguard/sdk` | The full surface above (re-exports). |
| `@spendguard/sdk/client` | `SpendGuardClient` + its types only. |
| `@spendguard/sdk/errors` | Error hierarchy only. |
| `@spendguard/sdk/ids` | UUID + idempotency helpers. |
| `@spendguard/sdk/pricing` | `PricingLookup`. |
| `@spendguard/sdk/prompt-hash` | `computePromptHash`. |
| `@spendguard/sdk/run-plan` | `withRunPlan` + `currentRunPlan`. |
| `@spendguard/sdk/proto` | Generated proto types + service clients. |

### 4.2 `SpendGuardClient` — class shape

```ts
export interface SpendGuardClientOptions {
  // Transport
  socketPath: string; // UDS path; required in v0.1.x
  // Identity
  tenantId: string;
  runtimeKind?: string;   // e.g. "vercel-ai-sdk" / "langchain-js" / "openai-agents-ts" — default ""
  runtimeVersion?: string;
  sdkVersion?: string;    // defaults to this package's VERSION constant
  protocolVersion?: number; // default 1
  capabilityLevel?: number; // default 0x40 (L3_POLICY_HOOK)
  workloadInstanceId?: string;
  // Deadlines (ms — TS-idiomatic, NOT seconds)
  decisionTimeoutMs?: number;   // default 250
  handshakeTimeoutMs?: number;  // default 2000
  publishTimeoutMs?: number;    // default 150
  traceTimeoutMs?: number;      // default 500
  // Observability hooks (optional)
  onSpan?: (span: SpanRecord) => void;
  // OTel — when provided, the client emits the same spans through it
  // INSTEAD of onSpan; the two are mutually exclusive (config error
  // if both set).
  otelTracer?: import("@opentelemetry/api").Tracer;
  // Forward-reserved (not used in v0.1.x; types defined for D04-D29 to
  // import without a version bump later)
  runtime?: "uds-grpc"; // future: "fetch"
}

export class SpendGuardClient implements AsyncDisposable {
  constructor(opts: SpendGuardClientOptions);
  // Lifecycle
  connect(): Promise<void>;
  close(): Promise<void>;
  // AsyncDisposable: `await using client = new SpendGuardClient(...)`
  [Symbol.asyncDispose](): Promise<void>;

  // Read-only state
  readonly tenantId: string;
  readonly sessionId: string;        // throws HandshakeError until handshake completes
  readonly handshakeOutcome: HandshakeOutcome;

  // Handshake
  handshake(opts?: { workloadInstanceId?: string }): Promise<HandshakeOutcome>;

  // ── Core RPC surface — names mirror the Python SDK with TS naming. ──

  // reserve(...) — equivalent to Python `request_decision`. Named
  // `reserve` per the v0.1.x public surface to align with ASP Draft-01
  // wire vocabulary AND the customer-facing build-plan deliverable
  // table. An alias `requestDecision` is also exported (= same function
  // reference) so Python users searching for the symbol find it.
  reserve(req: ReserveRequest): Promise<DecisionOutcome>;
  requestDecision: SpendGuardClient["reserve"]; // alias, identical fn ref

  // commitEstimated(...) — equivalent to Python `emit_llm_call_post`
  // with outcome=SUCCESS + estimated_amount_atomic. The TS SDK only
  // ships the CommitEstimated path in v0.1.x (no ProviderReport),
  // matching the Python `LlmCallPostPayload` Stage 7 mode.
  commitEstimated(req: CommitEstimatedRequest): Promise<void>;

  // release(...) — equivalent to Python `release_reservation`.
  // Matches ASP Draft-01 §4 wire one-to-one.
  release(req: ReleaseRequest): Promise<ReleaseOutcome>;

  // queryBudget(...) — read-only budget snapshot. Mirrors Python's
  // ad-hoc `query_budget` (introduced in 0.5.1 README; calls the
  // sidecar adapter's read-side RPC; in v0.1.0 the substrate ships
  // the wire path but a placeholder server response — see §9.4 for
  // deferred wiring).
  queryBudget(req: QueryBudgetRequest): Promise<QueryBudgetResult>;

  // ── Lower-level operations (exposed for adapters that need them) ──

  confirmPublishOutcome(req: PublishOutcomeRequest): Promise<string>;
  resumeAfterApproval(req: ResumeAfterApprovalRequest): Promise<DecisionOutcome>;
  safeConfirmApplyFailed(req: ApplyFailedRequest): Promise<void>;
  emitLlmCallPost(req: EmitLlmCallPostRequest): Promise<void>;
}
```

### 4.3 `ReserveRequest` (TS analog of Python `request_decision` kwargs)

```ts
export interface ReserveRequest {
  trigger: "RUN_PRE" | "AGENT_STEP_PRE" | "LLM_CALL_PRE" | "TOOL_CALL_PRE";
  runId: string;
  stepId: string;
  llmCallId: string;
  toolCallId?: string;
  decisionId: string;
  route: string;
  projectedClaims: BudgetClaim[];
  idempotencyKey: string;
  traceparent?: string;
  tracestate?: string;
  parentRunId?: string;
  budgetGrantJti?: string;
  // Risk band hints (decimal strings, NUMERIC(38,0))
  projectedP50Atomic?: string;
  projectedP90Atomic?: string;
  projectedP95Atomic?: string;
  projectedP99Atomic?: string;
  projectedUnit?: UnitRef;
  promptText?: string;
  decisionContextJson?: Record<string, unknown>;
  claimEstimate?: ClaimEstimate;
}
```

Field-naming convention: **TS camelCase on the public surface, proto snake_case on the wire.** The codegen layer (§7) handles the mapping; adapters never touch snake_case.

### 4.4 `CommitEstimatedRequest`, `ReleaseRequest`, `QueryBudgetRequest`, `DecisionOutcome`, `ReleaseOutcome`

See `implementation.md` §3 for full TypeScript signatures. Each field name is the camelCase of its Python counterpart, every semantic is preserved.

### 4.5 Error hierarchy (mirror of `errors.py`)

```ts
class SpendGuardError extends Error {}
class HandshakeError extends SpendGuardError {}
class SidecarUnavailable extends SpendGuardError {
  readonly statusCode: 503;
}
class DecisionDenied extends SpendGuardError {
  readonly statusCode: 403;
  readonly decisionId: string;
  readonly reasonCodes: string[];
  readonly auditDecisionEventId?: string;
  readonly matchedRuleIds: string[];
}
class DecisionStopped extends DecisionDenied {}
class DecisionSkipped extends DecisionDenied {}
class ApprovalRequired extends DecisionDenied {
  readonly approvalRequestId: string;
  readonly approverRole?: string;
  readonly tenantId?: string;
  resume(client: SpendGuardClient): Promise<DecisionOutcome>;
}
class ApprovalDeniedError extends DecisionDenied {
  readonly approverSubject?: string;
  readonly approverReason?: string;
}
class ApprovalLapsedError extends DecisionDenied {
  readonly state: "pending" | "expired" | "cancelled" | "unknown";
}
class ApprovalBundleHotReloadedError extends SpendGuardError {
  readonly originalBundleHash: string;
  readonly currentBundleHash: string;
}
class MutationApplyFailed extends SpendGuardError {}
class SpendGuardConfigError extends SpendGuardError {}
```

### 4.6 ID helpers — mirror of `ids.py`

```ts
function newUuid7(): string; // 36-char canonical UUIDv7 (RFC 9562 §5.7)

function deriveIdempotencyKey(args: {
  tenantId: string;
  sessionId: string;
  runId: string;
  stepId: string;
  llmCallId: string;
  trigger: string;
}): string; // "sg-<32 hex>"; BYTE-FOR-BYTE identical to Python output

function deriveUuidFromSignature(signature: string, args: { scope: string }): string;
function defaultCallSignature(messages: unknown[], modelSettings?: unknown): string;
function workloadInstanceId(): string; // reads SPENDGUARD_WORKLOAD_INSTANCE_ID
```

### 4.7 `withRunPlan` + `currentRunPlan`

TS uses `AsyncLocalStorage` (Node) as the `contextvars.ContextVar` equivalent. Bun + Deno both ship compatible `AsyncLocalStorage`. The decorator is exposed as a **higher-order function**, not a decorator-syntax decorator (TS5 decorators are still ergonomically rough; an HOF works everywhere):

```ts
export function withRunPlan<TArgs extends unknown[], TRet>(
  plan: { plannedCalls: number; plannedTools?: number },
  fn: (...args: TArgs) => TRet | Promise<TRet>,
): (...args: TArgs) => Promise<TRet>;

export function currentRunPlan(): RunPlan | null;
```

Stage-2 syntax (`@withRunPlan(...)` decorator) is added in a v0.2 minor when TS 5 decorators stabilise.

### 4.8 `computePromptHash` — cross-language determinism gate

```ts
export function computePromptHash(promptText: string, tenantId: string): string;
```

**Cross-language byte-for-byte identical** to the Rust sidecar `services/sidecar/src/prompt_hash.rs::compute` and the Python `spendguard.prompt_hash.compute`. Tested by a shared 64-vector fixture (§5.3 of `tests.md`). Drift here breaks audit-chain rule dedup; this is a P0 invariant.

### 4.9 `PricingLookup` — mirror of `pricing.py`

```ts
export type PriceKey = readonly [provider: string, model: string, tokenKind: string];
export type PriceTable = ReadonlyMap<string, number>; // key = `${provider}|${model}|${kind}`
export const USD_MICROS_PER_USD = 1_000_000;

export class PricingLookup {
  constructor(table: PriceTable, opts?: { defaultKind?: string });
  pricePerMillion(provider: string, model: string, tokenKind: string): number | null;
  usdMicrosForCall(args: {
    provider: string;
    model: string;
    inputTokens?: number;
    outputTokens?: number;
    cachedInputTokens?: number;
  }): number;
}
```

The package also ships an **embedded snapshot** of `deploy/demo/init/pricing/seed.yaml` so adapters can `import { DEMO_PRICING } from "@spendguard/sdk/pricing/demo"` and call sidecar in dev without wiring the control-plane pricing fetch. The snapshot is version-pinned by the YAML's `pricing_version` field.

## 5. Configuration

### 5.1 Environment variables

Mirrors Python:

| Env var | Type | Default | Notes |
|---|---|---|---|
| `SPENDGUARD_SIDECAR_UDS` | path | — | If set and `socketPath` not passed, used. |
| `SPENDGUARD_TENANT_ID` | string | — | If set and `tenantId` not passed, used. |
| `SPENDGUARD_WORKLOAD_INSTANCE_ID` | string | `""` | Read by `workloadInstanceId()`. |
| `SPENDGUARD_DECISION_TIMEOUT_MS` | int | 250 | |
| `SPENDGUARD_HANDSHAKE_TIMEOUT_MS` | int | 2000 | |
| `SPENDGUARD_DISABLE` | `"1"` / `"true"` | unset | When set, every method short-circuits to a no-op success. Used in unit test envs where the sidecar isn't available. **Must be advertised as "for tests only" in JSDoc** — production users who set this and forget have silently lost enforcement. |

### 5.2 Explicit config wins. Env is fallback. Both empty + required → throws `SpendGuardConfigError` at constructor.

## 6. Architecture

```
┌────────────────────────────────────────────────────────────────────┐
│ @spendguard/sdk (npm package, ESM-only)                            │
│                                                                    │
│  src/client.ts ──→ generated proto client ──→ @grpc/grpc-js        │
│       │                       │                       │            │
│       │                       │                       UDS          │
│       │                       │              unix:/var/run/        │
│       │                       │              spendguard.sock       │
│       │              ts-proto generated types                      │
│       │              (committed under src/_proto/)                 │
│       │                                                            │
│       ├──→ ids.ts          (newUuid7, deriveIdempotencyKey)       │
│       ├──→ errors.ts       (typed exception hierarchy)            │
│       ├──→ promptHash.ts   (HMAC-SHA256, tenant-keyed)            │
│       ├──→ pricing.ts      (USD-micros computation)               │
│       ├──→ runPlan.ts      (AsyncLocalStorage Signal 3)           │
│       ├──→ otel.ts         (optional span emission)               │
│       └──→ retry.ts        (decision-cache + bounded retry)       │
│                                                                    │
│  src/_proto/spendguard/v1/*.ts  (committed; protobuf-ts output)    │
│                                                                    │
│  tooling: tsup build, vitest tests, biome lint+format              │
└────────────────────────────────────────────────────────────────────┘
                          ▲           ▲           ▲           ▲
                          │           │           │           │
                  @spendguard/langchain   …/vercel-ai   …/openai-agents   …/inngest-agentkit
                  (D04)                   (D06)         (D08)             (D29)
                  peer dep on @spendguard/sdk (caret semver)
```

### 6.1 Toolchain — decision matrix

| Concern | Choice | Rejected | Rationale |
|---|---|---|---|
| Bundler | `tsup` 8.x | `tsc --emitDeclarationOnly` + esbuild | tsup gives one config file for both ESM + .d.ts + subpath entries; the `tsup-shim`-free path matters for our Node 20 / Bun / Deno triple-target.  |
| Test runner | `vitest` 2.x | `node:test` + `bun:test` | Vitest's mock + snapshot APIs are needed for the cross-language fixture diff tests (§5.3 `tests.md`). Works under Bun and Deno via the bundled compat shim. |
| Lint + format | `biome` 1.9+ | `eslint` + `prettier` | Biome is one dep, one config, ~10x faster than eslint, lint+format in one binary. The legacy 2024 eslint debate is settled in 2026: biome owns greenfield TS substrates. Adapters MAY use eslint if their downstream framework expects it; the substrate doesn't. |
| Proto codegen | `protobuf-ts` (`@protobuf-ts/plugin`) + `@grpc/grpc-js` | `ts-proto`, `@bufbuild/protobuf` + `connect-rpc` | protobuf-ts emits idiomatic camelCase TS, has stable BigInt support for NUMERIC(38,0) decimal-string fields (we keep them as strings on the wire, but the BigInt sites in `pricing.ts` need consistent treatment), and integrates cleanly with `@grpc/grpc-js`'s Channel API which is the only mature UDS-capable Node gRPC client. ts-proto's API has churned twice in 18 months; we want stability for a multi-adapter substrate. `@bufbuild/protobuf` + connect-rpc has no UDS support. |
| gRPC transport | `@grpc/grpc-js` (Node) | `@grpc/grpc-js` is the only choice for UDS today. | `nice-grpc`, `connect-rpc`, `bun-grpc` either don't support UDS or aren't production-tested across Bun + Deno. |
| Package format | ESM-only, `"type": "module"`, `"exports"` map | dual CJS+ESM | Adapter targets (D04 / D06 / D08 / D29) and modern Node 20 / Bun / Deno are all ESM-native. CJS adds a `require`-shim maintenance burden and `@grpc/grpc-js` has well-known dual-package hazards. Customers on CJS Node ≤ 18 are not supported. |

### 6.2 Node version policy

- **Node 20.10+** is the floor: needed for `using` / `await using` (TC39 explicit-resource-management), stable `AsyncLocalStorage` perf, and `--experimental-default-type=module` not required.
- Node 22 LTS is the test matrix default.
- Bun 1.1+ tested in CI via a separate matrix shard; Deno 1.46+ same.
- Browser: deliberately not in matrix (UDS-only transport in v0.1.x).

### 6.3 `unix:` URI handling

Match Python's workaround: pass `grpc.default_authority: "localhost"` channel option. Without it, `@grpc/grpc-js` defaults the `:authority` pseudo-header to the URL-encoded UDS path, which `tonic` (the sidecar) rejects with `PROTOCOL_ERROR`. This is the same bug the Python SDK already documented in `client.py:240-251`.

### 6.4 OTel hook semantics

The optional `otelTracer` field receives an `@opentelemetry/api` `Tracer`. The client wraps every RPC in a span named `spendguard.<rpc>`, with attributes:

| Attribute | Type | Value |
|---|---|---|
| `spendguard.tenant_id` | string | `tenantId` |
| `spendguard.decision_id` | string | request's `decisionId` if any |
| `spendguard.trigger` | string | `RUN_PRE` / `AGENT_STEP_PRE` / … |
| `spendguard.outcome.decision` | string | `CONTINUE` / `DEGRADE` / `STOP` / … |
| `spendguard.outcome.reason_codes` | string[] | from `DecisionOutcome.reasonCodes` |
| `spendguard.sdk.version` | string | `VERSION` |

The OTel dep is **`peerDependencyMeta.optional: true`** — adapters that never enable OTel never pay the dep cost. JSDoc on `otelTracer` documents this explicitly.

### 6.5 Retry + bounded backoff

The substrate ships a tiny retry helper for the sidecar-side `UNAVAILABLE` / `DEADLINE_EXCEEDED` / `CANCELLED` cluster, mirroring Python's `_classify_rpc_error`. Defaults:

- Max attempts: 2 (initial + 1 retry).
- Backoff: 25 ms + jitter [0..25 ms]; constant, not exponential, because the timeout is already 250 ms.
- **Idempotency-key-required.** Retry only runs when the caller passed a stable `idempotencyKey` — otherwise a retry creates a fresh decision and the ledger double-reserves. The contract: callers SHOULD derive via `deriveIdempotencyKey(...)`. Without one, retry is a no-op even on `UNAVAILABLE` (and we throw a more pointed `SidecarUnavailable` with `cause` set to the original error so adapters can route).

## 7. Proto codegen pipeline

```
proto/spendguard/**/*.proto
   │
   ▼
sdk/typescript/scripts/proto.ts  (Node script invoked by `pnpm run proto`)
   │ uses @protobuf-ts/plugin
   ▼
sdk/typescript/src/_proto/spendguard/
   ├── common/v1/common.ts
   ├── sidecar_adapter/v1/adapter.ts
   ├── ledger/v1/ledger.ts            (only for the demo's webhook simulator)
   ├── output_predictor_plugin/v1/plugin.ts  (optional — D08's plugin path may import)
   └── tokenizer/v1/tokenizer.ts      (forward-reserved — adapter may pre-tokenize)
```

- The script reads `proto/spendguard/**/*.proto`, runs `protoc` (downloaded via `@protobuf-ts/plugin` deps), pipes through the plugin, writes to `src/_proto/`.
- `src/_proto/` is **committed** (not gitignored) — same policy as `sdk/python/src/spendguard/_proto/`. Reason: customers should not need `protoc` on `pnpm install`; CI verifies determinism with `pnpm run proto && git diff --exit-code src/_proto`.
- The script is idempotent: deterministic output. A drift between proto sources and generated TS surfaces as a CI red.

## 8. Slice plan

| Slice | Title | Files | Size |
|---|---|---|---|
| `COV_S05_01_d05_package_init` | `package.json`, `tsconfig.json`, tsup config, biome config, vitest config, README placeholder, `pnpm-workspace.yaml` carve-out | sdk/typescript/{package.json,tsconfig.json,tsup.config.ts,biome.json,vitest.config.ts} | S |
| `COV_S05_02_d05_proto_codegen` | `scripts/proto.ts`, generated tree under `src/_proto/`, Makefile parity target, CI determinism check | sdk/typescript/scripts/proto.ts, sdk/typescript/src/_proto/**/* | M |
| `COV_S05_03_d05_client_skeleton` | `SpendGuardClient` shell, UDS connection (`@grpc/grpc-js`), `connect/close/asyncDispose`, env-var resolution, config validation | sdk/typescript/src/client.ts | M |
| `COV_S05_04_d05_handshake_reserve_commit` | `handshake()`, `reserve()` (= `requestDecision`), `commitEstimated()` (LLM_CALL_POST single-event path), error mapping, decision-name parsing | sdk/typescript/src/{client.ts,errors.ts} | M |
| `COV_S05_05_d05_release_query` | `release()` (ASP §4), `queryBudget()` placeholder, gRPC Status → typed-error mapping for `FailedPrecondition` cluster | sdk/typescript/src/{client.ts,errors.ts} | S |
| `COV_S05_06_d05_ids_prompt_hash_pricing` | `ids.ts`, `promptHash.ts`, `pricing.ts`, demo pricing snapshot subpath import | sdk/typescript/src/{ids.ts,promptHash.ts,pricing.ts,pricing/demo.ts} | M |
| `COV_S05_07_d05_run_plan` | `withRunPlan`, `currentRunPlan`, `AsyncLocalStorage` context propagation, sync + async parity | sdk/typescript/src/runPlan.ts | S |
| `COV_S05_08_d05_otel_retry_idempotency` | OTel span emission, retry helper, idempotency-key guard, in-process decision cache (same `idempotencyKey` → same outcome within process lifetime) | sdk/typescript/src/{otel.ts,retry.ts,client.ts} | M |
| `COV_S05_09_d05_test_matrix` | vitest unit tests, cross-language `computePromptHash` fixture (vs Python + Rust), `deriveIdempotencyKey` parity test, mock UDS server using `@grpc/grpc-js` server | sdk/typescript/tests/**/* | M |
| `COV_S05_10_d05_publish_pipeline` | npm Trusted Publisher OIDC workflow, `npm pack` size budget check, README + LICENSE_NOTICES, CHANGELOG, version 0.1.0 tag | .github/workflows/sdk-ts-publish.yml, sdk/typescript/{README.md,LICENSE_NOTICES.md,CHANGELOG.md} | S |

Total: **10 slices**, mostly S/M. Hits the build-plan §4 ratio guideline (≤ 50 % S, ≤ 40 % M).

## 9. Locked design decisions

These are decisions where the slice authors do **not** re-litigate.

1. **ESM-only.** No CJS. Customers on legacy CJS Node are not supported.
2. **`reserve()` is the canonical public name**, with `requestDecision` aliased. The Python SDK keeps `request_decision`; we mirror that name as the alias so cross-language docs work, but the v0.1 ASP-aligned vocabulary wins on the canonical surface.
3. **camelCase on the public surface, snake_case on the wire.** The codegen does the mapping. Adapters never see snake_case.
4. **`queryBudget` ships in v0.1.x as a public method but the underlying sidecar RPC is not yet wired** — see §9.4. The TS substrate ships the method signature and a placeholder implementation that throws `SpendGuardError("query_budget not yet wired in sidecar; tracked at <issue>")`. This is intentional: the method is on the public surface so adapter authors can program against it; the sidecar wire is a follow-up slice in `services/sidecar`. This is the one place where the TS surface intentionally precedes the Python surface (Python also lacks `query_budget` in 0.5.1's actual `client.py`).
5. **Biome over ESLint+Prettier.** Substrate-only. Adapters may pick differently.
6. **protobuf-ts over ts-proto.** Stability over feature velocity.
7. **OTel as `peerDependencyMeta.optional: true`**. Adapters that never enable OTel pay zero deps cost.
8. **Tokenizer is OUT of v0.1.x scope.** Python ships embedded Anthropic + Gemini tokenizers as `tiktoken` + `tokenizers`. TS substrate v0.1.x does NOT — adapters are expected to use the egress-proxy-side tokenizer service or the `ClaimEstimate.input_tokens` carry-over. A v0.x slice may add `@dqbd/tiktoken` if a downstream proves it's needed; not blocking D04 / D06 / D08 / D29.
9. **Pricing table embed.** The demo snapshot ships under `@spendguard/sdk/pricing/demo`. The full prod pricing table is fetched from the sidecar handshake (forward-reserved; not in v0.1.x). Demo snapshot is `< 50 KB` and meets the bundle-size budget.
10. **No browser.** UDS transport in v0.1.x. The `runtime: "fetch"` literal type is reserved in the options for a future ASP HTTP gateway transport.

## 10. Bundle-size budget

| Artefact | Budget | Enforced by |
|---|---|---|
| `dist/index.js` (minified ESM) | ≤ 120 KB | `pnpm run size` runs `esbuild --bundle --minify` and asserts. |
| `dist/index.js` (gzipped) | ≤ 35 KB | same |
| Generated proto tree (`src/_proto/`) | ≤ 250 KB unminified | grep-counted in CI |
| `node_modules/@spendguard/sdk` total install | ≤ 5 MB | CI `du -sh` check |

These budgets exist because the substrate is imported into multiple adapter packages each of which is imported into customer agent runtimes that sometimes run in resource-constrained envs (Vercel edge functions, Inngest workers).

## 11. Cross-language determinism gates

The substrate is part of an audit-chain that must produce byte-identical results across Rust (sidecar), Python (existing SDK), and TS (this spec). These three are P0 invariants tested in `tests.md` §5.3:

1. `computePromptHash(text, tenant)` — HMAC-SHA256 lowercase hex, byte-identical to Python + Rust.
2. `deriveIdempotencyKey({tenantId, sessionId, runId, stepId, llmCallId, trigger})` — `"sg-"` + 32 hex chars, byte-identical.
3. `defaultCallSignature(messages, modelSettings)` — same digest given identical canonicalised inputs. (This one is slightly relaxed: the input canonicalisation must be cross-language but the language SDK *uses* its own framework's message types; the canonicalisation rules are documented.)

Shared fixture: `sdk/fixtures/cross-language/` (created by slice S05_09) — 64 vectors. The Python tests already consume it; the Rust tests already consume it; the TS tests will consume it.

## 12. Open questions (locked at spec write)

1. **`@grpc/grpc-js` vs `nice-grpc`:** locked to `@grpc/grpc-js`. nice-grpc has a cleaner API but UDS support was added recently and is not battle-tested across all three runtimes.
2. **CJS publish:** locked to NO. The dual-package hazard with `@grpc/grpc-js` is well-documented.
3. **`pnpm` vs `npm` vs `yarn` in CI:** locked to `pnpm` 9.x (matches the rest of the repo's existing `pnpm-lock.yaml` convention; the docs site uses pnpm).
4. **Telemetry default:** locked to OFF. No anonymous metric send. Customers opt in by passing `otelTracer`.
5. **`npm publish --provenance`:** locked to ON.
6. **Embedded pricing snapshot:** locked to `pricing/seed.yaml` content as of release time; snapshot is regenerated and committed by slice S05_10 as part of the release dance.
7. **Subpath exports tree-shaking validation:** locked to vitest-driven assertion that `import { newUuid7 } from "@spendguard/sdk/ids"` does NOT pull `@grpc/grpc-js` into the bundle (esbuild metadata diff). Failure breaks the publish.
