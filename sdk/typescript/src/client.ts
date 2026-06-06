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
import { RpcError } from "@protobuf-ts/runtime-rpc";

import { SidecarAdapterClient } from "./_proto/spendguard/sidecar_adapter/v1/adapter.client.js";
import type {
  DecisionRequest as ProtoDecisionRequest,
  DecisionResponse as ProtoDecisionResponse,
  HandshakeRequest as ProtoHandshakeRequest,
  HandshakeResponse as ProtoHandshakeResponse,
  TraceEvent as ProtoTraceEvent,
  TraceEventAck as ProtoTraceEventAck,
} from "./_proto/spendguard/sidecar_adapter/v1/adapter.js";
import {
  DecisionRequest_Trigger,
  DecisionResponse_Decision,
  LlmCallPostPayload_Outcome,
  TraceEventAck_Status,
  TraceEvent_EventKind,
} from "./_proto/spendguard/sidecar_adapter/v1/adapter.js";
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
  ApprovalRequired,
  DecisionDenied,
  DecisionSkipped,
  DecisionStopped,
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

// ── SLICE 5 placeholder marker ────────────────────────────────────────────

const SLICE_5_NOT_WIRED =
  "not yet wired — SLICE 5 (see docs/slices/COV_S05_05_d05_release_query.md)";

// ── SpendGuardClient ──────────────────────────────────────────────────────

