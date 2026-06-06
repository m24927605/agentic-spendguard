// SpendGuard SDK — configuration surface + runtime validation.
//
// Design contract: design.md §4.2 (`SpendGuardClientOptions` LOCKED) +
// implementation.md §3.2. The slice doc names this file `config.ts` and
// requires:
//   - `SpendGuardClientConfig` interface (the LOCKED §4.2 surface).
//   - `validateConfig(cfg)` runtime validator (tenant_id is UUID, socket path
//     non-empty, etc.).
//
// The SLICE 3 validator is structural — it catches the constructor errors that
// design.md §5.2 mandates (missing required + mutually-exclusive otelTracer +
// onSpan + bogus numeric timeouts). It does NOT yet attempt to `stat()` the
// socket path (the design specifically warns against that in §5.2: the path
// may not exist at construction time because the sidecar starts later in a
// docker-compose dance).
//
// Anti-scope reminder: this file ONLY declares the structural config. Default
// resolution + env merging lives in `client.ts`'s constructor.

import type { Tracer } from "@opentelemetry/api";

import type { IdempotencyCache } from "./cache.js";
import { SpendGuardConfigError } from "./errors.js";

// ── Default deadlines (design.md §4.2) ─────────────────────────────────────

/** Default p99-ceiling for `requestDecision` round-trip; design §4.2. */
export const DEFAULT_DECISION_TIMEOUT_MS = 250 as const;
/** Default handshake deadline; design §4.2. */
export const DEFAULT_HANDSHAKE_TIMEOUT_MS = 2_000 as const;
/** Default deadline for `confirmPublishOutcome` / `release`; design §4.2. */
export const DEFAULT_PUBLISH_TIMEOUT_MS = 150 as const;
/** Default deadline for `emitTraceEvents` ack loop; design §4.2. */
export const DEFAULT_TRACE_TIMEOUT_MS = 500 as const;
/** Default capability the adapter advertises (L3_POLICY_HOOK = 0x40); design §4.2. */
export const DEFAULT_CAPABILITY_LEVEL = 0x40 as const;
/** Default protocol version (design §4.2); only `1` is wire-supported in v0.1.x. */
export const DEFAULT_PROTOCOL_VERSION = 1 as const;

// ── SpanRecord (design.md §3.2 / §6.4) ────────────────────────────────────

/**
 * Lightweight span record passed to the `onSpan` observer hook. The hook
 * receives one record per RPC the client makes. Used by adapters that want
 * to integrate with their own tracing pipeline without pulling
 * `@opentelemetry/api`.
 */
export interface SpanRecord {
  /** RPC span name, e.g. `spendguard.reserve`. */
  name: string;
  /** Span start time in milliseconds since epoch. */
  startTimeMs: number;
  /** Span wall-clock duration in milliseconds. */
  durationMs: number;
  /** Span attributes (snake_case keys per OTel semantic conventions). */
  attributes: Readonly<Record<string, string | number | boolean | undefined>>;
  /** Set when the span completed with an error. */
  error?: Error;
}

// ── SpendGuardClientConfig — LOCKED surface (design.md §4.2) ───────────────

/**
 * Constructor options for `SpendGuardClient`. This shape is part of the
 * LOCKED public surface — design.md §4.2. Adapters in D04 / D06 / D08 / D29
 * build directly against these fields; renames / removals require a v0.minor
 * bump and a coordinated update of every adapter spec.
 *
 * Per design.md §5.2:
 *   - Explicit fields override env fallback.
 *   - Required (no default + no env) fields throw `SpendGuardConfigError` at
 *     constructor time.
 *
 * Two SLICE 3 deviations from design.md §4.2 worth noting:
 *   - `socketPath` is *optional* on this interface even though design §4.2
 *     marks it `required in v0.1.x`. The reason: design §5.2 requires env
 *     fallback to fill it, which only the constructor can do. The validator
 *     enforces presence post-merge.
 *   - `tenantId` is similarly optional here, required post-merge.
 */
