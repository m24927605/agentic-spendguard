// SpendGuard SDK — `SpendGuardClient` (SLICE 3 + SLICE 4 + SLICE 5).
//
// SLICE 3 shipped the lifecycle surface and the UDS transport wiring; SLICE 4
// wired handshake / reserve / commitEstimated (single-event LLM_CALL_POST);
// SLICE 5 wires release / queryBudget (§9.4 placeholder), the multi-event
// commitEstimated extension, and the central gRPC Status → typed-error
// mapper that all four wired RPCs reuse.
//
// The class shell here is the contract D04 / D06 / D08 / D29 build against —
// the LOCKED §4.2 surface — so future slices author only the remaining bodies,
// not the class shape.
//
// Spec refs:
//   - design.md §4.2 (LOCKED public surface)
//   - design.md §4.4 (CommitEstimated / Release / QueryBudget shapes)
//   - design.md §4.5 (error hierarchy)
//   - design.md §5.1 / §5.2 (env var precedence + validation)
//   - design.md §6.3 (`grpc.default_authority=localhost` for UDS)
//   - design.md §9.4 (queryBudget deferral rationale)
//   - implementation.md §4 (skeleton)
//   - slices/COV_S05_03_d05_client_skeleton.md (SLICE 3)
//   - slices/COV_S05_04_d05_handshake_reserve_commit.md (SLICE 4)
//   - slices/COV_S05_05_d05_release_query.md (SLICE 5)
//
// What this file DOES wire (SLICE 3 + SLICE 4 + SLICE 5):
//   - Constructor that merges explicit options with env fallback + defaults.
//   - `connect()` → opens a `GrpcTransport` against `unix:<socketPath>` with
//     the `grpc.default_authority=localhost` channel option (the Python SDK's
//     well-documented tonic-compat workaround).
//   - `close()` graceful + idempotent.
//   - `[Symbol.asyncDispose]` for `await using client = new ...`.
//   - `tenantId` / `sessionId` / `handshakeOutcome` getters.
//   - `SpendGuardClient.fromEnv()` factory.
//   - `handshake()` + `reserve()` + `commitEstimated()` real RPC bodies.
//   - `release()` real RPC body (ASP Draft-01 §4 one-to-one).
//   - `queryBudget()` §9.4 placeholder (throws with tracking-issue URL).
//   - Multi-event `commitEstimated()` extension: optional `outcomeKind` +
//     actuals fields drive a second outcome-flavored event on the same bidi
//     stream. Single-event SLICE 4 path stays the unchanged default.
//   - Central `mapGrpcStatusToError()` consumed by all wired RPCs; preserves
//     the original `RpcError` in `cause`. Dispatches the FAILED_PRECONDITION
//     cluster on the `x-spendguard-reason-code` trailer metadata field
//     (IDEMPOTENCY_CONFLICT / BUDGET_EXCEEDED / BUNDLE_HOT_RELOADED).
//
// What this file does NOT wire (anti-scope, deferred to future slices):
//   - `confirmPublishOutcome()` / `resumeAfterApproval()` /
//     `safeConfirmApplyFailed()` / `emitLlmCallPost()` business bodies —
//     deferred to SLICE 7+. They are defined but throw with the
//     `SLICE_7_NOT_WIRED` marker.
//   - `ids.ts` / `promptHash.ts` / `pricing.ts` — SLICE 6.
//   - `withRunPlan` — SLICE 7.
//   - OTel / retry / idempotency cache — SLICE 8.

import { type ChannelCredentials, credentials as grpcCredentials } from "@grpc/grpc-js";
import type { status as GrpcStatus, ServiceError } from "@grpc/grpc-js";
import { GrpcTransport } from "@protobuf-ts/grpc-transport";
import { RpcError } from "@protobuf-ts/runtime-rpc";

import { ReservationSource } from "./_proto/spendguard/common/v1/common.js";
import { SidecarAdapterClient } from "./_proto/spendguard/sidecar_adapter/v1/adapter.client.js";
import type {
  CommitSessionDeltaOutcome as ProtoCommitSessionDeltaOutcome,
  DecisionRequest as ProtoDecisionRequest,
  DecisionResponse as ProtoDecisionResponse,
  HandshakeRequest as ProtoHandshakeRequest,
  HandshakeResponse as ProtoHandshakeResponse,
  ReleaseReservationRequest as ProtoReleaseReservationRequest,
  ReleaseReservationResponse as ProtoReleaseReservationResponse,
  ReleaseSessionOutcome as ProtoReleaseSessionOutcome,
  ReserveSessionOutcome as ProtoReserveSessionOutcome,
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
  ApprovalBundleHotReloadedError,
  ApprovalRequired,
  DecisionDenied,
  DecisionSkipped,
  DecisionStopped,
  HandshakeError,
  MutationApplyFailed,
  SidecarUnavailable,
  SpendGuardConfigError,
  SpendGuardConnectionError,
  SpendGuardError,
} from "./errors.js";
import { SPENDGUARD_OTEL_ATTR, withOtelSpan } from "./otel.js";
import { computePromptHash } from "./promptHash.js";
import { runWithRetry } from "./retry.js";
import { currentRunPlan } from "./runPlan.js";
import {
  type CommitSessionDeltaOutcome as SessionCommitDeltaOutcome,
  type CommitSessionDeltaRequest as SessionCommitDeltaRequest,
  type ReleaseSessionOutcome as SessionReleaseOutcome,
  type ReleaseSessionRequest as SessionReleaseRequest,
  type ReserveSessionOutcome as SessionReserveOutcome,
  type ReserveSessionRequest as SessionReserveRequest,
  buildCommitSessionDeltaRequest,
  buildReleaseSessionRequest,
  buildReserveSessionRequest,
  timestampToDate,
} from "./session.js";
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
  /** Canonical-truth UUID of the ledger unit row.
   *
   * When provided, the SDK threads it verbatim onto `BudgetClaim.unit.unit_id`
   * on the wire. When omitted, the SDK sends "" and the ledger reject with
   * `INVALID_REQUEST: claim[N].unit.unit_id empty`.
   *
   * Adapters that issue ledger-backed `client.reserve()` MUST provide
   * `unitId`. Recipe-style integrations (where no ledger reserve happens) MAY
   * omit. The most common operator path is to set this from the
   * `SPENDGUARD_UNIT_ID` env var at adapter construction time.
   *
   * NB: this is the ledger UUID, distinct from the free-form `unit` slug —
   * they are NOT interchangeable. Multiple unit slugs can resolve to the same
   * unit_id when migration aliasing is configured.
   */
  unitId?: string;
}

export interface BudgetClaim {
  scopeId: string;
  amountAtomic: string;
  unit: UnitRef;
  /** Canonical-truth UUID of the ledger window-instance row.
   *
   * When provided, the SDK threads it verbatim onto
   * `BudgetClaim.window_instance_id` on the wire. When omitted, the SDK sends
   * "" and the ledger rejects with
   * `INVALID_REQUEST: claim[N].window_instance_id empty`.
   *
   * Adapters that issue ledger-backed `client.reserve()` MUST provide
   * `windowInstanceId`. Recipe-style integrations (where no ledger reserve
   * happens) MAY omit. The most common operator path is to set this from the
   * `SPENDGUARD_WINDOW_INSTANCE_ID` env var at adapter construction time.
   *
   * Mirrors the HARDEN_D05_UR `UnitRef.unitId` broadening — same disease,
   * same additive backward-compatible cure (HARDEN_D05_WI).
   */
  windowInstanceId?: string;
}