/**
 * Async gRPC client for the SpendGuard sidecar over a Unix Domain Socket.
 *
 * SLICE 3 wired lifecycle + UDS transport; SLICE 4 wires handshake / reserve /
 * commitEstimated bodies; SLICE 5 wires release / queryBudget. Adapters (D04 /
 * D06 / D08 / D29) build against the LOCKED §4.2 surface and treat this class
 * as their primary integration point.
 *
 * Usage:
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
  /** Cached handshake outcome; first `handshake()` populates, subsequent reads reuse. */
  private handshakeResult: HandshakeOutcome | null = null;
  /**
   * Coalesces concurrent `handshake()` callers into a single in-flight RPC.
   * Mirrors Python `self._handshake_lock` in `client.py`. Stays non-null only
   * while a handshake RPC is pending; cleared after success or failure so a
   * post-failure retry can re-enter.
   */
  private handshakeInFlight: Promise<HandshakeOutcome> | null = null;

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

  // ── Handshake (design.md §4.5 lifecycle) ────────────────────────────────

  /**
   * Mandatory initial handshake. Idempotent — a second call returns the cached
   * outcome without re-issuing the RPC (design.md §4.5). Concurrent callers
   * are coalesced into the same in-flight RPC via `handshakeInFlight`.
   *
   * Disabled mode (`SPENDGUARD_DISABLE=1` / `disabled: true`) short-circuits
   * to a synthetic `HandshakeOutcome` so unit tests can run without a real
   * sidecar (`makeDisabledHandshake`).
   *
   * @throws HandshakeError on protocol-version mismatch or insufficient
   *   capability advertisement.
   * @throws SidecarUnavailable on UNAVAILABLE / DEADLINE_EXCEEDED / CANCELLED.
   * @throws SpendGuardError on any other gRPC failure surface.
   */
  async handshake(opts: { workloadInstanceId?: string } = {}): Promise<HandshakeOutcome> {
    if (this.cfg.disabled) {
      // Memoize the disabled-mode outcome so `client.sessionId` / repeated
      // `handshake()` calls resolve identically to the real-mode contract.
      if (this.handshakeResult === null) {
        this.handshakeResult = makeDisabledHandshake();
      }
      return this.handshakeResult;
    }
    if (this.handshakeResult !== null) return this.handshakeResult;
    if (this.handshakeInFlight !== null) return this.handshakeInFlight;

    const promise = this.doHandshake(opts).finally(() => {
      this.handshakeInFlight = null;
    });
    this.handshakeInFlight = promise;
    return promise;
  }

  /**
   * Internal: issue the real Handshake RPC and map the response.
   *
   * Splits out from `handshake()` so the idempotency guard there stays
   * obviously correct: `doHandshake` never reads `handshakeResult` itself;
   * it only writes it on success.
   */
  private async doHandshake(opts: {
    workloadInstanceId?: string;
  }): Promise<HandshakeOutcome> {
    if (this.adapterClient === null) {
      await this.connect();
    }
    const adapter = this.adapterClient;
    if (adapter === null) {
      // connect() in disabled mode short-circuits without setting adapterClient,
      // but disabled is handled above; reaching here means the transport open
      // failed but didn't throw — defensive guard for the type narrow.
      throw new SidecarUnavailable("transport not established for handshake");
    }
    const workloadInstanceId = opts.workloadInstanceId ?? this.cfg.workloadInstanceId ?? "";
    const req: ProtoHandshakeRequest = {
      sdkVersion: this.cfg.sdkVersion,
      runtimeKind: this.cfg.runtimeKind,
      runtimeVersion: this.cfg.runtimeVersion,
      // Wire-level enum uses the same numeric value as cfg.capabilityLevel
      // (DEFAULT_CAPABILITY_LEVEL = 0x40 = L3_POLICY_HOOK = 64).
      capabilityLevel: this.cfg.capabilityLevel,
      tenantIdAssertion: this.cfg.tenantId,
      workloadInstanceId,
      protocolVersion: this.cfg.protocolVersion,
    };
    let resp: ProtoHandshakeResponse;
    try {
      resp = await adapter.handshake(req, {
        timeout: this.cfg.handshakeTimeoutMs,
      }).response;
    } catch (err) {
      throw classifyRpcError(err, "handshake");
    }
    if (resp.protocolVersion !== this.cfg.protocolVersion) {
      throw new HandshakeError(
        `protocol version mismatch: adapter=${this.cfg.protocolVersion} sidecar=${resp.protocolVersion}`,
      );
    }
    const outcome: HandshakeOutcome = {
      sessionId: resp.sessionId,
      sidecarVersion: resp.sidecarVersion,
      schemaBundleId: resp.schemaBundle?.schemaBundleId ?? "",
      schemaBundleHash: resp.schemaBundle?.schemaBundleHash ?? new Uint8Array(),
      contractBundleId: resp.contractBundle?.bundleId ?? "",
      contractBundleHash: resp.contractBundle?.bundleHash ?? new Uint8Array(),
      capabilityRequired: Number(resp.capabilityRequired ?? 0),
      signingKeyId: resp.signingKeyId,
      announcementSignature: resp.announcementSignature ?? new Uint8Array(),
    };
    if (outcome.capabilityRequired > this.cfg.capabilityLevel) {
      throw new HandshakeError(
        `sidecar requires capability ${toHex(outcome.capabilityRequired)} but adapter advertised ${toHex(this.cfg.capabilityLevel)}; refusing`,
      );
    }
    this.handshakeResult = outcome;
    return outcome;
  }

  // ── Core RPC surface (handshake / reserve / commitEstimated wired) ───────

  /**
   * Run a `*.pre` decision boundary through the sidecar. Equivalent to the
   * Python SDK's `request_decision` (design.md §4.7).
   *
   * The wire shape is built in `buildDecisionRequest()` and consumes:
   *   - the cached handshake `sessionId` (auto-handshakes on first use),
   *   - the caller-supplied `idempotencyKey` (REQUIRED — see design §6.5),
   *   - `runProjectionDefault` from config when the caller did not pass
   *     one in `decisionContextJson.run_projection_policy` (closes MJ-1).
   *
   * The response is mapped through `mapDecisionResponse()`: CONTINUE / DEGRADE
   * return a `DecisionOutcome`; STOP / STOP_RUN_PROJECTION / SKIP /
   * REQUIRE_APPROVAL raise the matching typed exception so adapters can route
   * on `instanceof DecisionDenied` (and its subclasses) per review-standards §5.
   *
   * @throws DecisionStopped on STOP / STOP_RUN_PROJECTION.
   * @throws DecisionSkipped on SKIP.
   * @throws ApprovalRequired on REQUIRE_APPROVAL — `await err.resume(client)`
   *   surfaces the operator decision.
   * @throws DecisionDenied on an unknown decision enum.
   * @throws SidecarUnavailable on UNAVAILABLE / DEADLINE_EXCEEDED / CANCELLED.
   * @throws SpendGuardError on any other gRPC failure surface.
   */
  async reserve(req: ReserveRequest): Promise<DecisionOutcome> {
    if (this.cfg.disabled) return makeDisabledDecision(req);
    if (this.handshakeResult === null) {
      // Mirror Python: callers that forget `await client.handshake()` get a
      // clear typed error rather than a cryptic UNAUTHENTICATED status from
      // the sidecar.
      throw new HandshakeError(
        "reserve() requires handshake(); call await client.handshake() before reserve()",
      );
    }
    if (this.adapterClient === null) {
      await this.connect();
    }
    const adapter = this.adapterClient;
    if (adapter === null) {
      throw new SidecarUnavailable("transport not established for reserve");
    }
    const grpcReq = this.buildDecisionRequest(req);
    let resp: ProtoDecisionResponse;
    try {
      resp = await adapter.requestDecision(grpcReq, {
        timeout: this.cfg.decisionTimeoutMs,
      }).response;
    } catch (err) {
      throw classifyRpcError(err, "reserve");
    }
    if (resp.error && resp.error.code !== 0) {
      throw new SpendGuardError(
        `sidecar error code=${resp.error.code} message=${resp.error.message}`,
      );
    }
    return mapDecisionResponse(resp, this.cfg.tenantId);
  }

  /**
   * Alias for `reserve()` — identical function reference (review-standards §1.5
   * P0 BLOCKER). The Python SDK exposes the symbol as `request_decision`; the
   * TS surface keeps `reserve` as the canonical name AND exposes
   * `requestDecision` so cross-language docs work without surprise.
   *
   * Implemented as an instance-field initializer that reads `this.reserve`
   * during construction. The dot-lookup on `this.reserve` (inside the field
   * initializer, before any instance shadow exists) resolves to the prototype
   * method `SpendGuardClient.prototype.reserve`. Assigning it to the field
   * makes `client.requestDecision === client.reserve` Boolean-true at runtime
   * (both resolve to the same prototype function reference).
   *
   * NOTE on `.bind(this)`: implementation.md §4 line 581 sketches the field
   * as `this.reserve.bind(this)`. The literal `bind` would produce a NEW
   * function object and break the §1.5 identity gate; the constraint cited
   * by the slice doc (review-standards §1.5 P0 BLOCKER) wins, so we drop
   * `.bind(this)`. Callers always invoke as `client.requestDecision(req)`
   * (method-call form) which preserves `this` via JS dispatch semantics —
   * the bind was over-specification for the Pythonic detached-method
   * pattern, which the TS SDK does not advertise.
   *
   * NOTE: do NOT add a JSDoc `@throws` block here — TypeScript erases JSDoc
   * from runtime fields and the identity invariant is the primary contract
   * this declaration enforces.
   */
  readonly requestDecision: SpendGuardClient["reserve"] = this.reserve;

  /**
   * Commit an estimated LLM-call outcome. Equivalent to the Python SDK's
   * `emit_llm_call_post` with `estimated_amount_atomic` (design.md §4.8).
   *
   * Single-event LlmCallPostPayload over the EmitTraceEvents duplex stream:
   * the client opens a fresh stream per commit, sends one event, awaits one
   * ack, and closes (Python parity — `emit_llm_call_post` at client.py:818).
   * SLICE 5+ may switch to a long-lived stream for production latency, but
   * the per-event setup cost is acceptable in v0.1.x.
   *
   * Ack semantics: the sidecar emits exactly one `TraceEventAck` per inbound
   * event in this POC. Status != ACCEPTED surfaces as `SpendGuardError`
   * (Codex round-2 P1.1 from Python parity — silent failure here would mask
   * a commit-lifecycle bug).
   *
   * Mutually exclusive with the deferred provider-report path: this method
   * always sends `estimated_amount_atomic`; the `provider_reported_amount_atomic`
   * wire field stays empty. Adapters needing the provider-report path use the
   * lower-level `emitLlmCallPost` (SLICE 5+).
   *
   * @throws SidecarUnavailable on UNAVAILABLE / DEADLINE_EXCEEDED / CANCELLED.
   * @throws SpendGuardError on rejected ack or any other gRPC failure surface.
   */
  async commitEstimated(req: CommitEstimatedRequest): Promise<void> {
    if (this.cfg.disabled) return;
    if (this.handshakeResult === null) {
      throw new HandshakeError(
        "commitEstimated() requires handshake(); call await client.handshake() before commitEstimated()",
      );
    }
    if (this.adapterClient === null) {
      await this.connect();
    }
    const adapter = this.adapterClient;
    if (adapter === null) {
      throw new SidecarUnavailable("transport not established for commitEstimated");
    }
    const event = this.buildLlmCallPostEvent(req);
    let call: ReturnType<SidecarAdapterClient["emitTraceEvents"]>;
    try {
      call = adapter.emitTraceEvents({
        timeout: this.cfg.traceTimeoutMs,
      });
      await call.requests.send(event);
      await call.requests.complete();
    } catch (err) {
      throw classifyRpcError(err, "commitEstimated");
    }

    // Drain the ack stream — sidecar emits exactly one ack per inbound event.
    try {
      let acked = false;
      for await (const ack of call.responses) {
        acked = true;
        if (ack.status !== TraceEventAck_Status.ACCEPTED) {
          throw new SpendGuardError(buildAckRejectMessage(ack));
        }
      }
      // Surface the final RPC status so a server-side error after the ack
      // (e.g. trailers-only cancellation) doesn't silently disappear.
      await call.status;
      await call.trailers;
      if (!acked) {
        throw new SpendGuardError("EmitTraceEvents closed without an ack from sidecar");
      }
    } catch (err) {
      if (err instanceof SpendGuardError) throw err;
      throw classifyRpcError(err, "commitEstimated");
    }
  }

  /**
   * Explicit release of a held reservation. Matches Agent Spend Protocol
   * Draft-01 §4. SLICE 5 wires the body.
   *
   * @throws SpendGuardError until SLICE 5 wires the body.
   */
  async release(req: ReleaseRequest): Promise<ReleaseOutcome> {
    void req;
    throw new SpendGuardError(`release() ${SLICE_5_NOT_WIRED}`);
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
    throw new SpendGuardError(`queryBudget() ${SLICE_5_NOT_WIRED}`);
  }

  // ── Lower-level surface (release / resume / confirm land in SLICE 5) ────

  /**
   * Confirm `publish_effect` outcome. SLICE 5 wires body.
   * @throws SpendGuardError until SLICE 5 wires the body.
   */
  async confirmPublishOutcome(req: PublishOutcomeRequest): Promise<string> {
    void req;
    throw new SpendGuardError(`confirmPublishOutcome() ${SLICE_5_NOT_WIRED}`);
  }

  /**
   * Resume after a human approver acted on a `REQUIRE_APPROVAL` decision.
   * SLICE 5 wires the body; references this method from `ApprovalRequired.resume`.
   * @throws SpendGuardError until SLICE 5 wires the body.
   */
  async resumeAfterApproval(req: ResumeAfterApprovalRequest): Promise<DecisionOutcome> {
    void req;
    throw new SpendGuardError(`resumeAfterApproval() ${SLICE_5_NOT_WIRED}`);
  }

  /**
   * Safe-ack the `APPLY_FAILED` publish outcome — swallows transport errors
   * so the caller's original exception is never shadowed. SLICE 5 wires body.
   * @throws SpendGuardError until SLICE 5 wires the body.
   */
  async safeConfirmApplyFailed(req: ApplyFailedRequest): Promise<void> {
    void req;
    throw new SpendGuardError(`safeConfirmApplyFailed() ${SLICE_5_NOT_WIRED}`);
  }

  /**
   * Lower-level entry point that `commitEstimated()` wraps. Provided so
   * adapters that need the raw trace-event surface have access. SLICE 5
   * wires body (the provider-report path).
   * @throws SpendGuardError until SLICE 5 wires the body.
   */
  async emitLlmCallPost(req: EmitLlmCallPostRequest): Promise<void> {
    void req;
    throw new SpendGuardError(`emitLlmCallPost() ${SLICE_5_NOT_WIRED}`);
  }

  // ── Internals ────────────────────────────────────────────────────────────

  /**
   * Build the gRPC channel credentials. Always insecure over UDS — the kernel
   * `SO_PEERCRED` check on the sidecar side is the trust anchor (Sidecar
   * Architecture §5). TLS over a Unix socket adds overhead with no security
   * benefit when the connection is implicitly local.
   *
   * Carved into its own method so a future slice can override under a
   * `runtime` flag once HTTP-gateway transport is added — `runtime: "fetch"`
   * would override to use TLS credentials. v0.1.x only supports `"uds-grpc"`.
   */
  private buildChannelCredentials(): ChannelCredentials {
    return grpcCredentials.createInsecure();
  }

  /**
   * Translate the public `ReserveRequest` (camelCase, TS-idiomatic) into the
   * snake_case-on-wire `DecisionRequest` proto. Per implementation.md §4:
   *
   *   1. SessionId from the cached handshake (caller already gated above).
   *   2. Trigger enum mapping via `triggerEnumOf()`.
   *   3. W3C `traceparent` → `TraceContext` via `buildTraceContext()` (matches
   *      Python `_build_trace_context`).
   *   4. `runtimeMetadata` carries the prompt hash (when caller supplied
   *      `promptText`) and any `decisionContextJson` keys. The
   *      `run_projection_policy` slot is filled from the caller's
   *      `decisionContextJson.run_projection_policy` if present, otherwise
   *      from `cfg.runProjectionDefault` when non-empty. **This is the
   *      SLICE 4 consumption of MJ-1** — SLICE 3 stored the field on the
   *      config; this method wires it onto the wire.
   *   5. `plannedStepsHint` lands as the proto3 default `0` until SLICE 7
   *      wires `withRunPlan` (anti-scope of this slice).
   *
   * `runtime_metadata` is encoded as a hand-built `google.protobuf.Struct`
   * payload because the SDK does not yet ship `computePromptHash` (SLICE 6).
   * Until then this method ALWAYS sends an empty Struct body when no caller
   * decoration is requested — matching Python `runtime_metadata = None` which
   * is wire-equivalent to "field absent" under proto3 message optionality.
   */
  private buildDecisionRequest(req: ReserveRequest): ProtoDecisionRequest {
    if (this.handshakeResult === null) {
      // Re-asserts the gate `reserve()` already enforced; lets TS narrow.
      throw new HandshakeError("internal: buildDecisionRequest without handshake");
    }
    const trigger = triggerEnumOf(req.trigger);
    const trace = buildTraceContext(req.traceparent ?? "", req.tracestate ?? "");
    const ids = {
      runId: req.runId,
      stepId: req.stepId,
      llmCallId: req.llmCallId,
      toolCallId: req.toolCallId ?? "",
      decisionId: req.decisionId,
      snapshotId: "",
    };
    const projectedClaims = req.projectedClaims.map((claim) => ({
      budgetId: claim.scopeId,
      unit: mapUnitRef(claim.unit),
      amountAtomic: claim.amountAtomic,
      // direction: DEBIT (1) — SDK callers only project debits; credits are
      // generated server-side as compensating ledger entries (Stage 2 §4.6).
      direction: 1 as const,
      windowInstanceId: "",
    }));
    const runtimeMetadata = this.buildRuntimeMetadataStruct(req);
    const inputs = {
      projectedClaims,
      projectedP50Atomic: req.projectedP50Atomic ?? "",
      projectedP90Atomic: req.projectedP90Atomic ?? "",
      projectedP95Atomic: req.projectedP95Atomic ?? "",
      projectedP99Atomic: req.projectedP99Atomic ?? "",
      ...(req.projectedUnit !== undefined ? { projectedUnit: mapUnitRef(req.projectedUnit) } : {}),
      ...(runtimeMetadata !== undefined ? { runtimeMetadata } : {}),
      ...(req.claimEstimate !== undefined
        ? { claimEstimate: mapClaimEstimate(req.claimEstimate) }
        : {}),
    };
    const idempotency = {
      key: req.idempotencyKey,
      // The sidecar/ledger own the canonical request hash; SDK leaves it empty
      // (matches Python parity at client.py:494).
      requestHash: new Uint8Array(),
    };
    return {
      sessionId: this.handshakeResult.sessionId,
      trigger,
      trace,
      ids,
      route: req.route,
      inputs,
      parentRunId: req.parentRunId ?? "",
      budgetGrantJti: req.budgetGrantJti ?? "",
      idempotency,
      // SLICE 7 wires `withRunPlan`; until then send the proto3 default `0`
      // which the projector treats as "Signal 3 inactive" per
      // run-cost-projector-spec-v1alpha1.md §5.2.
      plannedStepsHint: 0,
    };
  }

  /**
   * Build the `google.protobuf.Struct` payload that lands in
   * `DecisionRequest.inputs.runtime_metadata`. Returns `undefined` when there
   * is nothing to send (proto3 message optionality — wire equivalent to
   * "field absent").
   *
   * Two slots are populated here:
   *   - `decision_context_json.*` keys from the caller (verbatim).
   *   - `run_projection_policy` from the caller (if present in
   *     `decisionContextJson`) OR `cfg.runProjectionDefault` (when set and
   *     non-empty). The caller's value wins; the default only fills in when
   *     the caller did not provide one — matches design.md §4.2 R2 semantics.
   *
   * Note: `prompt_hash` enrichment is INTENTIONALLY deferred to SLICE 6 when
   * `computePromptHash` ships. The Python parity is loose here — Python
   * already has the helper available; the TS SDK adds it in SLICE 6 along
   * with the cross-language fixture gate (review-standards §2). Until then,
   * `promptText` on the request is silently discarded with a JSDoc note.
   */
  private buildRuntimeMetadataStruct(
    req: ReserveRequest,
  ): { fields: Record<string, ProtoStructValue> } | undefined {
    const fields: Record<string, ProtoStructValue> = {};
    let hasField = false;
    if (req.decisionContextJson !== undefined) {
      for (const [k, v] of Object.entries(req.decisionContextJson)) {
        fields[k] = jsonValueToStructValue(v);
        hasField = true;
      }
    }
    // Default `run_projection` policy — fills only when the caller did not
    // already set it. Empty string is treated as "unset" per design §5.1.
    if (this.cfg.runProjectionDefault !== "" && fields.run_projection_policy === undefined) {
      fields.run_projection_policy = jsonValueToStructValue(this.cfg.runProjectionDefault);
      hasField = true;
    }
    return hasField ? { fields } : undefined;
  }

  /**
   * Build the single LLM_CALL_POST trace event for `commitEstimated()`.
   * Mirrors Python `emit_llm_call_post` at client.py:818 with the difference
   * that `provider_reported_amount_atomic` is always empty here (the
   * provider-report path lives in SLICE 5+'s `emitLlmCallPost`).
   */
  private buildLlmCallPostEvent(req: CommitEstimatedRequest): ProtoTraceEvent {
    if (this.handshakeResult === null) {
      throw new HandshakeError("internal: buildLlmCallPostEvent without handshake");
    }
    const ts = wallClockToTimestamp(Date.now());
    return {
      sessionId: this.handshakeResult.sessionId,
      trace: buildTraceContext(req.traceparent ?? "", req.tracestate ?? ""),
      ids: {
        runId: req.runId,
        stepId: req.stepId,
        llmCallId: req.llmCallId,
        toolCallId: "",
        decisionId: req.decisionId,
        snapshotId: "",
      },
      kind: TraceEvent_EventKind.LLM_CALL_POST,
      eventTime: ts,
      payload: {
        oneofKind: "llmCallPost",
        llmCallPost: {
          reservationId: req.reservationId,
          providerReportedAmountAtomic: "",
          unit: mapUnitRef(req.unit),
          pricing: {
            pricingVersion: req.pricing.pricingVersion,
            priceSnapshotHash: req.pricing.pricingHash,
            fxRateVersion: "",
            unitConversionVersion: "",
          },
          providerEventId: req.providerEventId,
          outcome: llmOutcomeEnumOf(req.outcome),
          estimatedAmountAtomic: req.estimatedAmountAtomic,
          ...(req.actualInputTokens !== undefined
            ? { actualInputTokens: String(req.actualInputTokens) }
            : {}),
          ...(req.actualOutputTokens !== undefined
            ? { actualOutputTokens: String(req.actualOutputTokens) }
            : {}),
          ...(req.deltaBRatio !== undefined ? { deltaBRatio: req.deltaBRatio } : {}),
          ...(req.deltaCRatio !== undefined ? { deltaCRatio: req.deltaCRatio } : {}),
        },
      },
      providerResponseMetadata: req.providerResponseMetadata ?? "",
    };
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

/** Format `n` as `0x<hex>`; matches Python `hex(n)`. */
function toHex(n: number): string {
  return `0x${n.toString(16)}`;
}

/**
 * Map a public-surface `ReserveRequest.trigger` literal to the proto enum
 * value. Mirrors Python `_trigger_for_name`.
 */
function triggerEnumOf(name: ReserveRequest["trigger"]): DecisionRequest_Trigger {
  switch (name) {
    case "RUN_PRE":
      return DecisionRequest_Trigger.RUN_PRE;
    case "AGENT_STEP_PRE":
      return DecisionRequest_Trigger.AGENT_STEP_PRE;
    case "LLM_CALL_PRE":
      return DecisionRequest_Trigger.LLM_CALL_PRE;
    case "TOOL_CALL_PRE":
      return DecisionRequest_Trigger.TOOL_CALL_PRE;
    default: {
      // Exhaustiveness check — unreachable while the LOCKED §4.3 union holds.
      const _exhaustive: never = name;
      void _exhaustive;
      throw new SpendGuardError(`unknown trigger: ${String(name)}`);
    }
  }
}

/**
 * Map a `LlmCallPostPayload.outcome` literal to the proto enum value.
 * Mirrors Python `_llm_outcome_for_name`.
 */
function llmOutcomeEnumOf(name: CommitEstimatedRequest["outcome"]): LlmCallPostPayload_Outcome {
  switch (name) {
    case "SUCCESS":
      return LlmCallPostPayload_Outcome.SUCCESS;
    case "PROVIDER_ERROR":
      return LlmCallPostPayload_Outcome.PROVIDER_ERROR;
    case "CLIENT_TIMEOUT":
      return LlmCallPostPayload_Outcome.CLIENT_TIMEOUT;
    case "RUN_ABORTED":
      return LlmCallPostPayload_Outcome.RUN_ABORTED;
    default: {
      const _exhaustive: never = name;
      void _exhaustive;
      throw new SpendGuardError(`unknown llm outcome: ${String(name)}`);
    }
  }
}

/**
 * Convert the proto `DecisionResponse.decision` enum back to its name. Used by
 * `mapDecisionResponse` to dispatch on the wire decision.
 */
function decisionEnumName(value: DecisionResponse_Decision): string {
  switch (value) {
    case DecisionResponse_Decision.CONTINUE:
      return "CONTINUE";
    case DecisionResponse_Decision.DEGRADE:
      return "DEGRADE";
    case DecisionResponse_Decision.SKIP:
      return "SKIP";
    case DecisionResponse_Decision.STOP:
      return "STOP";
    case DecisionResponse_Decision.REQUIRE_APPROVAL:
      return "REQUIRE_APPROVAL";
    case DecisionResponse_Decision.STOP_RUN_PROJECTION:
      return "STOP_RUN_PROJECTION";
    default:
      return "UNKNOWN";
  }
}

/**
 * Translate a W3C `traceparent` header into the wire `TraceContext`.
 *
 * Mirrors Python `_build_trace_context` at client.py:77. Empty / malformed
 * input yields an empty `TraceContext` (no trace propagation); the sidecar
 * treats trace fields as observability decoration and does not gate
 * enforcement on them.
 */
function buildTraceContext(
  traceparent: string,
  tracestate: string,
): {
  traceId: string;
  spanId: string;
  parentSpanId: string;
  traceState: string;
} {
  if (traceparent === "") {
    return {
      traceId: "",
      spanId: "",
      parentSpanId: "",
      traceState: tracestate,
    };
  }
  const parts = traceparent.split("-");
  if (parts.length !== 4 || parts[1]?.length !== 32 || parts[2]?.length !== 16) {
    return {
      traceId: "",
      spanId: "",
      parentSpanId: "",
      traceState: tracestate,
    };
  }
  return {
    traceId: parts[1],
    spanId: parts[2],
    // Per Python parity, reuse upstream span_id as parent_span_id — the next
    // event is a child span from the sidecar's perspective.
    parentSpanId: parts[2],
    traceState: tracestate,
  };
}

/** Map the public-surface `UnitRef` (compact 2-field shape) to the proto's 7-field shape. */
function mapUnitRef(unit: UnitRef): {
  unitId: string;
  kind: 0;
  currency: string;
  unitName: string;
  tokenKind: string;
  modelFamily: string;
  creditProgram: string;
} {
  // The public surface compresses the proto UnitRef to its (unit, denomination)
  // pair so adapters do not need to know about ledger_unit_id. We carry the
  // free-form `unit` literal into `unitName` (the proto's free-form slot when
  // kind is non-monetary) and leave the canonical-truth `unit_id` empty —
  // ledger resolves canonical truth server-side. SLICE 6 may broaden this when
  // pricing helpers land.
  return {
    unitId: "",
    kind: 0,
    currency: "",
    unitName: unit.unit,
    tokenKind: "",
    modelFamily: "",
    creditProgram: "",
  };
}

/** Map the public-surface `ClaimEstimate` to the proto `ClaimEstimate` shape. */
function mapClaimEstimate(estimate: ClaimEstimate): {
  tokenizerTier: string;
  tokenizerVersionId: string;
  inputTokens: string;
  predictedATokens: string;
  predictedBTokens: string;
  predictedCTokens: string;
  reservedStrategy: string;
  predictionStrategyUsed: string;
  predictionPolicyUsed: string;
  predictionConfidence: number;
  predictionSampleSize: string;
  coldStartLayerUsed: string;
  classifierVersion: string;
  fingerprintVersion: string;
  promptClassFingerprint: string;
  runProjectionAtDecisionAtomic: string;
  runPredictedRemainingSteps: number;
  runStepsCompletedSoFar: string;
  runCodeTriggered: string;
  model: string;
  promptClass: string;
} {
  return {
    tokenizerTier: estimate.tokenizerTier ?? "",
    tokenizerVersionId: estimate.tokenizerVersionId ?? "",
    inputTokens: int64Field(estimate.inputTokens),
    predictedATokens: int64Field(estimate.predictedATokens),
    predictedBTokens: int64Field(estimate.predictedBTokens),
    predictedCTokens: int64Field(estimate.predictedCTokens),
    reservedStrategy: estimate.reservedStrategy ?? "",
    predictionStrategyUsed: estimate.predictionStrategyUsed ?? "",
    predictionPolicyUsed: estimate.predictionPolicyUsed ?? "",
    predictionConfidence: estimate.predictionConfidence ?? 0,
    predictionSampleSize: int64Field(estimate.predictionSampleSize),
    coldStartLayerUsed: estimate.coldStartLayerUsed ?? "",
    classifierVersion: estimate.classifierVersion ?? "",
    fingerprintVersion: estimate.fingerprintVersion ?? "",
    promptClassFingerprint: estimate.promptClassFingerprint ?? "",
    runProjectionAtDecisionAtomic: int64Field(estimate.runProjectionAtDecisionAtomic),
    runPredictedRemainingSteps: estimate.runPredictedRemainingSteps ?? 0,
    runStepsCompletedSoFar: int64Field(estimate.runStepsCompletedSoFar),
    runCodeTriggered: estimate.runCodeTriggered ?? "",
    model: estimate.model ?? "",
    promptClass: estimate.promptClass ?? "",
  };
}

/** Convert a `number | bigint | undefined` to the proto `int64` string form. */
function int64Field(v: number | bigint | undefined): string {
  if (v === undefined) return "0";
  return typeof v === "bigint" ? v.toString() : Math.trunc(v).toString();
}

/**
 * Build a `google.protobuf.Timestamp` from epoch milliseconds. protobuf-ts
 * encodes `seconds` as a string and `nanos` as a number.
 */
function wallClockToTimestamp(epochMs: number): { seconds: string; nanos: number } {
  const seconds = Math.floor(epochMs / 1000);
  const nanos = (epochMs % 1000) * 1_000_000;
  return { seconds: seconds.toString(), nanos };
}

// ── Struct value translation (google.protobuf.Struct) ────────────────────

/**
 * Mirror of the `google.protobuf.Value` shape used by protobuf-ts.
 *
 * We hand-build the union here (instead of importing the generated type) to
 * keep the public-surface `decisionContextJson` ergonomic — callers pass a
 * plain `Record<string, unknown>` and this helper folds it onto the wire.
 * Proto3 `Value` is itself a oneof; we encode each branch verbatim.
 */
type ProtoStructValue =
  | { kind: { oneofKind: "nullValue"; nullValue: 0 } }
  | { kind: { oneofKind: "numberValue"; numberValue: number } }
  | { kind: { oneofKind: "stringValue"; stringValue: string } }
  | { kind: { oneofKind: "boolValue"; boolValue: boolean } }
  | {
      kind: { oneofKind: "structValue"; structValue: { fields: Record<string, ProtoStructValue> } };
    }
  | { kind: { oneofKind: "listValue"; listValue: { values: ProtoStructValue[] } } };

/**
 * Convert an arbitrary JSON-like JS value to `google.protobuf.Value`.
 *
 * Notes:
 *   - `undefined` is coerced to `null` (the wire has no `undefined`).
 *   - `bigint` is converted to a decimal string (no `Value.intValue`; the
 *     Struct spec only carries `numberValue: double`, which would lose
 *     precision for large bigints).
 *   - `Date` becomes its ISO string.
 *   - Functions / symbols become their `String(value)` repr (defensive — no
 *     adapter should pass them, but we want to avoid silent drops).
 */
function jsonValueToStructValue(value: unknown): ProtoStructValue {
  if (value === null || value === undefined) {
    return { kind: { oneofKind: "nullValue", nullValue: 0 } };
  }
  if (typeof value === "boolean") {
    return { kind: { oneofKind: "boolValue", boolValue: value } };
  }
  if (typeof value === "number") {
    return { kind: { oneofKind: "numberValue", numberValue: value } };
  }
  if (typeof value === "bigint") {
    return { kind: { oneofKind: "stringValue", stringValue: value.toString() } };
  }
  if (typeof value === "string") {
    return { kind: { oneofKind: "stringValue", stringValue: value } };
  }
  if (Array.isArray(value)) {
    return {
      kind: {
        oneofKind: "listValue",
        listValue: { values: value.map(jsonValueToStructValue) },
      },
    };
  }
  if (value instanceof Date) {
    return { kind: { oneofKind: "stringValue", stringValue: value.toISOString() } };
  }
  if (typeof value === "object") {
    const fields: Record<string, ProtoStructValue> = {};
    for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
      fields[k] = jsonValueToStructValue(v);
    }
    return {
      kind: { oneofKind: "structValue", structValue: { fields } },
    };
  }
  return { kind: { oneofKind: "stringValue", stringValue: String(value) } };
}

