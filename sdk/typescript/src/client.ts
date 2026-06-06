// SpendGuard SDK — `SpendGuardClient` skeleton (SLICE 3).
//
// This slice ships the lifecycle surface and the UDS transport wiring; the
// business RPCs (`reserve` / `commitEstimated` / `release` / `queryBudget`)
// land in SLICE 4-5. The class shell here is the contract D04 / D06 / D08 /
// D29 build against — the LOCKED §4.2 surface — so SLICE 4-5 author only the
// RPC bodies, not the class shape.
//
// Spec refs:
//   - design.md §4.2 (LOCKED public surface)
//   - design.md §4.5 (error hierarchy)
//   - design.md §5.1 / §5.2 (env var precedence + validation)
//   - design.md §6.3 (`grpc.default_authority=localhost` for UDS)
//   - implementation.md §4 (skeleton)
//   - slices/COV_S05_03_d05_client_skeleton.md (this slice)
//
// What this slice DOES wire:
//   - Constructor that merges explicit options with env fallback + defaults.
//   - `connect()` → opens a `GrpcTransport` against `unix:<socketPath>` with
//     the `grpc.default_authority=localhost` channel option (the Python SDK's
//     well-documented tonic-compat workaround).
//   - `close()` graceful + idempotent.
//   - `[Symbol.asyncDispose]` for `await using client = new ...`.
//   - `tenantId` / `sessionId` / `handshakeOutcome` getters.
//   - `SpendGuardClient.fromEnv()` factory that defaults `socketPath` to
//     `/var/run/spendguard/adapter.sock` per the slice doc.
//
// What this slice does NOT wire (anti-scope from the slice doc):
//   - `handshake()` body — SLICE 4.
//   - `reserve()` / `commitEstimated()` / `release()` / `queryBudget()` /
//     `confirmPublishOutcome()` / `resumeAfterApproval()` /
//     `safeConfirmApplyFailed()` / `emitLlmCallPost()` business bodies.
//     They are defined but throw `SpendGuardError("...wired in SLICE 4-5")`.
//   - `ids.ts` / `promptHash.ts` / `pricing.ts` (SLICE 6).
//   - `withRunPlan` (SLICE 7).
//   - OTel / retry / idempotency cache (SLICE 8).

import { type ChannelCredentials, credentials as grpcCredentials } from "@grpc/grpc-js";
import type { status as GrpcStatus, ServiceError } from "@grpc/grpc-js";
import { GrpcTransport } from "@protobuf-ts/grpc-transport";

import { SidecarAdapterClient } from "./_proto/spendguard/sidecar_adapter/v1/adapter.client.js";
import {
  DEFAULT_CAPABILITY_LEVEL,
  DEFAULT_DECISION_TIMEOUT_MS,
  DEFAULT_HANDSHAKE_TIMEOUT_MS,
  DEFAULT_PROTOCOL_VERSION,
  DEFAULT_PUBLISH_TIMEOUT_MS,
  DEFAULT_TRACE_TIMEOUT_MS,
  type ResolvedConfig,
  type SpanRecord,
  type SpendGuardClientConfig,
  validateConfig,
} from "./config.js";
import { DEFAULT_SOCKET_PATH, EnvParseError, resolveEnvConfig } from "./env.js";
import {
  HandshakeError,
  SidecarUnavailable,
  SpendGuardConfigError,
  SpendGuardConnectionError,
  SpendGuardError,
} from "./errors.js";
import { VERSION } from "./version.js";

// Suppress the unused-import warning by referencing in JSDoc — these are
// re-exported by the public barrel for adapter convenience.
export {
  DEFAULT_CAPABILITY_LEVEL,
  DEFAULT_DECISION_TIMEOUT_MS,
  DEFAULT_HANDSHAKE_TIMEOUT_MS,
  DEFAULT_PROTOCOL_VERSION,
  DEFAULT_PUBLISH_TIMEOUT_MS,
  DEFAULT_TRACE_TIMEOUT_MS,
};

// ── HandshakeOutcome (design.md §3.2) ─────────────────────────────────────