export interface PricingFreeze {
  pricingVersion: string;
  pricingHash: Uint8Array;
  /** HARDEN_D05_WI — optional FX rate version of the pricing freeze tuple.
   * When omitted the SDK sends "" (pre-HARDEN wire shape). Ledger commit
   * validation compares the FULL tuple against the reservation's freeze, so
   * adapters whose reservations carry a non-empty fx version MUST thread it. */
  fxRateVersion?: string;
  /** HARDEN_D05_WI — optional unit-conversion version (see fxRateVersion). */
  unitConversionVersion?: string;
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
  // ── SLICE 5 multi-event extension ───────────────────────────────────────
  //
  // When `outcomeKind` is present, `commitEstimated()` emits TWO events on
  // the same EmitTraceEvents bidi stream:
  //   1. the original LLM_CALL_POST event (single-event SLICE 4 shape), and
  //   2. a second LLM_CALL_POST-kind event carrying the outcome semantics —
  //      `outcome` field = SUCCESS / PROVIDER_ERROR depending on `outcomeKind`,
  //      with `actualInputTokens` / `actualOutputTokens` re-asserted and
  //      `actualErrorMessage` threaded onto the wire's
  //      `providerResponseMetadata` JSON envelope.
  // When absent, behavior is unchanged from SLICE 4 — a single event is sent.
  //
  // **Declared deviation #1**: the slice doc references a `LLM_CALL_OUTCOME`
  // proto event kind and a `LlmCallOutcomeKind` enum that do not yet exist
  // in `sidecar_adapter/v1/adapter.proto`. To honor the slice scope without
  // a proto bump (which would require a coordinated sidecar release), the
  // second event reuses `LLM_CALL_POST` and projects FAILURE onto the
  // existing `LlmCallPostPayload_Outcome.PROVIDER_ERROR`. When the sidecar
  // ships dedicated `LLM_CALL_OUTCOME` / `LlmCallOutcomeKind` types, this
  // path is the migration target — adapters reading the events MUST treat
  // a (post, post) pair with second event's `provider_event_id` carrying
  // the actuals as semantically equivalent to the future
  // (LLM_CALL_POST, LLM_CALL_OUTCOME) pair.
  //
  // Fields are *additive*; they never change the meaning of existing fields.
  outcomeKind?: "SUCCESS" | "FAILURE";
  /**
   * `int64`-as-string, mirroring SLICE 4 `actualInputTokens` semantics for
   * the multi-event path. When `outcomeKind` is set and this is provided,
   * the outcome event carries the value verbatim; when omitted on the
   * outcome event, the SDK reuses `actualInputTokens` from this request
   * (avoids a double-spec for callers that already populate the SLICE 4
   * field).
   */
  actualInputTokensWire?: string;
  /** `int64`-as-string companion of `actualOutputTokens` (see above). */
  actualOutputTokensWire?: string;
  /**
   * Free-form error message threaded onto the outcome event's
   * `providerResponseMetadata` JSON envelope as `{"error_message": "..."}`.
   * Only consulted when `outcomeKind === "FAILURE"`; ignored on SUCCESS.
   */
  actualErrorMessage?: string;
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

// ── SLICE 7 placeholder marker ────────────────────────────────────────────
//
// The lower-level methods `confirmPublishOutcome` / `resumeAfterApproval` /
// `safeConfirmApplyFailed` / `emitLlmCallPost` ship as named stubs that throw
// with this marker until the SLICE 7 (lower-level RPC surface) lands. SLICE 5
// finishes the hot-path RPCs (`release` / `queryBudget` placeholder + the
// multi-event `commitEstimated` extension) so the SLICE 5 marker constant
// is gone.

const SLICE_7_NOT_WIRED =
  "not yet wired — SLICE 7 (see docs/slices/COV_S05_05_d05_release_query.md anti-scope)";

// ── SLICE 5 §9.4 placeholder marker ────────────────────────────────────────
//
// `queryBudget` is a public-surface method (LOCKED §4.2) but the sidecar wire
// is intentionally deferred — design.md §9.4 (locked decision #4) declares
// the TS surface precedes the Python surface AND the sidecar implementation.
// SLICE 5 wires the placeholder shape so adapters can program against the
// method; the throw message carries the tracking-issue URL so production
// callers see a clear "feature not yet available" instead of a stray gRPC
// NOT_FOUND.
const QUERY_BUDGET_NOT_YET_WIRED =
  "query_budget not yet wired in sidecar; tracked at https://github.com/m24927605/agentic-spendguard/issues/TBD-queryBudget";

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
    if (rawOpts.idempotencyCache !== undefined) {
      cfg.idempotencyCache = rawOpts.idempotencyCache;
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
    // ── SLICE 8 — wrap handshake RPC in OTel span (design.md §6.4) ─────────
    //
    // No retry helper here: handshake is one-shot per client lifetime; a
    // failure should surface to the caller, not silently retry (retrying a
    // handshake against a broken sidecar would mask the broken-sidecar signal
    // the caller needs to see).
    const resp: ProtoHandshakeResponse = await withOtelSpan(
      this.cfg.otelTracer,
      "handshake",
      {
        [SPENDGUARD_OTEL_ATTR.TENANT_ID]: this.cfg.tenantId,
        [SPENDGUARD_OTEL_ATTR.SDK_VERSION]: this.cfg.sdkVersion,
      },
      async () => {
        try {
          return await adapter.handshake(req, {
            timeout: this.cfg.handshakeTimeoutMs,
          }).response;
        } catch (err) {
          throw mapGrpcStatusToError(err, { rpc: "handshake" });
        }
      },
    );
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
    // ── SLICE 8 — in-process idempotency cache (design.md §6.5) ────────────
    //
    // Before issuing the sidecar RPC, consult `cfg.idempotencyCache?` for a
    // hit on `req.idempotencyKey`. A hit short-circuits the UDS round trip;
    // a miss falls through to the wire path normally. The sidecar maintains
    // its OWN idempotency cache (it MUST — it is the correctness gate); the
    // local cache lives ABOVE it for latency.
    const cache = this.cfg.idempotencyCache;
    if (cache !== undefined && req.idempotencyKey.length > 0) {
      const cached = cache.get(req.idempotencyKey);
      if (cached !== undefined) return cached;
    }
    if (this.adapterClient === null) {
      await this.connect();
    }
    const adapter = this.adapterClient;
    if (adapter === null) {
      throw new SidecarUnavailable("transport not established for reserve");
    }
    const grpcReq = this.buildDecisionRequest(req);
    // ── SLICE 8 — wrap RPC in OTel span (design.md §6.4) + retry helper ────
    return await withOtelSpan(
      this.cfg.otelTracer,
      "reserve",
      {
        [SPENDGUARD_OTEL_ATTR.TENANT_ID]: this.cfg.tenantId,
        [SPENDGUARD_OTEL_ATTR.DECISION_ID]: req.decisionId,
        [SPENDGUARD_OTEL_ATTR.TRIGGER]: req.trigger,
        [SPENDGUARD_OTEL_ATTR.SDK_VERSION]: this.cfg.sdkVersion,
      },
      async () => {
        let resp: ProtoDecisionResponse;
        try {
          resp = await runWithRetry(
            async () => {
              try {
                return await adapter.requestDecision(grpcReq, {
                  timeout: this.cfg.decisionTimeoutMs,
                }).response;
              } catch (err) {
                throw mapGrpcStatusToError(err, { rpc: "reserve" });
              }
            },
            { idempotencyKey: req.idempotencyKey },
          );
        } catch (err) {
          if (err instanceof SpendGuardError) throw err;
          throw mapGrpcStatusToError(err, { rpc: "reserve" });
        }
        if (resp.error && resp.error.code !== 0) {
          throw new SpendGuardError(
            `sidecar error code=${resp.error.code} message=${resp.error.message}`,
          );
        }
        const outcome = mapDecisionResponse(resp, this.cfg.tenantId);
        // Cache the successful outcome — design.md §6.5 + cache.ts JSDoc:
        // the cache key is the SAME idempotencyKey the sidecar would dedupe
        // on, so a same-process retry observes the cached outcome.
        if (cache !== undefined && req.idempotencyKey.length > 0) {
          cache.set(req.idempotencyKey, outcome);
        }
        return outcome;
      },
    );
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
   * lower-level `emitLlmCallPost` (SLICE 7+).
   *
   * **SLICE 5 multi-event extension.** When `req.outcomeKind` is set, the
   * client emits TWO events on the same bidi stream — the original
   * LLM_CALL_POST event first, then a second LLM_CALL_POST-kind event whose
   * `outcome` field reflects `outcomeKind` (SUCCESS → SUCCESS,
   * FAILURE → PROVIDER_ERROR) and whose `providerResponseMetadata` carries a
   * `{"error_message": ...}` envelope when `actualErrorMessage` is supplied.
   * Both events are acked individually; if either ack is non-ACCEPTED the
   * method raises `SpendGuardError`. See the JSDoc on
   * `CommitEstimatedRequest.outcomeKind` for the LLM_CALL_OUTCOME proto-kind
   * deviation note (Declared Deviation #1 in SLICE 5).
   *
   * When `req.outcomeKind` is ABSENT, behaviour is identical to SLICE 4 —
   * a single event is sent, a single ack is drained.
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
    const events: ProtoTraceEvent[] = [this.buildLlmCallPostEvent(req)];
    if (req.outcomeKind !== undefined) {
      events.push(this.buildLlmCallOutcomeEvent(req));
    }
    // ── SLICE 8 — wrap commit RPC in OTel span (design.md §6.4) ────────────
    //
    // No retry helper: the bidi-stream + ack drain is not safe to retry from
    // the outside (a partial-ack state on the sidecar side would observe a
    // duplicate event). The sidecar's own idempotency dedup on
    // `providerEventId` handles same-tenant repeat events; transient failures
    // here surface to the adapter for routing.
    await withOtelSpan(
      this.cfg.otelTracer,
      "commitEstimated",
      {
        [SPENDGUARD_OTEL_ATTR.TENANT_ID]: this.cfg.tenantId,
        [SPENDGUARD_OTEL_ATTR.DECISION_ID]: req.decisionId,
        [SPENDGUARD_OTEL_ATTR.SDK_VERSION]: this.cfg.sdkVersion,
      },
      async () => {
        let call: ReturnType<SidecarAdapterClient["emitTraceEvents"]>;
        try {
          call = adapter.emitTraceEvents({
            timeout: this.cfg.traceTimeoutMs,
          });
          for (const event of events) {
            await call.requests.send(event);
          }
          await call.requests.complete();
        } catch (err) {
          throw mapGrpcStatusToError(err, { rpc: "commitEstimated" });
        }

        // Drain the ack stream — sidecar emits exactly one ack per inbound event.
        try {
          let acked = 0;
          for await (const ack of call.responses) {
            acked += 1;
            if (ack.status !== TraceEventAck_Status.ACCEPTED) {
              throw new SpendGuardError(buildAckRejectMessage(ack));
            }
          }
          // Surface the final RPC status so a server-side error after the ack
          // (e.g. trailers-only cancellation) doesn't silently disappear.
          await call.status;
          await call.trailers;
          if (acked === 0) {
            throw new SpendGuardError("EmitTraceEvents closed without an ack from sidecar");
          }
          if (acked < events.length) {
            throw new SpendGuardError(
              `EmitTraceEvents acked ${acked} of ${events.length} events before closing`,
            );
          }
        } catch (err) {
          if (err instanceof SpendGuardError) throw err;
          throw mapGrpcStatusToError(err, { rpc: "commitEstimated" });
        }
      },
    );
  }

  /**
   * Explicit release of a held reservation. Matches Agent Spend Protocol
   * Draft-01 §4 one-to-one (the proto wire's
   * `ReleaseReservationRequest` carries the canonical ASP fields at tags 1-3
   * and SpendGuard extensions at tag 100+).
   *
   * Behaviour:
   *   - Disabled-mode short-circuit returns a synthetic
   *     `makeDisabledReleaseOutcome(req)` — no UDS contact.
   *   - Pre-handshake call throws `HandshakeError` via the `sessionId` getter
   *     gate (the request envelope requires the negotiated session id).
   *   - Wire envelope built by `buildReleaseRequest(req, sessionId)`.
   *   - Response mapped by `mapReleaseResponse(res, decisionIdHint)`.
   *   - Errors mapped centrally through `mapGrpcStatusToError`, with the
   *     `release`-specific NOT_FOUND override (reservation lookup misses
   *     surface as a plain `SpendGuardError("reservation not found")` so
   *     adapters can distinguish "no such reservation" from the rich
   *     FAILED_PRECONDITION cluster).
   *
   * @throws HandshakeError before `handshake()` completes.
   * @throws SidecarUnavailable on UNAVAILABLE / DEADLINE_EXCEEDED / CANCELLED.
   * @throws MutationApplyFailed on FAILED_PRECONDITION + IDEMPOTENCY_CONFLICT
   *   or BUDGET_EXCEEDED — or an unknown FAILED_PRECONDITION reason (the
   *   conservative default; never bare `SpendGuardError` for this cluster).
   * @throws ApprovalBundleHotReloadedError on FAILED_PRECONDITION +
   *   BUNDLE_HOT_RELOADED.
   * @throws SpendGuardError on NOT_FOUND ("reservation not found") + any
   *   other unmapped gRPC failure.
   */
  async release(req: ReleaseRequest): Promise<ReleaseOutcome> {
    if (this.cfg.disabled) return makeDisabledReleaseOutcome(req);
    if (this.handshakeResult === null) {
      throw new HandshakeError(
        "release() requires handshake(); call await client.handshake() before release()",
      );
    }
    if (this.adapterClient === null) {
      await this.connect();
    }
    const adapter = this.adapterClient;
    if (adapter === null) {
      throw new SidecarUnavailable("transport not established for release");
    }
    const grpcReq = buildReleaseRequest(req, this.handshakeResult.sessionId);
    // ── SLICE 8 — wrap release RPC in OTel span (design.md §6.4) ───────────
    //
    // No retry helper: release is idempotent on `idempotencyKey` on the
    // sidecar side, but the adapter typically calls release at most once
    // per reservation lifetime — a retry inside the SDK would mask the
    // unavailable signal the adapter needs to escalate to its own retry
    // budget (e.g. queued release-on-restart).
    return await withOtelSpan(
      this.cfg.otelTracer,
      "release",
      {
        [SPENDGUARD_OTEL_ATTR.TENANT_ID]: this.cfg.tenantId,
        [SPENDGUARD_OTEL_ATTR.RESERVATION_ID]: req.reservationId,
        [SPENDGUARD_OTEL_ATTR.SDK_VERSION]: this.cfg.sdkVersion,
      },
      async () => {
        let resp: ProtoReleaseReservationResponse;
        try {
          resp = await adapter.releaseReservation(grpcReq, {
            timeout: this.cfg.publishTimeoutMs,
          }).response;
        } catch (err) {
          throw mapGrpcStatusToError(err, { rpc: "release", releaseNotFoundAsPlain: true });
        }
        return mapReleaseResponse(resp);
      },
    );
  }

  /**
   * Reserve a session-scoped hold for a realtime voice session (D41 SR-V3).
   *
   * The public request shape mirrors `buildReserveSessionRequest`. When
   * `req.sessionId` is empty, the SDK fills it from the completed sidecar
   * handshake so adapter code can bind the session reservation to the active
   * UDS session without duplicating handshake plumbing.
   *
   * @throws HandshakeError before `handshake()` completes.
   * @throws SidecarUnavailable on UNAVAILABLE / DEADLINE_EXCEEDED / CANCELLED.
   * @throws SpendGuardError on proto error outcome or unmapped gRPC failure.
   */
  async reserveSession(req: SessionReserveRequest): Promise<SessionReserveOutcome> {
    if (this.cfg.disabled) {
      buildReserveSessionRequest(req);
      return makeDisabledReserveSessionOutcome(req);
    }
    if (this.handshakeResult === null) {
      throw new HandshakeError(
        "reserveSession() requires handshake(); call await client.handshake() before reserveSession()",
      );
    }
    if (this.adapterClient === null) {
      await this.connect();
    }
    const adapter = this.adapterClient;
    if (adapter === null) {
      throw new SidecarUnavailable("transport not established for reserveSession");
    }
    const grpcReq = buildReserveSessionRequest({
      ...req,
      sessionId: req.sessionId || this.handshakeResult.sessionId,
    });
    return await withOtelSpan(
      this.cfg.otelTracer,
      "reserveSession",
      {
        [SPENDGUARD_OTEL_ATTR.TENANT_ID]: this.cfg.tenantId,
        [SPENDGUARD_OTEL_ATTR.SDK_VERSION]: this.cfg.sdkVersion,
      },
      async () => {
        try {
          const resp = await adapter.reserveSession(grpcReq, {
            timeout: this.cfg.decisionTimeoutMs,
          }).response;
          return mapReserveSessionOutcome(resp);
        } catch (err) {
          throw mapGrpcStatusToError(err, { rpc: "reserveSession" });
        }
      },
    );
  }

  /**
   * Commit one positive streaming spend delta against a session reservation.
   *
   * @throws HandshakeError before `handshake()` completes.
   * @throws SidecarUnavailable on UNAVAILABLE / DEADLINE_EXCEEDED / CANCELLED.
   * @throws SpendGuardError on proto error outcome or unmapped gRPC failure.
   */
  async commitSessionDelta(req: SessionCommitDeltaRequest): Promise<SessionCommitDeltaOutcome> {
    if (this.cfg.disabled) {
      buildCommitSessionDeltaRequest(req);
      return makeDisabledCommitSessionDeltaOutcome(req);
    }
    if (this.handshakeResult === null) {
      throw new HandshakeError(
        "commitSessionDelta() requires handshake(); call await client.handshake() before commitSessionDelta()",
      );
    }
    if (this.adapterClient === null) {
      await this.connect();
    }
    const adapter = this.adapterClient;
    if (adapter === null) {
      throw new SidecarUnavailable("transport not established for commitSessionDelta");
    }
    const grpcReq = buildCommitSessionDeltaRequest(req);
    return await withOtelSpan(
      this.cfg.otelTracer,
      "commitSessionDelta",
      {
        [SPENDGUARD_OTEL_ATTR.TENANT_ID]: this.cfg.tenantId,
        [SPENDGUARD_OTEL_ATTR.SDK_VERSION]: this.cfg.sdkVersion,
      },
      async () => {
        try {
          const resp = await adapter.commitSessionDelta(grpcReq, {
            timeout: this.cfg.traceTimeoutMs,
          }).response;
          return mapCommitSessionDeltaOutcome(resp);
        } catch (err) {
          throw mapGrpcStatusToError(err, { rpc: "commitSessionDelta" });
        }
      },
    );
  }

  /**
   * Release the uncommitted remainder of a session reservation.
   *
   * @throws HandshakeError before `handshake()` completes.
   * @throws SidecarUnavailable on UNAVAILABLE / DEADLINE_EXCEEDED / CANCELLED.
   * @throws SpendGuardError on proto error outcome or unmapped gRPC failure.
   */
  async releaseSession(req: SessionReleaseRequest): Promise<SessionReleaseOutcome> {
    if (this.cfg.disabled) {
      buildReleaseSessionRequest(req);
      return makeDisabledReleaseSessionOutcome(req);
    }
    if (this.handshakeResult === null) {
      throw new HandshakeError(
        "releaseSession() requires handshake(); call await client.handshake() before releaseSession()",
      );
    }
    if (this.adapterClient === null) {
      await this.connect();
    }
    const adapter = this.adapterClient;
    if (adapter === null) {
      throw new SidecarUnavailable("transport not established for releaseSession");
    }
    const grpcReq = buildReleaseSessionRequest(req);
    return await withOtelSpan(
      this.cfg.otelTracer,
      "releaseSession",
      {
        [SPENDGUARD_OTEL_ATTR.TENANT_ID]: this.cfg.tenantId,
        [SPENDGUARD_OTEL_ATTR.SDK_VERSION]: this.cfg.sdkVersion,
      },
      async () => {
        try {
          const resp = await adapter.releaseSession(grpcReq, {
            timeout: this.cfg.publishTimeoutMs,
          }).response;
          return mapReleaseSessionOutcome(resp);
        } catch (err) {
          throw mapGrpcStatusToError(err, { rpc: "releaseSession" });
        }
      },
    );
  }

  /**
   * Read-only budget snapshot. Locked decision #4 of design.md §9: in v0.1.x
   * the substrate ships the method signature but the sidecar wire is NOT yet
   * implemented. SLICE 5 wires the §9.4 placeholder body — adapters call this
   * method, catch the explicit `SpendGuardError`, and surface a clear "feature
   * not yet available" upstream rather than a stray NOT_FOUND from a missing
   * RPC route.
   *
   * Disabled-mode short-circuit returns a synthetic
   * `makeDisabledQueryBudgetResult(req)` so unit tests can program against
   * the method without a sidecar.
   *
   * @throws HandshakeError before `handshake()` completes.
   * @throws SpendGuardError carrying the tracking-issue URL otherwise.
   */
  async queryBudget(req: QueryBudgetRequest): Promise<QueryBudgetResult> {
    if (this.cfg.disabled) return makeDisabledQueryBudgetResult(req);
    if (this.handshakeResult === null) {
      throw new HandshakeError(
        "queryBudget() requires handshake(); call await client.handshake() before queryBudget()",
      );
    }
    // ── SLICE 8 — wrap (still placeholder) RPC in OTel span ────────────────
    //
    // The not-yet-wired throw inside the callback records as a span exception
    // via `withOtelSpan` — observability dashboards can see the call attempt
    // even before the §9.4 wire lands.
    return await withOtelSpan(
      this.cfg.otelTracer,
      "queryBudget",
      {
        [SPENDGUARD_OTEL_ATTR.TENANT_ID]: this.cfg.tenantId,
        [SPENDGUARD_OTEL_ATTR.SCOPE_ID]: req.scopeId,
        [SPENDGUARD_OTEL_ATTR.SDK_VERSION]: this.cfg.sdkVersion,
      },
      async () => {
        throw new SpendGuardError(QUERY_BUDGET_NOT_YET_WIRED);
      },
    );
  }

  // ── Lower-level surface (deferred to SLICE 7+) ──────────────────────────

  /**
   * Confirm `publish_effect` outcome. SLICE 7 wires body.
   * @throws SpendGuardError until SLICE 7 wires the body.
   */
  async confirmPublishOutcome(req: PublishOutcomeRequest): Promise<string> {
    void req;
    throw new SpendGuardError(`confirmPublishOutcome() ${SLICE_7_NOT_WIRED}`);
  }

  /**
   * Resume after a human approver acted on a `REQUIRE_APPROVAL` decision.
   * SLICE 7 wires the body; references this method from `ApprovalRequired.resume`.
   * @throws SpendGuardError until SLICE 7 wires the body.
   */
  async resumeAfterApproval(req: ResumeAfterApprovalRequest): Promise<DecisionOutcome> {
    void req;
    throw new SpendGuardError(`resumeAfterApproval() ${SLICE_7_NOT_WIRED}`);
  }

  /**
   * Safe-ack the `APPLY_FAILED` publish outcome — swallows transport errors
   * so the caller's original exception is never shadowed. SLICE 7 wires body.
   * @throws SpendGuardError until SLICE 7 wires the body.
   */
  async safeConfirmApplyFailed(req: ApplyFailedRequest): Promise<void> {
    void req;
    throw new SpendGuardError(`safeConfirmApplyFailed() ${SLICE_7_NOT_WIRED}`);
  }

  /**
   * Lower-level entry point that `commitEstimated()` wraps. Provided so
   * adapters that need the raw trace-event surface have access. SLICE 7
   * wires body (the provider-report path).
   * @throws SpendGuardError until SLICE 7 wires the body.
   */
  async emitLlmCallPost(req: EmitLlmCallPostRequest): Promise<void> {
    void req;
    throw new SpendGuardError(`emitLlmCallPost() ${SLICE_7_NOT_WIRED}`);
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
   *   5. `plannedStepsHint` is `plan.plannedCalls + plan.plannedTools` when
   *      a `withRunPlan` scope is active (SLICE 7 R2), otherwise the proto3
   *      default `0`.
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
    // ── SLICE 7 (COV_S05_07) R2 — Signal 3 plannedStepsHint auto-fold ────
    //
    // When an adapter has wrapped its agent body in `withRunPlan({ plannedCalls,
    // plannedTools }, fn)`, every nested `reserve()` call ships the sum on
    // the wire `DecisionRequest.plannedStepsHint` field (Signal 3). The
    // sidecar forwards the hint to `run_cost_projector`, which uses Signal 3
    // to override its history-induced Signal 1 estimate. Without an active
    // plan, the field stays at proto3 default `0` and the projector falls
    // back to Signal 1.
    //
    // R2 retired the SLICE 7 R1 identity auto-fold (runId / parentRunId /
    // traceparent / tracestate / budgetGrantJti) — those fields stay on
    // `ReserveRequest` and are caller-threaded per the SLICE 4-5 wire path.
    // The LOCKED `RunPlan` shape (design.md §4.7 + impl.md §9) is purely a
    // budget-hint surface; identity propagation needs its own future spec
    // (`RunContext` / `withRunContext`).
    const plan = currentRunPlan();
    const plannedStepsHint = plan !== null ? plan.plannedCalls + plan.plannedTools : 0;

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
      // HARDEN_D05_WI — thread caller-supplied windowInstanceId onto the
      // wire claim. Omitted keeps the pre-HARDEN wire shape ("").
      windowInstanceId: claim.windowInstanceId ?? "",
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
      // SLICE 7 R2: Signal 3 — `plannedCalls + plannedTools` when an active
      // `withRunPlan` scope is in flight, otherwise proto3 default `0`. The
      // sidecar enforces the upper bound `[0, MAX_PLANNED_STEPS]` server-side
      // (`services/run_cost_projector/src/server.rs`) so the SDK doesn't gate
      // on a value the server may bump independently.
      plannedStepsHint,
      // D13 additive proto fields. These defaults preserve the pre-D13 BYOK
      // request-scoped path until an adapter explicitly opts into meter-only.
      reservationSource: ReservationSource.UNSPECIFIED,
      meterOnlyEstimate: false,
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
   * SLICE 6 R1 closure of SLICE 4 M-3: when `req.promptText` is set,
   * `computePromptHash(req.promptText, this.cfg.tenantId)` populates
   * `runtime_metadata.prompt_hash` as a stringValue. Mirrors Python parity
   * at `sdk/python/.../client.py` (the `prompt_hash` field is the rules
   * dedup key per Cost Advisor P0.5 §5.1). The caller may pre-set
   * `decisionContextJson.prompt_hash` to override (e.g. when the prompt is
   * tokenised upstream and the hash is computed there); the caller-supplied
   * value wins.
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
    // M-3 closure: prompt_hash from promptText. Caller-supplied value via
    // decisionContextJson wins so adapters that tokenise upstream can pass
    // the pre-computed hash and avoid re-hashing on the SDK hot path.
    if (req.promptText !== undefined && fields.prompt_hash === undefined) {
      const hash = computePromptHash(req.promptText, this.cfg.tenantId);
      fields.prompt_hash = jsonValueToStructValue(hash);
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
            // HARDEN_D05_WI — thread the full freeze tuple (omitted → "").
            fxRateVersion: req.pricing.fxRateVersion ?? "",
            unitConversionVersion: req.pricing.unitConversionVersion ?? "",
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

  /**
   * Build the SLICE 5 multi-event "outcome" companion event for
   * `commitEstimated()` when `req.outcomeKind` is set.
   *
   * The event reuses `TraceEvent_EventKind.LLM_CALL_POST` (per Declared
   * Deviation #1 — `LLM_CALL_OUTCOME` does not exist as a proto enum value
   * in `sidecar_adapter/v1/adapter.proto` yet).
   *
   * TODO(GH-issue-TBD): proto bump for `LLM_CALL_OUTCOME` +
   * sidecar `x-spendguard-reason-code` trailer extension. Both deferred to
   * the cross-component slice that touches `proto/` and
   * `services/sidecar/`. Track at
   * https://github.com/m24927605/agentic-spendguard/issues/TBD-proto-bump-llm-call-outcome
   * (R2 follow-up; not in scope for D05 SLICE 5).
   *
   * The `outcome` field on the inner `LlmCallPostPayload` carries the
   * semantic:
   *
   *   - `outcomeKind === "SUCCESS"` → `LlmCallPostPayload_Outcome.SUCCESS`
   *   - `outcomeKind === "FAILURE"` → `LlmCallPostPayload_Outcome.PROVIDER_ERROR`
   *
   * Actuals (`actualInputTokens` / `actualOutputTokens`) prefer the SLICE 5
   * `*Wire` fields when supplied (int64-as-string form), falling back to
   * the SLICE 4 numeric `actualInputTokens` / `actualOutputTokens` shape so
   * adapters do not need to double-specify. `actualErrorMessage` is threaded
   * onto `TraceEvent.providerResponseMetadata` as a JSON envelope
   * `{"error_message": "..."}` only when `outcomeKind === "FAILURE"`.
   *
   * `estimatedAmountAtomic` is intentionally set to an empty string on the
   * outcome event — the first event already booked the commit; the outcome
   * event only carries observation, not commit semantics. The sidecar will
   * surface a rejection ack if it requires the field (`mock-sidecar`'s tests
   * lock this in).
   *
   * @private
   */
  private buildLlmCallOutcomeEvent(req: CommitEstimatedRequest): ProtoTraceEvent {
    if (this.handshakeResult === null) {
      throw new HandshakeError("internal: buildLlmCallOutcomeEvent without handshake");
    }
    if (req.outcomeKind === undefined) {
      throw new SpendGuardError(
        "internal: buildLlmCallOutcomeEvent called without outcomeKind set",
      );
    }
    const ts = wallClockToTimestamp(Date.now());
    const inputWire =
      req.actualInputTokensWire ??
      (req.actualInputTokens !== undefined ? String(req.actualInputTokens) : undefined);
    const outputWire =
      req.actualOutputTokensWire ??
      (req.actualOutputTokens !== undefined ? String(req.actualOutputTokens) : undefined);
    const outcomeEnum: LlmCallPostPayload_Outcome =
      req.outcomeKind === "SUCCESS"
        ? LlmCallPostPayload_Outcome.SUCCESS
        : LlmCallPostPayload_Outcome.PROVIDER_ERROR;
    const errorMessageEnvelope =
      req.outcomeKind === "FAILURE" && req.actualErrorMessage !== undefined
        ? JSON.stringify({ error_message: req.actualErrorMessage })
        : (req.providerResponseMetadata ?? "");
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
            // HARDEN_D05_WI — thread the full freeze tuple (omitted → "").
            fxRateVersion: req.pricing.fxRateVersion ?? "",
            unitConversionVersion: req.pricing.unitConversionVersion ?? "",
          },
          providerEventId: req.providerEventId,
          outcome: outcomeEnum,
          // The outcome companion event carries observation; the booking
          // amount stayed on the first event. Empty string is wire-equivalent
          // to "absent" for the proto3 string field.
          estimatedAmountAtomic: "",
          ...(inputWire !== undefined ? { actualInputTokens: inputWire } : {}),
          ...(outputWire !== undefined ? { actualOutputTokens: outputWire } : {}),
          ...(req.deltaBRatio !== undefined ? { deltaBRatio: req.deltaBRatio } : {}),
          ...(req.deltaCRatio !== undefined ? { deltaCRatio: req.deltaCRatio } : {}),
        },
      },
      providerResponseMetadata: errorMessageEnvelope,
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
  // The public surface compresses the proto UnitRef to its (unit, denomination,
  // unitId?) shape. We carry the free-form `unit` literal into `unitName` (the
  // proto's free-form slot when kind is non-monetary) and thread `unitId`
  // through verbatim when the caller provides it — empty string triggers a
  // ledger `INVALID_REQUEST: claim[N].unit.unit_id empty` rejection at
  // reserve time, which is the intended fail-closed semantics for adapters
  // that forgot to wire the canonical-truth ledger UUID (HARDEN_D05_UR closes
  // the substrate gap that previously hardcoded "").
  return {
    unitId: unit.unitId ?? "",
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

// ── Central gRPC Status → typed-error mapper (SLICE 5) ────────────────────

/**
 * Trailer-metadata header name carrying the SpendGuard reason code for the
 * FAILED_PRECONDITION cluster.
 *
 * **R2 layering note (D05 SLICE 5)**: the production sidecar
 * (`services/sidecar/src/domain/error.rs:107-134` `DomainError::to_status`)
 * currently emits FAILED_PRECONDITION with the discriminator baked into the
 * Status message string (e.g. `"idempotency conflict: ..."`) and does NOT
 * set the `x-spendguard-reason-code` trailer. The mock sidecar
 * (`tests/_support/mockSidecar.ts:426`) DOES set the trailer because tests
 * were written before the production gap was reviewed.
 *
 * Mapper dispatch order (see `readReasonCode`):
 *   1. PRIMARY: parse the `RpcError.message` string (production sidecar
 *      path — `DomainError::Display` prefixes are the lockable contract).
 *   2. SECONDARY: read the `x-spendguard-reason-code` trailer (mock
 *      compatibility + forward-compat hook).
 *
 * TODO(cross-component-slice): when the sidecar sets
 * `x-spendguard-reason-code` trailer alongside `e.to_status()`, the
 * message-string parse becomes secondary. Track at the cross-component
 * sidecar trailer extension slice (NOT in scope for D05 SLICE 5).
 */
const REASON_CODE_HEADER = "x-spendguard-reason-code";

/**
 * Lockable contract: `DomainError::Display` prefix → canonical reason code.
 *
 * Sourced from `services/sidecar/src/domain/error.rs` `#[error(...)]`
 * attributes (lines 26-45) — these strings ARE the public Status-message
 * format the sidecar emits via `Status::failed_precondition(self.to_string())`.
 * Match is case-insensitive prefix on the bare `RpcError.message`
 * (anchored at the start; the `: <detail>` tail varies per call).
 *
 * Forward-compat: the bracket-tagged `[BUNDLE_HOT_RELOADED]` form comes from
 * `services/sidecar/src/server/adapter_uds.rs:1379` (resume path) and is
 * included so dispatch already works when a future cross-component slice
 * unifies the release/resume Status surface.
 *
 * Ordered ARRAY (not a map): longest / most-specific prefixes FIRST so a
 * shorter prefix doesn't accidentally swallow a longer one.
 */
const REASON_CODE_PREFIXES: ReadonlyArray<readonly [string, string]> = [
  // Forward-compat: bracket-tagged emission from the resume path.
  ["[bundle_hot_reloaded]", "BUNDLE_HOT_RELOADED"],
  // Defensive: matches the test fixture wording ("bundle hot-reloaded ...")
  // and any future un-bracketed sidecar emission without coupling to exact
  // detail text.
  ["bundle hot-reload", "BUNDLE_HOT_RELOADED"],
  // DomainError::IdempotencyConflict (error.rs:44-45).
  ["idempotency conflict", "IDEMPOTENCY_CONFLICT"],
  // FAILED_PRECONDITION cluster mapped to BUDGET_EXCEEDED — the five
  // variants from error.rs (lines 26-39) that to_status() routes to
  // Status::failed_precondition (excluding IdempotencyConflict above).
  ["reservation state conflict", "BUDGET_EXCEEDED"],
  ["reservation ttl expired", "BUDGET_EXCEEDED"],
  ["pricing freeze mismatch", "BUDGET_EXCEEDED"],
  ["overrun reservation", "BUDGET_EXCEEDED"],
  ["multi-reservation commit deferred", "BUDGET_EXCEEDED"],
];

/**
 * Context passed to `mapGrpcStatusToError`. The `rpc` field is folded into
 * the typed-error message; `releaseNotFoundAsPlain` opt-in flips NOT_FOUND
 * into a plain `SpendGuardError("reservation not found")` for the
 * `release()` call path (the only RPC that needs the override — `reserve` /
 * `handshake` / `commitEstimated` never legitimately surface NOT_FOUND).
 */
export interface MapGrpcStatusContext {
  rpc: string;
  releaseNotFoundAsPlain?: boolean;
}

/**
 * Translate a gRPC `Status` (surfaced as a protobuf-ts `RpcError`) into a
 * typed SpendGuard exception. Replaces the SLICE 3 `classifyRpcError` as the
 * single dispatcher for handshake / reserve / commitEstimated / release.
 *
 * Dispatch table:
 *   - `UNAVAILABLE` / `DEADLINE_EXCEEDED` / `CANCELLED` → `SidecarUnavailable`
 *     (mirrors Python `_classify_rpc_error`; SLICE 8 wires retry on this
 *     cluster).
 *   - `FAILED_PRECONDITION` — dispatch on `REASON_CODE_HEADER` metadata:
 *       * `IDEMPOTENCY_CONFLICT` | `BUDGET_EXCEEDED` → `MutationApplyFailed`
 *       * `BUNDLE_HOT_RELOADED` → `ApprovalBundleHotReloadedError`
 *       * unknown / missing reason → `MutationApplyFailed` (default; never
 *         a bare `SpendGuardError` for this cluster, so adapters can route
 *         on `instanceof MutationApplyFailed` without surprise).
 *   - `NOT_FOUND` — usually `SpendGuardError`; when
 *     `ctx.releaseNotFoundAsPlain` is true, returns the exact-message
 *     `SpendGuardError("reservation not found")` so callers can `===`-match.
 *   - `ABORTED` → `SpendGuardError` (Python parity).
 *   - any other gRPC status → `SpendGuardError`.
 *   - any other thrown value (non-RpcError) → `SpendGuardError` with the
 *     value preserved on `cause`.
 *
 * Every returned error preserves the original `RpcError` (or thrown value)
 * on `cause` so adapters debugging in dev tools see the underlying gRPC
 * status + trailer metadata.
 */
function mapGrpcStatusToError(err: unknown, ctx: MapGrpcStatusContext): SpendGuardError {
  if (err instanceof SpendGuardError) return err;
  if (!(err instanceof RpcError)) {
    return new SpendGuardError(`${ctx.rpc} failed: ${errorMessage(err)}`, { cause: err });
  }
  const code = err.code;
  const cause = err;
  if (code === "UNAVAILABLE" || code === "DEADLINE_EXCEEDED" || code === "CANCELLED") {
    return new SidecarUnavailable(
      `${ctx.rpc} failed: code=${code} detail=${JSON.stringify(err.message)}`,
      { cause },
    );
  }
  if (code === "FAILED_PRECONDITION") {
    const reason = readReasonCode(err);
    if (reason === "BUNDLE_HOT_RELOADED") {
      return new ApprovalBundleHotReloadedError(
        `${ctx.rpc} failed: code=FAILED_PRECONDITION reason=BUNDLE_HOT_RELOADED detail=${JSON.stringify(err.message)}`,
        { originalBundleHash: "", currentBundleHash: "" },
        { cause },
      );
    }
    // IDEMPOTENCY_CONFLICT / BUDGET_EXCEEDED / unknown → MutationApplyFailed
    // (the conservative default per review-standards §5: callers see a
    // typed exception, never bare SpendGuardError, for FAILED_PRECONDITION).
    const reasonSuffix = reason !== undefined ? ` reason=${reason}` : "";
    return new MutationApplyFailed(
      `${ctx.rpc} failed: code=FAILED_PRECONDITION${reasonSuffix} detail=${JSON.stringify(err.message)}`,
      { cause },
    );
  }
  if (code === "NOT_FOUND") {
    if (ctx.releaseNotFoundAsPlain === true) {
      return new SpendGuardError("reservation not found", { cause });
    }
    return new SpendGuardError(
      `${ctx.rpc} failed: code=NOT_FOUND detail=${JSON.stringify(err.message)}`,
      { cause },
    );
  }
  if (code === "ABORTED") {
    return new SpendGuardError(
      `${ctx.rpc} failed: code=ABORTED detail=${JSON.stringify(err.message)}`,
      { cause },
    );
  }
  return new SpendGuardError(
    `${ctx.rpc} failed: code=${code} detail=${JSON.stringify(err.message)}`,
    { cause },
  );
}

/**
 * Resolve the canonical SpendGuard reason code for a FAILED_PRECONDITION
 * `RpcError`. Two layered discriminators (see R2 layering note on
 * `REASON_CODE_HEADER`):
 *
 *   1. PRIMARY: `RpcError.message` string parse against
 *      `REASON_CODE_PREFIXES` (production sidecar — `DomainError::Display`).
 *   2. SECONDARY: `x-spendguard-reason-code` trailer (mock harness +
 *      forward-compat for the cross-component slice that will extend the
 *      sidecar to set the trailer alongside `e.to_status()`).
 *
 * Returns `undefined` when neither discriminator yields a match — the
 * caller falls back to the conservative `MutationApplyFailed` default per
 * `review-standards §5`.
 *
 * `RpcMetadata` values are `string | string[]`; when an array is supplied
 * we take the first entry (gRPC-js coalesces duplicate headers).
 */
function readReasonCode(err: RpcError): string | undefined {
  // PRIMARY: message-string prefix dispatch. Production sidecar's
  // DomainError::Display strings (see REASON_CODE_PREFIXES). Case-fold the
  // candidate; the prefix table is already lowercase.
  const msg = err.message;
  if (typeof msg === "string" && msg.length > 0) {
    const lower = msg.toLowerCase();
    for (const [prefix, code] of REASON_CODE_PREFIXES) {
      if (lower.startsWith(prefix)) return code;
    }
  }
  // SECONDARY: trailer metadata. Mock sidecar tests + future
  // cross-component slice when the sidecar starts setting the trailer.
  const raw = err.meta?.[REASON_CODE_HEADER];
  if (raw === undefined) return undefined;
  if (typeof raw === "string") return raw;
  if (Array.isArray(raw) && raw.length > 0 && typeof raw[0] === "string") return raw[0];
  return undefined;
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

/**
 * Disabled-mode `release()` outcome — a synthetic, signature-empty
 * acknowledgement that records the caller's `reservationId` in the released
 * list. **For tests only** — production code that relies on this has
 * silently lost the audit chain entry for the release.
 */
function makeDisabledReleaseOutcome(req: ReleaseRequest): ReleaseOutcome {
  return {
    auditEventSignature: new Uint8Array(),
    ledgerTransactionId: "",
    releasedReservationIds: Object.freeze([req.reservationId]),
  };
}

function makeDisabledReserveSessionOutcome(req: SessionReserveRequest): SessionReserveOutcome {
  return {
    kind: "accepted",
    sessionReservationId: `disabled:${req.sessionId || "session"}`,
    ledgerTransactionId: "",
    auditSessionEventId: "",
    ttlExpiresAt: null,
    reservedAmountAtomic: req.estimatedAmountAtomic,
    remainingAmountAtomic: req.estimatedAmountAtomic,
  };
}

function makeDisabledCommitSessionDeltaOutcome(
  req: SessionCommitDeltaRequest,
): SessionCommitDeltaOutcome {
  return {
    sessionReservationId: req.sessionReservationId,
    streamingCommitId: req.streamingCommitId,
    ledgerTransactionId: "",
    auditSessionEventId: "",
    committedDeltaAtomic: req.amountAtomicDelta,
    cumulativeCommittedAtomic: req.amountAtomicDelta,
    remainingAmountAtomic: "0",
    recordedAt: null,
  };
}

function makeDisabledReleaseSessionOutcome(req: SessionReleaseRequest): SessionReleaseOutcome {
  return {
    sessionReservationId: req.sessionReservationId,
    ledgerTransactionId: "",
    auditSessionEventId: "",
    releasedAmountAtomic: "0",
    committedAmountAtomic: "0",
    recordedAt: null,
  };
}

/**
 * Disabled-mode `queryBudget()` result — a synthetic zero snapshot at the
 * caller's `asOfSeconds` (or `0` when unset). The unit is forced to
 * `USD_MICROS` with denomination 1 because the disabled mode has no handshake
 * context to consult. **For tests only** — production code that relies on
 * this is reading a fake budget.
 */
function makeDisabledQueryBudgetResult(req: QueryBudgetRequest): QueryBudgetResult {
  return {
    availableAtomic: "0",
    reservedAtomic: "0",
    committedAtomic: "0",
    unit: { unit: "USD_MICROS", denomination: 1 },
    asOfSeconds: req.asOfSeconds ?? 0,
  };
}

// ── Release wire mapping (SLICE 5) ────────────────────────────────────────

/**
 * Translate the public `ReleaseRequest` to the snake_case-on-wire
 * `ReleaseReservationRequest` proto. ASP Draft-01 §4 fields land at proto
 * tags 1-3; SpendGuard extensions (sessionId / tenantId / workloadInstanceId)
 * land at tags 100+.
 *
 * `sessionId` is the handshake-cached value (callers gated above pass it in
 * verbatim). Empty strings on `tenantId` / `workloadInstanceId` mean
 * "sidecar defaults apply" per the proto's documented semantic.
 */
function buildReleaseRequest(
  req: ReleaseRequest,
  sessionId: string,
): ProtoReleaseReservationRequest {
  return {
    reservationId: req.reservationId,
    idempotencyKey: req.idempotencyKey,
    reasonCodes: [...(req.reasonCodes ?? [])],
    tenantId: req.tenantId ?? "",
    workloadInstanceId: req.workloadInstanceId ?? "",
    sessionId,
  };
}

function mapReserveSessionOutcome(resp: ProtoReserveSessionOutcome): SessionReserveOutcome {
  switch (resp.outcome.oneofKind) {
    case "accepted": {
      const accepted = resp.outcome.accepted;
      return {
        kind: "accepted",
        sessionReservationId: accepted.sessionReservationId,
        ledgerTransactionId: accepted.ledgerTransactionId,
        auditSessionEventId: accepted.auditSessionEventId,
        ttlExpiresAt: timestampToDate(accepted.ttlExpiresAt),
        reservedAmountAtomic: accepted.reservedAmountAtomic,
        remainingAmountAtomic: accepted.remainingAmountAtomic,
      };
    }
    case "denied": {
      const denied = resp.outcome.denied;
      return {
        kind: "denied",
        auditSessionEventId: denied.auditSessionEventId,
        reasonCodes: Object.freeze([...denied.reasonCodes]),
        matchedRuleIds: Object.freeze([...denied.matchedRuleIds]),
        ...(denied.error !== undefined ? { error: denied.error } : {}),
      };
    }
    case "error":
      throw new SpendGuardError(
        `sidecar reserveSession error code=${resp.outcome.error.code} message=${resp.outcome.error.message}`,
      );
    default:
      throw new SpendGuardError("sidecar reserveSession returned empty outcome");
  }
}

function mapCommitSessionDeltaOutcome(
  resp: ProtoCommitSessionDeltaOutcome,
): SessionCommitDeltaOutcome {
  switch (resp.outcome.oneofKind) {
    case "accepted": {
      const accepted = resp.outcome.accepted;
      return {
        sessionReservationId: accepted.sessionReservationId,
        streamingCommitId: accepted.streamingCommitId,
        ledgerTransactionId: accepted.ledgerTransactionId,
        auditSessionEventId: accepted.auditSessionEventId,
        committedDeltaAtomic: accepted.committedDeltaAtomic,
        cumulativeCommittedAtomic: accepted.cumulativeCommittedAtomic,
        remainingAmountAtomic: accepted.remainingAmountAtomic,
        recordedAt: timestampToDate(accepted.recordedAt),
      };
    }
    case "error":
      throw new SpendGuardError(
        `sidecar commitSessionDelta error code=${resp.outcome.error.code} message=${resp.outcome.error.message}`,
      );
    default:
      throw new SpendGuardError("sidecar commitSessionDelta returned empty outcome");
  }
}

function mapReleaseSessionOutcome(resp: ProtoReleaseSessionOutcome): SessionReleaseOutcome {
  switch (resp.outcome.oneofKind) {
    case "accepted": {
      const accepted = resp.outcome.accepted;
      return {
        sessionReservationId: accepted.sessionReservationId,
        ledgerTransactionId: accepted.ledgerTransactionId,
        auditSessionEventId: accepted.auditSessionEventId,
        releasedAmountAtomic: accepted.releasedAmountAtomic,
        committedAmountAtomic: accepted.committedAmountAtomic,
        recordedAt: timestampToDate(accepted.recordedAt),
      };
    }
    case "error":
      throw new SpendGuardError(
        `sidecar releaseSession error code=${resp.outcome.error.code} message=${resp.outcome.error.message}`,
      );
    default:
      throw new SpendGuardError("sidecar releaseSession returned empty outcome");
  }
}

/**
 * Translate the wire `ReleaseReservationResponse` to the public-surface
 * `ReleaseOutcome`. The wire returns:
 *   - `auditEventSignature` — detached Ed25519 signature over the emitted
 *     `audit.release` CloudEvent (may be empty on idempotent replay miss).
 *   - `ledgerTransactionId` — sidecar's release-side transaction id.
 *   - `releasedReservationIds` — single-element array in the current
 *     single-reservation-per-call model; preserved as readonly to match the
 *     `ReleaseOutcome` type.
 */
function mapReleaseResponse(resp: ProtoReleaseReservationResponse): ReleaseOutcome {
  return {
    auditEventSignature: resp.auditEventSignature ?? new Uint8Array(),
    ledgerTransactionId: resp.ledgerTransactionId,
    releasedReservationIds: Object.freeze([...resp.releasedReservationIds]),
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