// ── Decision response mapping ────────────────────────────────────────────

/**
 * Translate a wire `DecisionResponse` into a public-surface `DecisionOutcome`
 * (for CONTINUE / DEGRADE) or raise the matching typed exception (STOP /
 * STOP_RUN_PROJECTION / SKIP / REQUIRE_APPROVAL / unknown). Mirrors Python
 * `client.py:531-582`.
 *
 * `tenantId` is carried in so `ApprovalRequired.resume()` can scope the
 * `GetApprovalForResume` round-trip against tenant (Python parity).
 */
function mapDecisionResponse(resp: ProtoDecisionResponse, tenantId: string): DecisionOutcome {
  const name = decisionEnumName(resp.decision);
  if (name === "CONTINUE" || name === "DEGRADE") {
    return {
      decisionId: resp.decisionId,
      auditDecisionEventId: resp.auditDecisionEventId,
      decision: name,
      mutationPatchJson: resp.mutationPatchJson,
      effectHash: resp.effectHash ?? new Uint8Array(),
      ledgerTransactionId: resp.ledgerTransactionId,
      reservationIds: Object.freeze([...resp.reservationIds]),
      ttlExpiresAtSeconds: Number(resp.ttlExpiresAt?.seconds ?? 0),
      reasonCodes: Object.freeze([...resp.reasonCodes]),
      matchedRuleIds: Object.freeze([...resp.matchedRuleIds]),
    };
  }
  if (name === "STOP" || name === "STOP_RUN_PROJECTION") {
    throw new DecisionStopped(
      `sidecar ${name} terminal=${resp.terminal} reasons=${JSON.stringify(resp.reasonCodes)}`,
      {
        decisionId: resp.decisionId,
        reasonCodes: [...resp.reasonCodes],
        ...(resp.auditDecisionEventId ? { auditDecisionEventId: resp.auditDecisionEventId } : {}),
        matchedRuleIds: [...resp.matchedRuleIds],
      },
    );
  }
  if (name === "SKIP") {
    throw new DecisionSkipped(`sidecar SKIP reasons=${JSON.stringify(resp.reasonCodes)}`, {
      decisionId: resp.decisionId,
      reasonCodes: [...resp.reasonCodes],
      ...(resp.auditDecisionEventId ? { auditDecisionEventId: resp.auditDecisionEventId } : {}),
      matchedRuleIds: [...resp.matchedRuleIds],
    });
  }
  if (name === "REQUIRE_APPROVAL") {
    throw new ApprovalRequired(
      `sidecar REQUIRE_APPROVAL approval_request_id=${resp.approvalRequestId}`,
      {
        decisionId: resp.decisionId,
        approvalRequestId: resp.approvalRequestId,
        ...(resp.approverRole ? { approverRole: resp.approverRole } : {}),
        reasonCodes: [...resp.reasonCodes],
        ...(resp.auditDecisionEventId ? { auditDecisionEventId: resp.auditDecisionEventId } : {}),
        matchedRuleIds: [...resp.matchedRuleIds],
        tenantId,
      },
    );
  }
  throw new DecisionDenied(`sidecar returned unknown decision=${resp.decision}`, {
    decisionId: resp.decisionId,
    reasonCodes: [...resp.reasonCodes],
    ...(resp.auditDecisionEventId ? { auditDecisionEventId: resp.auditDecisionEventId } : {}),
    matchedRuleIds: [...resp.matchedRuleIds],
  });
}