/**
 * Outcome of a successful handshake. SLICE 4 populates this; SLICE 3 only
 * locks the type so `sessionId` / `handshakeOutcome` getters compile.
 */
export interface HandshakeOutcome {
  sessionId: string;
  sidecarVersion: string;
  schemaBundleId: string;
  schemaBundleHash: Uint8Array;
  contractBundleId: string;
  contractBundleHash: Uint8Array;
  capabilityRequired: number;
  signingKeyId: string;
  announcementSignature: Uint8Array;
}

// ── Forward-declared request / response types from design.md §4.2 ─────────
//
// These shape declarations exist so the SLICE 3 method signatures compile.
// SLICE 4 / SLICE 5 populate the bodies; until then every method throws
// `SpendGuardError` with a clear "wired in SLICE X" message.

export interface UnitRef {
  unit: string;
  denomination: number;
}

export interface BudgetClaim {
  scopeId: string;
  amountAtomic: string;
  unit: UnitRef;
}

export interface PricingFreeze {
  pricingVersion: string;
  pricingHash: Uint8Array;
}

export interface ClaimEstimate {
  tokenizerTier?: "T1" | "T2" | "T3" | "";
  tokenizerVersionId?: string;
  inputTokens?: number | bigint;
  predictedATokens?: number | bigint;
  predictedBTokens?: number | bigint;
  predictedCTokens?: number | bigint;
  reservedStrategy?: "A" | "B" | "C" | "";
  predictionStrategyUsed?: "A" | "B" | "C" | "";
  predictionPolicyUsed?: string;
  predictionConfidence?: number;
  predictionSampleSize?: number | bigint;
  coldStartLayerUsed?: "L1" | "L2" | "L3" | "L4" | "";
  classifierVersion?: string;
  fingerprintVersion?: string;
  promptClassFingerprint?: string;
  runProjectionAtDecisionAtomic?: number | bigint;
  runPredictedRemainingSteps?: number;
  runStepsCompletedSoFar?: number | bigint;
  runCodeTriggered?: string;
  model?: string;
  promptClass?: string;
}

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
  projectedP50Atomic?: string;
  projectedP90Atomic?: string;
  projectedP95Atomic?: string;
  projectedP99Atomic?: string;
  projectedUnit?: UnitRef;
  promptText?: string;
  decisionContextJson?: Record<string, unknown>;
  claimEstimate?: ClaimEstimate;
}

export interface DecisionOutcome {
  decisionId: string;
  auditDecisionEventId: string;
  decision: "CONTINUE" | "DEGRADE";
  mutationPatchJson: string;
  effectHash: Uint8Array;
  ledgerTransactionId: string;
  reservationIds: readonly string[];
  ttlExpiresAtSeconds: number;
  reasonCodes: readonly string[];
  matchedRuleIds: readonly string[];
}

export interface CommitEstimatedRequest {
  runId: string;
  stepId: string;
  llmCallId: string;
  decisionId: string;
  reservationId: string;
  estimatedAmountAtomic: string;
  unit: UnitRef;
  pricing: PricingFreeze;
  providerEventId: string;
  outcome: "SUCCESS" | "PROVIDER_ERROR" | "CLIENT_TIMEOUT" | "RUN_ABORTED";
  actualInputTokens?: number;
  actualOutputTokens?: number;
  deltaBRatio?: number;
  deltaCRatio?: number;
  traceparent?: string;
  tracestate?: string;
  providerResponseMetadata?: string;
}

export interface ReleaseRequest {
  reservationId: string;
  idempotencyKey: string;
  reasonCodes?: readonly string[];
  workloadInstanceId?: string;
  tenantId?: string;
}

export interface ReleaseOutcome {
  auditEventSignature: Uint8Array;
  ledgerTransactionId: string;
  releasedReservationIds: readonly string[];
}

export interface QueryBudgetRequest {
  scopeId: string;
  asOfSeconds?: number;
}

export interface QueryBudgetResult {
  availableAtomic: string;
  reservedAtomic: string;
  committedAtomic: string;
  unit: UnitRef;
  asOfSeconds: number;
}