export interface SpendGuardClientConfig {
  /** UDS path the sidecar listens on. Env fallback: `SPENDGUARD_SOCKET_PATH` then `SPENDGUARD_SIDECAR_UDS`. */
  socketPath?: string;
  /** Tenant id assertion the sidecar verifies at handshake time. Env fallback: `SPENDGUARD_TENANT_ID`. */
  tenantId?: string;
  /** Runtime kind tag e.g. `"langchain-js"` / `"vercel-ai-sdk"`; default `""`. */
  runtimeKind?: string;
  /** Optional runtime version tag (caller's own SDK version). */
  runtimeVersion?: string;
  /** Override the SDK version reported on the wire. Defaults to the package's `VERSION`. */
  sdkVersion?: string;
  /** Adapter↔sidecar protocol version; only `1` is wire-supported in v0.1.x. */
  protocolVersion?: number;
  /** Capability the adapter advertises; default `0x40` (L3_POLICY_HOOK). */
  capabilityLevel?: number;
  /** Stable workload-instance id; env fallback: `SPENDGUARD_WORKLOAD_INSTANCE_ID`. */
  workloadInstanceId?: string;
  /** Per-decision RPC deadline in ms; default 250; env fallback: `SPENDGUARD_DECISION_TIMEOUT_MS`. */
  decisionTimeoutMs?: number;
  /** Handshake RPC deadline in ms; default 2_000; env fallback: `SPENDGUARD_HANDSHAKE_TIMEOUT_MS`. */
  handshakeTimeoutMs?: number;
  /** Publish-side RPC deadline in ms; default 150. */
  publishTimeoutMs?: number;
  /** Trace-event RPC deadline in ms; default 500. */
  traceTimeoutMs?: number;
  /**
   * Observer hook invoked once per RPC. Mutually exclusive with `otelTracer`
   * (constructor throws `SpendGuardConfigError` when both are set).
   * Wired in SLICE 8.
   */
  onSpan?: (span: SpanRecord) => void;
  /**
   * OTel Tracer. When provided, the client emits spans through it instead of
   * `onSpan`. Mutually exclusive with `onSpan`.
   * `@opentelemetry/api` is a `peerDependenciesMeta.optional` dep — adapters
   * that never enable OTel pay zero dep cost. Wired in SLICE 8.
   */
  otelTracer?: Tracer;
  /**
   * Forward-reserved transport selector. v0.1.x supports only `"uds-grpc"`;
   * the literal type carries the slot for a future ASP HTTP gateway transport
   * (design §9 decision 10) without a v0.minor bump.
   */
  runtime?: "uds-grpc";
  /**
   * Test-only short-circuit: when set, every RPC method returns a no-op
   * outcome without contacting the sidecar. **For tests only** — production
   * users who set this and forget have silently lost enforcement. See
   * design.md §5.1 `SPENDGUARD_DISABLE` for the env fallback. Wired in SLICE 4.
   */
  disabled?: boolean;
  /**
   * Default `run_projection` policy when callers do not pass one. Slice-doc
   * addition; env fallback: `SPENDGUARD_RUN_PROJECTION_DEFAULT`. SLICE 4
   * wires consumption: when set to a non-empty value, the resolved policy
   * is folded into the `DecisionRequest.inputs.runtime_metadata` under the
   * `run_projection_policy` key so the sidecar's projector observes the
   * default per design.md §4.2 R2 amendment.
   */
  runProjectionDefault?: RunProjectionPolicy;
  /**
   * In-process idempotency cache. When set, `reserve()` consults this cache
   * before issuing the sidecar RPC: a hit short-circuits to the cached
   * `DecisionOutcome`. SLICE 8 wires the consumption; see
   * `InMemoryIdempotencyCache` / `NoopIdempotencyCache` from
   * `@spendguard/sdk/cache` for ready-made impls. When `undefined`,
   * `reserve()` issues every RPC unconditionally (the sidecar is still the
   * correctness gate via its own idempotency-key dedup).
   */
  idempotencyCache?: IdempotencyCache;
}

/**
 * Default `run_projection` policy. Forward-extensible string-literal union per
 * design.md §4.2 R2 amendment + COV_S05_03 R2 commitment MJ-1.
 *
 * v0.1.x ships:
 *   - `""` — leave policy unset; rely on contract-bundle default.
 *   - `"STRICT_CEILING"` — ASP Draft-01 strict-ceiling policy.
 *   - `"ELASTIC"` — ASP Draft-01 elastic policy.
 *
 * The third member `(string & {})` is the standard TS literal-string escape
 * hatch: adapters can pass policy names that land in a future contract DSL
 * bump without forcing a v0.minor on the SDK, while preserving completion
 * suggestions for the two named members above.
 */
export type RunProjectionPolicy = "" | "STRICT_CEILING" | "ELASTIC" | (string & {});