// ── RPC error classification ─────────────────────────────────────────────

/**
 * Translate an `RpcError` from protobuf-ts (which wraps the @grpc/grpc-js
 * status) into a typed SpendGuard exception. Mirrors Python
 * `_classify_rpc_error` at client.py:929:
 *
 *   - UNAVAILABLE / DEADLINE_EXCEEDED / CANCELLED → `SidecarUnavailable`
 *   - everything else → `SpendGuardError`
 *
 * Non-RpcError throws are wrapped as `SpendGuardError` to preserve the
 * untyped surface for higher layers (SLICE 8 adds retry routing).
 */
function classifyRpcError(err: unknown, op: string): SpendGuardError {
  if (err instanceof RpcError) {
    const code = err.code; // already a string like "UNAVAILABLE"
    if (code === "UNAVAILABLE" || code === "DEADLINE_EXCEEDED" || code === "CANCELLED") {
      return new SidecarUnavailable(
        `${op} failed: code=${code} detail=${JSON.stringify(err.message)}`,
        { cause: err },
      );
    }
    return new SpendGuardError(`${op} failed: code=${code} detail=${JSON.stringify(err.message)}`, {
      cause: err,
    });
  }
  if (err instanceof SpendGuardError) return err;
  return new SpendGuardError(`${op} failed: ${errorMessage(err)}`, { cause: err });
}