export interface PublishOutcomeRequest {
  decisionId: string;
  effectHash: Uint8Array;
  outcome:
    | "APPLIED"
    | "APPLIED_NOOP"
    | "APPLY_FAILED"
    | "APPROVAL_GRANTED"
    | "APPROVAL_DENIED"
    | "APPROVAL_TIMED_OUT";
  adapterError?: string;
}

export interface ApplyFailedRequest {
  decisionId: string;
  effectHash: Uint8Array;
  adapterError: string;
}

export interface ResumeAfterApprovalRequest {
  approvalId: string;
  tenantId: string;
  decisionId: string;
  workloadInstanceId?: string;
}

/** Alias for `CommitEstimatedRequest` exposed for the lower-level entry point. */
export type EmitLlmCallPostRequest = CommitEstimatedRequest;

// ── SLICE 3 placeholder marker ────────────────────────────────────────────

const SLICE_4_5_NOT_WIRED =
  "not yet wired — SLICE 4-5 (see docs/slices/COV_S05_04_d05_handshake_reserve_commit.md)";

// ── SpendGuardClient ──────────────────────────────────────────────────────

/**
 * Async gRPC client for the SpendGuard sidecar over a Unix Domain Socket.
 *
 * SLICE 3 wires lifecycle + UDS transport; SLICE 4-5 wire the RPC bodies.
 * Adapters (D04 / D06 / D08 / D29) build against the LOCKED §4.2 surface
 * and treat this class as their primary integration point.
 *
 * Usage (SLICE 4+, once RPCs are wired):
 *
 *     await using client = SpendGuardClient.fromEnv();
 *     await client.connect();
 *     const handshake = await client.handshake();
 *     const decision = await client.reserve({ ... });
 *     await client.commitEstimated({ ... });
 *     // `await using` runs `[Symbol.asyncDispose]` here → graceful close
 *
 * @example Test-only short-circuit (per design.md §5.1)
 *
 *     // SPENDGUARD_DISABLE=1 in env, or:
 *     const client = new SpendGuardClient({
 *       socketPath: "/dev/null",
 *       tenantId: "test",
 *       disabled: true,
 *     });
 *     // Every RPC returns a no-op outcome; no UDS contact. **TESTS ONLY** —
 *     // a forgotten production setting silently loses enforcement.
 */
export class SpendGuardClient implements AsyncDisposable {
  /** Frozen, merged + validated configuration. */
  private readonly cfg: ResolvedConfig;
  /** Active gRPC transport, or `null` before `connect()` / after `close()`. */
  private transport: GrpcTransport | null = null;
  /** Active SidecarAdapter gRPC client; mirrors `transport` lifetime. */
  private adapterClient: SidecarAdapterClient | null = null;
  /** Cached handshake outcome; SLICE 4 fills it. */
  private handshakeResult: HandshakeOutcome | null = null;

  /**
   * Construct a client. Per design.md §5.2: explicit options win over env
   * fallback; required fields without either throw `SpendGuardConfigError`
   * immediately.
   */
  constructor(rawOpts: SpendGuardClientConfig = {}) {
    let envSnapshot: import("./env.js").ResolvedEnvConfig;
    try {
      envSnapshot = resolveEnvConfig();
    } catch (err) {
      if (err instanceof EnvParseError) {
        throw new SpendGuardConfigError(err.message);
      }
      throw err;
    }

    const socketPath = rawOpts.socketPath ?? envSnapshot.socketPath ?? "";
    const tenantId = rawOpts.tenantId ?? envSnapshot.tenantId ?? "";

    const cfg: ResolvedConfig = {
      socketPath,
      tenantId,
      runtimeKind: rawOpts.runtimeKind ?? "",
      runtimeVersion: rawOpts.runtimeVersion ?? "",
      sdkVersion: rawOpts.sdkVersion ?? VERSION,
      protocolVersion: rawOpts.protocolVersion ?? DEFAULT_PROTOCOL_VERSION,
      capabilityLevel: rawOpts.capabilityLevel ?? DEFAULT_CAPABILITY_LEVEL,
      workloadInstanceId: rawOpts.workloadInstanceId ?? envSnapshot.workloadInstanceId ?? "",
      decisionTimeoutMs:
        rawOpts.decisionTimeoutMs ?? envSnapshot.decisionTimeoutMs ?? DEFAULT_DECISION_TIMEOUT_MS,
      handshakeTimeoutMs:
        rawOpts.handshakeTimeoutMs ??
        envSnapshot.handshakeTimeoutMs ??
        DEFAULT_HANDSHAKE_TIMEOUT_MS,
      publishTimeoutMs: rawOpts.publishTimeoutMs ?? DEFAULT_PUBLISH_TIMEOUT_MS,
      traceTimeoutMs: rawOpts.traceTimeoutMs ?? DEFAULT_TRACE_TIMEOUT_MS,
      runtime: rawOpts.runtime ?? "uds-grpc",
      disabled: rawOpts.disabled ?? envSnapshot.disabled ?? false,
      runProjectionDefault: rawOpts.runProjectionDefault ?? envSnapshot.runProjectionDefault ?? "",
    };
    if (rawOpts.onSpan !== undefined) {
      cfg.onSpan = rawOpts.onSpan;
    }
    if (rawOpts.otelTracer !== undefined) {
      cfg.otelTracer = rawOpts.otelTracer;
    }

    validateConfig(cfg);
    this.cfg = Object.freeze(cfg);
  }