/**
 * **LOCKED §4.1 spec name** for the constructor options. Identical to
 * `SpendGuardClientConfig` — this alias preserves the symbol that
 * design.md §4.1 line 45 enumerates as part of the consumer-facing barrel
 * import for D04 / D06 / D08 / D29. The slice doc named the file-internal
 * shape `SpendGuardClientConfig`; the spec name `SpendGuardClientOptions`
 * is the cross-deliverable contract. Both resolve to the same shape so
 * that `import { type SpendGuardClientOptions } from "@spendguard/sdk"`
 * (the form every adapter spec uses) compiles unchanged.
 *
 * Added in COV_S05_03 R2 to close the spec-vs-slice-doc rename drift.
 */
export type SpendGuardClientOptions = SpendGuardClientConfig;

/**
 * Resolved config: every field that the SLICE 3 constructor promises to fill
 * is non-optional here. The runtime validator narrows `SpendGuardClientConfig`
 * to this stricter shape after merging env fallback + defaults.
 */
export interface ResolvedConfig {
  socketPath: string;
  tenantId: string;
  runtimeKind: string;
  runtimeVersion: string;
  sdkVersion: string;
  protocolVersion: number;
  capabilityLevel: number;
  workloadInstanceId: string;
  decisionTimeoutMs: number;
  handshakeTimeoutMs: number;
  publishTimeoutMs: number;
  traceTimeoutMs: number;
  onSpan?: (span: SpanRecord) => void;
  otelTracer?: Tracer;
  idempotencyCache?: IdempotencyCache;
  runtime: "uds-grpc";
  disabled: boolean;
  runProjectionDefault: RunProjectionPolicy;
}

// ── validateConfig — runtime checks (slice doc deliverable) ────────────────

/**
 * Validate a config that has already been merged with env fallback + defaults.
 * Throws `SpendGuardConfigError` on any failure. The errors here are the
 * exact set design.md §5.2 enumerates as constructor-time failures.
 *
 * What we DON'T validate (deliberate):
 *   - Socket path existence on disk — the sidecar may start later in a
 *     docker-compose dance; design §5.2 calls this out explicitly.
 *   - Tenant ID UUID shape — the Python SDK accepts non-UUID tenant ids
 *     (e.g. friendly slugs) for dev environments. We only require a non-empty
 *     string. The cross-language `computePromptHash` canonicalisation
 *     lowercases UUID-shaped tenants but does not reject non-UUID ones.
 */
export function validateConfig(cfg: ResolvedConfig): void {
  if (cfg.socketPath.length === 0) {
    throw new SpendGuardConfigError(
      "socketPath is required (or set SPENDGUARD_SOCKET_PATH / SPENDGUARD_SIDECAR_UDS)",
    );
  }
  if (cfg.tenantId.length === 0) {
    throw new SpendGuardConfigError("tenantId is required (or set SPENDGUARD_TENANT_ID)");
  }
  if (cfg.otelTracer !== undefined && cfg.onSpan !== undefined) {
    throw new SpendGuardConfigError("otelTracer and onSpan are mutually exclusive");
  }
  if (cfg.runtime !== "uds-grpc") {
    throw new SpendGuardConfigError(
      `runtime=${JSON.stringify(cfg.runtime)} is not supported in v0.1.x; only "uds-grpc" is wired`,
    );
  }
  if (cfg.protocolVersion !== 1) {
    throw new SpendGuardConfigError(
      `protocolVersion=${cfg.protocolVersion} is not supported in v0.1.x; only 1 is wired`,
    );
  }
  assertPositiveIntegerField(cfg.capabilityLevel, "capabilityLevel");
  assertPositiveIntegerField(cfg.decisionTimeoutMs, "decisionTimeoutMs");
  assertPositiveIntegerField(cfg.handshakeTimeoutMs, "handshakeTimeoutMs");
  assertPositiveIntegerField(cfg.publishTimeoutMs, "publishTimeoutMs");
  assertPositiveIntegerField(cfg.traceTimeoutMs, "traceTimeoutMs");
}

/** Throws `SpendGuardConfigError` if the field isn't a non-negative finite integer. */
function assertPositiveIntegerField(value: number, field: string): void {
  if (!Number.isFinite(value) || !Number.isInteger(value) || value < 0) {
    throw new SpendGuardConfigError(`${field}=${value} must be a finite non-negative integer`);
  }
}