/** Build a human-readable rejection message for a non-ACCEPTED ack. */
function buildAckRejectMessage(ack: ProtoTraceEventAck): string {
  const statusName = ackStatusName(ack.status);
  const code = ack.error?.code ?? 0;
  const message = ack.error?.message ?? "";
  return `EmitTraceEvents rejected: status=${statusName} code=${code} message=${JSON.stringify(message)}`;
}

/** Pretty-print a `TraceEventAck_Status` enum value. */
function ackStatusName(value: TraceEventAck_Status): string {
  switch (value) {
    case TraceEventAck_Status.ACCEPTED:
      return "ACCEPTED";
    case TraceEventAck_Status.QUARANTINED:
      return "QUARANTINED";
    case TraceEventAck_Status.REJECTED:
      return "REJECTED";
    default:
      return `STATUS_${value}`;
  }
}

// ── Disabled-mode helpers (design.md §5.1 / implementation.md §4) ────────

/**
 * Disabled-mode handshake outcome. Provides a stable synthetic session id so
 * subsequent disabled-mode `reserve()` / `commitEstimated()` work without
 * surprise. The string `"disabled-noop-session"` is intentionally not a UUID
 * so any production code that mistakenly leaks into a real audit chain is
 * obviously broken.
 */
function makeDisabledHandshake(): HandshakeOutcome {
  return {
    sessionId: "disabled-noop-session",
    sidecarVersion: "disabled",
    schemaBundleId: "",
    schemaBundleHash: new Uint8Array(),
    contractBundleId: "",
    contractBundleHash: new Uint8Array(),
    capabilityRequired: 0,
    signingKeyId: "",
    announcementSignature: new Uint8Array(),
  };
}

/**
 * Disabled-mode decision outcome — always CONTINUE with the caller's
 * `decisionId` echoed. No reservations are issued; no ledger transaction is
 * created. **For tests only** — production code that relies on this in
 * disabled mode has silently lost enforcement.
 */
function makeDisabledDecision(req: ReserveRequest): DecisionOutcome {
  return {
    decisionId: req.decisionId,
    auditDecisionEventId: "",
    decision: "CONTINUE",
    mutationPatchJson: "",
    effectHash: new Uint8Array(),
    ledgerTransactionId: "",
    reservationIds: Object.freeze([]),
    ttlExpiresAtSeconds: 0,
    reasonCodes: Object.freeze(["disabled_mode"]),
    matchedRuleIds: Object.freeze([]),
  };
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
  RunProjectionPolicy,
} from "./config.js";