  /**
   * Convenience factory that reads required config from env vars and falls
   * back to `/var/run/spendguard/adapter.sock` when `SPENDGUARD_SOCKET_PATH`
   * is unset.
   *
   * Env vars consumed:
   *   - `SPENDGUARD_SOCKET_PATH` (slice-doc alias) / `SPENDGUARD_SIDECAR_UDS`
   *     (design §5.1) — UDS path; defaults to `/var/run/spendguard/adapter.sock`.
   *   - `SPENDGUARD_TENANT_ID` — **required**; throws `SpendGuardConfigError`
   *     when unset.
   *   - `SPENDGUARD_RUN_PROJECTION_DEFAULT` — optional default
   *     `run_projection` policy name; SLICE 4 wires consumption.
   *   - `SPENDGUARD_WORKLOAD_INSTANCE_ID` / `SPENDGUARD_DECISION_TIMEOUT_MS`
   *     / `SPENDGUARD_HANDSHAKE_TIMEOUT_MS` / `SPENDGUARD_DISABLE` — optional
   *     per design §5.1.
   *
   * Extra options provided as the `overrides` argument win over env per
   * design.md §5.2.
   *
   * @throws SpendGuardConfigError when `SPENDGUARD_TENANT_ID` is missing.
   */
  static fromEnv(overrides: SpendGuardClientConfig = {}): SpendGuardClient {
    let envSnapshot: import("./env.js").ResolvedEnvConfig;
    try {
      envSnapshot = resolveEnvConfig();
    } catch (err) {
      if (err instanceof EnvParseError) {
        throw new SpendGuardConfigError(err.message);
      }
      throw err;
    }
    const resolvedTenant = overrides.tenantId ?? envSnapshot.tenantId;
    const merged: SpendGuardClientConfig = {
      socketPath: overrides.socketPath ?? envSnapshot.socketPath ?? DEFAULT_SOCKET_PATH,
      ...(resolvedTenant !== undefined ? { tenantId: resolvedTenant } : {}),
      ...(envSnapshot.workloadInstanceId !== undefined
        ? { workloadInstanceId: envSnapshot.workloadInstanceId }
        : {}),
      ...(envSnapshot.runProjectionDefault !== undefined
        ? { runProjectionDefault: envSnapshot.runProjectionDefault }
        : {}),
      ...(envSnapshot.decisionTimeoutMs !== undefined
        ? { decisionTimeoutMs: envSnapshot.decisionTimeoutMs }
        : {}),
      ...(envSnapshot.handshakeTimeoutMs !== undefined
        ? { handshakeTimeoutMs: envSnapshot.handshakeTimeoutMs }
        : {}),
      ...(envSnapshot.disabled !== undefined ? { disabled: envSnapshot.disabled } : {}),
      ...overrides,
    };
    return new SpendGuardClient(merged);
  }

  // ── Lifecycle ────────────────────────────────────────────────────────────

  /**
   * Open the UDS gRPC channel. Idempotent — a second call when already
   * connected is a no-op.
   *
   * Per design.md §6.3 / Python `client.py:240-251`, the `unix:` URI scheme
   * is used and `grpc.default_authority=localhost` is set so the tonic-based
   * sidecar accepts the HTTP/2 `:authority` pseudo-header. Without this
   * channel option, tonic resets every stream with `PROTOCOL_ERROR`.
   *
   * In disabled mode (`SPENDGUARD_DISABLE=1` or `disabled: true`), no
   * transport is opened — the call returns immediately. Subsequent RPCs
   * short-circuit to no-op outcomes in SLICE 4.
   *
   * @throws SpendGuardConnectionError when the underlying transport could
   *   not be opened (e.g. malformed socket path).
   */
  async connect(): Promise<void> {
    if (this.cfg.disabled) return;
    if (this.transport !== null) return;
    const target = `unix:${this.cfg.socketPath}`;
    try {
      this.transport = new GrpcTransport({
        host: target,
        channelCredentials: this.buildChannelCredentials(),
        clientOptions: {
          // tonic-compat: see design §6.3 + Python `client.py:240-251`.
          "grpc.default_authority": "localhost",
          // v1 message ceiling per review-standards §6.3 (≥ 4 MiB).
          "grpc.max_receive_message_length": 4 * 1024 * 1024,
          "grpc.max_send_message_length": 4 * 1024 * 1024,
        },
      });
      this.adapterClient = new SidecarAdapterClient(this.transport);
    } catch (err) {
      // GrpcTransport's constructor is largely synchronous; failure usually
      // means a malformed URI. Surface as a typed connection error rather
      // than letting the underlying string-typed throw escape.
      this.transport = null;
      this.adapterClient = null;
      throw new SpendGuardConnectionError(
        `failed to open UDS transport to ${target}: ${errorMessage(err)}`,
        { cause: err },
      );
    }
  }

  /**
   * Graceful close. Idempotent — calling `close()` twice (or `close()` before
   * `connect()`) does not throw.
   *
   * On success the transport is dropped; the next `connect()` allocates a new
   * channel. In-flight RPCs may complete with a `CANCELLED` status; SLICE 8
   * adds the grace-period drain semantics that mirror the Python SDK's
   * `await ch.close(grace=0.5)` path.
   */
  async close(): Promise<void> {
    const t = this.transport;
    this.transport = null;
    this.adapterClient = null;
    if (t === null) return;
    try {
      t.close();
    } catch {
      // Closing a half-broken transport is fine; we own the cleanup and the
      // gRPC channel will be GC'd. Matches the Python SDK's `except Exception`
      // in `close()`.
    }
  }

  /**
   * ESM 2024 `await using` hook. Equivalent to `await this.close()`.
   *
   * Usage:
   *
   *     await using client = new SpendGuardClient({ ... });
   *     // ... use client ...
   *     // [Symbol.asyncDispose] runs here automatically.
   */
  async [Symbol.asyncDispose](): Promise<void> {
    await this.close();
  }

  // ── Read-only state ──────────────────────────────────────────────────────

  /** The tenant id this client asserted at construction. Stable for the client's lifetime. */
  get tenantId(): string {
    return this.cfg.tenantId;
  }

  /**
   * The negotiated session id. Throws `HandshakeError` until `handshake()`
   * has completed (SLICE 4 wires the handshake; until then the getter is
   * effectively unusable, which is intentional — adapters should call
   * `handshake()` before reading state).
   *
   * @throws HandshakeError before `handshake()` completes.
   */
  get sessionId(): string {
    if (this.handshakeResult === null) {
      throw new HandshakeError("handshake() has not completed; sessionId is not yet known");
    }
    return this.handshakeResult.sessionId;
  }

  /** Full handshake outcome. Throws `HandshakeError` until `handshake()` completes. */
  get handshakeOutcome(): HandshakeOutcome {
    if (this.handshakeResult === null) {
      throw new HandshakeError("handshake() has not completed");
    }
    return this.handshakeResult;
  }

  /** Whether the client is currently connected to the sidecar. */
  get isConnected(): boolean {
    return this.transport !== null;
  }

  /** Frozen view of the resolved configuration. Useful for tests + debugging. */
  get config(): Readonly<ResolvedConfig> {
    return this.cfg;
  }

  // ── Handshake (body lands in SLICE 4) ───────────────────────────────────

  /**
   * Mandatory initial handshake. SLICE 4 wires the body; SLICE 3 throws so
   * adapter tests that need a real handshake know to wait for SLICE 4.
   *
   * @throws SpendGuardError until SLICE 4 wires the body.
   */
  async handshake(opts: { workloadInstanceId?: string } = {}): Promise<HandshakeOutcome> {
    void opts;
    throw new SpendGuardError(`handshake() ${SLICE_4_5_NOT_WIRED}`);
  }

  // ── Core RPC surface (bodies land in SLICE 4-5) ─────────────────────────

  /**
   * Run a `*.pre` decision boundary through the sidecar. Equivalent to the
   * Python SDK's `request_decision`. SLICE 4 wires the body.
   *
   * @throws SpendGuardError until SLICE 4 wires the body.
   */
  async reserve(req: ReserveRequest): Promise<DecisionOutcome> {
    void req;
    throw new SpendGuardError(`reserve() ${SLICE_4_5_NOT_WIRED}`);
  }

  /**
   * Wrapper that will become an alias in SLICE 4 (review-standards §1.5).
   *
   * Design §4.2 line 155 LOCKS `reserve === requestDecision` as identical
   * function references. SLICE 3 ships a thin wrapper because the body of
   * `reserve()` is itself a placeholder — making the symbols identical now
   * yields no observable contract benefit (both still throw the SLICE 4-5
   * marker). SLICE 4 replaces this declaration with an instance-field
   * initializer `readonly requestDecision = this.reserve.bind(this);` so
   * `client.reserve === client.requestDecision` holds at runtime and the
   * acceptance §1.5 identity assertion passes.
   *
   * Acceptance §1.5 explicitly authorizes this SLICE 3 → SLICE 4 deferral.
   *
   * @throws SpendGuardError until SLICE 4 wires the body.
   */
  async requestDecision(req: ReserveRequest): Promise<DecisionOutcome> {
    // SLICE 4 will rewrite this body and replace with
    // `readonly requestDecision = this.reserve.bind(this);` instance-field
    // initializer per design §4.2 + review-standards §1.5. Until then the
    // wrapper proxies to `reserve()` so callers using the Python-named
    // symbol see the same "SLICE 4-5 not yet wired" surface.
    return this.reserve(req);
  }

  /**
   * Commit an estimated LLM-call outcome. Equivalent to the Python SDK's
   * `emit_llm_call_post` with `estimated_amount_atomic`. SLICE 4 wires body.
   *
   * @throws SpendGuardError until SLICE 4 wires the body.
   */
  async commitEstimated(req: CommitEstimatedRequest): Promise<void> {
    void req;
    throw new SpendGuardError(`commitEstimated() ${SLICE_4_5_NOT_WIRED}`);
  }

  /**
   * Explicit release of a held reservation. Matches Agent Spend Protocol
   * Draft-01 §4. SLICE 5 wires the body.
   *
   * @throws SpendGuardError until SLICE 5 wires the body.
   */
  async release(req: ReleaseRequest): Promise<ReleaseOutcome> {
    void req;
    throw new SpendGuardError(`release() ${SLICE_4_5_NOT_WIRED}`);
  }

  /**
   * Read-only budget snapshot. Locked decision #4 of design.md §9: in v0.1.x
   * the substrate ships the method signature but the sidecar wire is a
   * follow-up. SLICE 5 wires the request envelope; the sidecar RPC itself
   * may still be a placeholder.
   *
   * @throws SpendGuardError until SLICE 5 wires the body.
   */
  async queryBudget(req: QueryBudgetRequest): Promise<QueryBudgetResult> {
    void req;
    throw new SpendGuardError(`queryBudget() ${SLICE_4_5_NOT_WIRED}`);
  }

  // ── Lower-level surface (bodies land in SLICE 4-5) ──────────────────────

  /**
   * Confirm `publish_effect` outcome. SLICE 4 wires body.
   * @throws SpendGuardError until SLICE 4 wires the body.
   */
  async confirmPublishOutcome(req: PublishOutcomeRequest): Promise<string> {
    void req;
    throw new SpendGuardError(`confirmPublishOutcome() ${SLICE_4_5_NOT_WIRED}`);
  }

  /**
   * Resume after a human approver acted on a `REQUIRE_APPROVAL` decision.
   * SLICE 4 wires the body; references this method from `ApprovalRequired.resume`.
   * @throws SpendGuardError until SLICE 4 wires the body.
   */
  async resumeAfterApproval(req: ResumeAfterApprovalRequest): Promise<DecisionOutcome> {
    void req;
    throw new SpendGuardError(`resumeAfterApproval() ${SLICE_4_5_NOT_WIRED}`);
  }

  /**
   * Safe-ack the `APPLY_FAILED` publish outcome — swallows transport errors
   * so the caller's original exception is never shadowed. SLICE 4 wires body.
   * @throws SpendGuardError until SLICE 4 wires the body.
   */
  async safeConfirmApplyFailed(req: ApplyFailedRequest): Promise<void> {
    void req;
    throw new SpendGuardError(`safeConfirmApplyFailed() ${SLICE_4_5_NOT_WIRED}`);
  }

  /**
   * Lower-level entry point that `commitEstimated()` wraps. Provided so
   * adapters that need the raw trace-event surface have access. SLICE 4
   * wires body.
   * @throws SpendGuardError until SLICE 4 wires the body.
   */
  async emitLlmCallPost(req: EmitLlmCallPostRequest): Promise<void> {
    void req;
    throw new SpendGuardError(`emitLlmCallPost() ${SLICE_4_5_NOT_WIRED}`);
  }

  // ── Internals ────────────────────────────────────────────────────────────

  /**
   * Build the gRPC channel credentials. Always insecure over UDS — the kernel
   * `SO_PEERCRED` check on the sidecar side is the trust anchor (Sidecar
   * Architecture §5). TLS over a Unix socket adds overhead with no security
   * benefit when the connection is implicitly local.
   *
   * Carved into its own method so SLICE 4 can override under a `runtime` flag
   * once HTTP-gateway transport is added — `runtime: "fetch"` would override
   * to use TLS credentials. v0.1.x only supports `"uds-grpc"`.
   */
  private buildChannelCredentials(): ChannelCredentials {
    return grpcCredentials.createInsecure();
  }
}

// ── Helpers ────────────────────────────────────────────────────────────────

/** Pull a string message out of an unknown thrown value. */
function errorMessage(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === "string") return err;
  try {
    return JSON.stringify(err);
  } catch {
    return String(err);
  }
}

// ── Forward-reference type imports ─────────────────────────────────────────
//
// `ServiceError` / `status` are imported as types-only above so SLICE 4 can
// switch them to runtime imports without touching this file. The reference
// here keeps verbatimModuleSyntax happy.
export type { ServiceError, GrpcStatus };
// Re-export the SpanRecord type for adapters that wire `onSpan`.
export type { SpanRecord };

// ── LOCKED §4.1 subpath contract — `@spendguard/sdk/client` ───────────────
//
// design.md §4.1 line 93: `@spendguard/sdk/client | SpendGuardClient + its
// types only.` The class is exported as `SpendGuardClient` above; the
// config shapes that constructor + read-only `config` getter return must
// also be reachable through this subpath so adapters importing only the
// client surface (D04 / D06 / D08 / D29 tree-shaking branch) do not have
// to also pull the full `@spendguard/sdk` barrel for the option type.
//
// `SpendGuardClientOptions` is the LOCKED §4.1 spec name (alias of
// `SpendGuardClientConfig`); both ship so the slice-doc-internal name and
// the spec name resolve here. Added in COV_S05_03 R2 to close the
// subpath-vs-spec contract gap caught by R1 B-2.
export type {
  SpendGuardClientConfig,
  SpendGuardClientOptions,
  ResolvedConfig,
} from "./config.js";
