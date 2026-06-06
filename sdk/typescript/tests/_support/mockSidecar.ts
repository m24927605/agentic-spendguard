// Mock sidecar UDS server for SLICE 3 lifecycle tests + SLICE 4 / SLICE 5
// RPC bodies.
//
// SLICE 3 only verified the connect → close lifecycle. SLICE 4 extended the
// mock with REAL `SidecarAdapter` service handlers for handshake /
// requestDecision / emitTraceEvents so the new RPC bodies can be exercised
// against a deterministic in-memory sidecar. SLICE 5 adds:
//   - `ReleaseReservation` handler (default success + parameterizable failing
//     handler with configurable gRPC Status code + trailer reason-code).
//   - Multi-event emitTraceEvents capture: tests can observe both
//     LLM_CALL_POST AND a second-event payload on the same bidi stream.
//
// SLICE 9 ships the full cross-language fixture mock per `tests.md` §4.2; this
// file remains the lighter-weight per-slice harness.

import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  status as GrpcStatus,
  Metadata,
  Server,
  ServerCredentials,
  type ServerDuplexStream,
  type ServerUnaryCall,
  type sendUnaryData,
} from "@grpc/grpc-js";

import {
  DecisionRequest,
  DecisionResponse,
  DecisionResponse_Decision,
  HandshakeRequest,
  HandshakeRequest_CapabilityLevel,
  HandshakeResponse,
  ReleaseReservationRequest,
  ReleaseReservationResponse,
  TraceEvent,
  TraceEventAck,
  TraceEventAck_Status,
} from "../../src/_proto/spendguard/sidecar_adapter/v1/adapter.js";

/**
 * Behavior hooks for the mock SidecarAdapter service. Tests register a hook
 * per RPC and drive the mock through expected (or adversarial) flows.
 *
 * Each hook is optional — when omitted, the mock returns a deterministic
 * "happy path" default (CONTINUE for reserve, ACCEPTED for trace events,
 * synthetic session for handshake).
 */
export interface MockSidecarHooks {
  /** Custom handshake handler. Default returns a fixed session id + bundle. */
  onHandshake?: (req: HandshakeRequest) => HandshakeResponse;
  /** Custom requestDecision handler. Default returns CONTINUE. */
  onRequestDecision?: (req: DecisionRequest) => DecisionResponse;
  /** Custom emitTraceEvents handler. Default acks every event as ACCEPTED. */
  onEmitTraceEvents?: (event: TraceEvent) => TraceEventAck;
  /**
   * Custom releaseReservation handler (SLICE 5). Default returns a
   * deterministic success with `release_id` derived from
   * `mock-release-${decisionCount}`. Throw a `ReleaseGrpcFailure` to surface
   * a gRPC Status (NOT_FOUND / FAILED_PRECONDITION / UNAVAILABLE) with an
   * optional trailer reason-code; the harness packs the reason into the
   * standard `x-spendguard-reason-code` trailer metadata field.
   */
  onReleaseReservation?: (req: ReleaseReservationRequest) => ReleaseReservationResponse;
}

/**
 * Helper to throw a gRPC Status from a mock RPC handler. The handler runtime
 * (Handshake / RequestDecision / ReleaseReservation) catches this, packs the
 * reason-code (when supplied AND `suppressTrailer` is not set) onto the
 * trailer `x-spendguard-reason-code` metadata, and forwards the status to
 * grpc-js. Tests use this for the typed-error-mapper coverage.
 *
 * **D05 SLICE 5 R2**: the production sidecar
 * (`services/sidecar/src/domain/error.rs::DomainError::to_status`) does NOT
 * set the trailer — it bakes the reason-code prefix into the Status message
 * string. Tests that need to exercise the production code path pass
 * `suppressTrailer: true` so the trailer is omitted and the SDK mapper
 * falls through to message-string parse.
 *
 * Usage:
 *
 *     // Mock-style: trailer set (legacy test convention).
 *     throw new ReleaseGrpcFailure(GrpcStatus.FAILED_PRECONDITION,
 *       "bundle hot-reloaded", "BUNDLE_HOT_RELOADED");
 *
 *     // Production-shape: message-only, no trailer.
 *     throw new ReleaseGrpcFailure(GrpcStatus.FAILED_PRECONDITION,
 *       "idempotency conflict: replay body diverged",
 *       undefined, { suppressTrailer: true });
 */
export class ReleaseGrpcFailure extends Error {
  readonly code: number;
  readonly reason?: string;
  readonly suppressTrailer: boolean;
  constructor(
    code: number,
    message: string,
    reason?: string,
    opts: { suppressTrailer?: boolean } = {},
  ) {
    super(message);
    this.name = "ReleaseGrpcFailure";
    this.code = code;
    if (reason !== undefined) {
      this.reason = reason;
    }
    this.suppressTrailer = opts.suppressTrailer === true;
  }
}

/** Default handshake response — CONTINUE-ready session with capability_required=L3. */
const DEFAULT_HANDSHAKE_RESPONSE: HandshakeResponse = {
  sidecarVersion: "mock-0.0.0",
  schemaBundle: {
    schemaBundleId: "mock-schema",
    schemaBundleHash: new Uint8Array([0xaa, 0xbb]),
    canonicalSchemaVersion: "spendguard.v1alpha1",
  },
  contractBundle: {
    bundleId: "mock-contract",
    bundleHash: new Uint8Array([0xcc, 0xdd]),
    bundleSignature: new Uint8Array([0xee]),
    signingKeyId: "mock-key-1",
  },
  capabilityRequired: HandshakeRequest_CapabilityLevel.L3_POLICY_HOOK,
  protocolVersion: 1,
  sessionId: "mock-session-1",
  signingKeyId: "mock-key-1",
  announcementSignature: new Uint8Array([0xff, 0x00, 0x01]),
};

/** Default decision response — CONTINUE with reserved id `mock-reservation-1`. */
const DEFAULT_DECISION_RESPONSE: DecisionResponse = {
  decisionId: "mock-decision-1",
  auditDecisionEventId: "mock-audit-1",
  decision: DecisionResponse_Decision.CONTINUE,
  reasonCodes: ["mock_allow"],
  matchedRuleIds: ["mock-rule-1"],
  mutationPatchJson: "",
  effectHash: new Uint8Array([0x10, 0x20]),
  ledgerTransactionId: "mock-tx-1",
  reservationIds: ["mock-reservation-1"],
  ttlExpiresAt: { seconds: "0", nanos: 0 },
  approvalRequestId: "",
  approverRole: "",
  terminal: false,
  runCodeTriggered: "",
};

/** Convenience: build a DecisionResponse with a STOP outcome. */
export function makeStopResponse(
  args: {
    decisionId?: string;
    reasonCodes?: string[];
    matchedRuleIds?: string[];
  } = {},
): DecisionResponse {
  return {
    ...DEFAULT_DECISION_RESPONSE,
    decisionId: args.decisionId ?? "mock-decision-stop",
    decision: DecisionResponse_Decision.STOP,
    reasonCodes: args.reasonCodes ?? ["mock_deny", "budget_exhausted"],
    matchedRuleIds: args.matchedRuleIds ?? ["mock-rule-deny"],
    reservationIds: [],
    ledgerTransactionId: "",
    terminal: true,
  };
}

/** Convenience: build a DecisionResponse with a DEGRADE outcome. */
export function makeDegradeResponse(args: { decisionId?: string } = {}): DecisionResponse {
  return {
    ...DEFAULT_DECISION_RESPONSE,
    decisionId: args.decisionId ?? "mock-decision-degrade",
    decision: DecisionResponse_Decision.DEGRADE,
    mutationPatchJson: '[{"op":"replace","path":"/model","value":"gpt-4o-mini"}]',
    reasonCodes: ["mock_degrade", "budget_threshold"],
    matchedRuleIds: ["mock-rule-degrade"],
  };
}

/** Default release response — success with synthetic signature + tx id. */
const DEFAULT_RELEASE_RESPONSE: ReleaseReservationResponse = {
  auditEventSignature: new Uint8Array([0x73, 0x69, 0x67]),
  ledgerTransactionId: "mock-release-tx-1",
  releasedReservationIds: ["mock-reservation-1"],
};

/**
 * Convenience: build a ReleaseReservationResponse with custom released ids.
 * Test helper for the release() success-path assertions.
 */
export function makeReleaseResponse(
  args: {
    auditEventSignature?: Uint8Array;
    ledgerTransactionId?: string;
    releasedReservationIds?: string[];
  } = {},
): ReleaseReservationResponse {
  return {
    auditEventSignature: args.auditEventSignature ?? DEFAULT_RELEASE_RESPONSE.auditEventSignature,
    ledgerTransactionId: args.ledgerTransactionId ?? DEFAULT_RELEASE_RESPONSE.ledgerTransactionId,
    releasedReservationIds: args.releasedReservationIds ?? [
      ...DEFAULT_RELEASE_RESPONSE.releasedReservationIds,
    ],
  };
}

/**
 * Mock UDS sidecar with full SidecarAdapter service.
 *
 * Usage:
 *
 *     const mock = await MockSidecar.start({
 *       onRequestDecision: (req) => makeDegradeResponse({ decisionId: req.ids?.decisionId }),
 *     });
 *     try { ... } finally { await mock.close(); }
 *
 * Or with `await using`:
 *
 *     await using mock = await MockSidecar.start();
 */
export class MockSidecar {
  /** The UDS path the server is bound to. Stable across the mock's lifetime. */
  readonly socketPath: string;
  /** Mutable hooks — tests can update mid-test if they need to flip behavior. */
  hooks: MockSidecarHooks;
  private readonly server: Server;
  private readonly socketDir: string;
  private bound = false;
  /** Tracks how many requestDecision calls have been served (idempotency tests). */
  private decisionCount = 0;
  /** Tracks how many handshake calls have been served (idempotency tests). */
  private handshakeCount = 0;
  /** Tracks how many emitTraceEvents events have been served. */
  private traceEventCount = 0;
  /** Tracks how many releaseReservation calls have been served (SLICE 5). */
  private releaseCount = 0;
  /**
   * Captured trace events from the most recent EmitTraceEvents call. Tests
   * assert against this to verify multi-event ordering (LLM_CALL_POST followed
   * by outcome event). Each new EmitTraceEvents call resets the array (so a
   * test only ever observes one call's-worth of events).
   */
  private capturedTraceEvents: TraceEvent[] = [];

  private constructor(socketPath: string, socketDir: string, hooks: MockSidecarHooks) {
    this.socketPath = socketPath;
    this.socketDir = socketDir;
    this.hooks = hooks;
    this.server = new Server();
  }

  /**
   * Start a fresh mock instance on a random UDS path under the system tempdir.
   * Pass `hooks` to override any of the default RPC behaviors.
   */
  static async start(hooks: MockSidecarHooks = {}): Promise<MockSidecar> {
    const dir = mkdtempSync(join(tmpdir(), "spendguard-mock-"));
    const path = join(dir, "adapter.sock");
    const mock = new MockSidecar(path, dir, hooks);
    mock.registerService();
    await mock.bind();
    return mock;
  }

  /** How many requestDecision calls have been served since start. */
  get decisionsServed(): number {
    return this.decisionCount;
  }

  /** How many handshake calls have been served since start. */
  get handshakesServed(): number {
    return this.handshakeCount;
  }

  /** How many emitTraceEvents events have been served since start. */
  get traceEventsServed(): number {
    return this.traceEventCount;
  }

  /** How many releaseReservation calls have been served since start (SLICE 5). */
  get releasesServed(): number {
    return this.releaseCount;
  }

  /**
   * Snapshot of the events captured during the most recent EmitTraceEvents
   * call. Each entry is a deep-cloned `TraceEvent` so the caller can inspect
   * payload fields without racing the next call.
   */
  get lastEmittedTraceEvents(): readonly TraceEvent[] {
    return this.capturedTraceEvents;
  }

  /**
   * Register the SidecarAdapter service on `this.server`. The method paths must
   * exactly match the protobuf-ts client's request URI — see
   * `node_modules/@protobuf-ts/grpc-transport/build/es2015/grpc-transport.js`
   * `makeUnaryRequest(\`/${typeName}/${methodName}\`, ...)`.
   */
  private registerService(): void {
    const typeName = "spendguard.sidecar_adapter.v1.SidecarAdapter";
    this.server.addService(
      {
        Handshake: {
          path: `/${typeName}/Handshake`,
          requestStream: false,
          responseStream: false,
          requestDeserialize: (buf: Buffer) => HandshakeRequest.fromBinary(buf),
          requestSerialize: (msg: HandshakeRequest) => Buffer.from(HandshakeRequest.toBinary(msg)),
          responseSerialize: (msg: HandshakeResponse) =>
            Buffer.from(HandshakeResponse.toBinary(msg)),
          responseDeserialize: (buf: Buffer) => HandshakeResponse.fromBinary(buf),
        },
        RequestDecision: {
          path: `/${typeName}/RequestDecision`,
          requestStream: false,
          responseStream: false,
          requestDeserialize: (buf: Buffer) => DecisionRequest.fromBinary(buf),
          requestSerialize: (msg: DecisionRequest) => Buffer.from(DecisionRequest.toBinary(msg)),
          responseSerialize: (msg: DecisionResponse) => Buffer.from(DecisionResponse.toBinary(msg)),
          responseDeserialize: (buf: Buffer) => DecisionResponse.fromBinary(buf),
        },
        EmitTraceEvents: {
          path: `/${typeName}/EmitTraceEvents`,
          requestStream: true,
          responseStream: true,
          requestDeserialize: (buf: Buffer) => TraceEvent.fromBinary(buf),
          requestSerialize: (msg: TraceEvent) => Buffer.from(TraceEvent.toBinary(msg)),
          responseSerialize: (msg: TraceEventAck) => Buffer.from(TraceEventAck.toBinary(msg)),
          responseDeserialize: (buf: Buffer) => TraceEventAck.fromBinary(buf),
        },
        ReleaseReservation: {
          path: `/${typeName}/ReleaseReservation`,
          requestStream: false,
          responseStream: false,
          requestDeserialize: (buf: Buffer) => ReleaseReservationRequest.fromBinary(buf),
          requestSerialize: (msg: ReleaseReservationRequest) =>
            Buffer.from(ReleaseReservationRequest.toBinary(msg)),
          responseSerialize: (msg: ReleaseReservationResponse) =>
            Buffer.from(ReleaseReservationResponse.toBinary(msg)),
          responseDeserialize: (buf: Buffer) => ReleaseReservationResponse.fromBinary(buf),
        },
      },
      {
        Handshake: (
          call: ServerUnaryCall<HandshakeRequest, HandshakeResponse>,
          callback: sendUnaryData<HandshakeResponse>,
        ) => {
          this.handshakeCount += 1;
          try {
            const handler = this.hooks.onHandshake;
            const response = handler ? handler(call.request) : DEFAULT_HANDSHAKE_RESPONSE;
            callback(null, response);
          } catch (err) {
            callback({
              code: GrpcStatus.UNKNOWN,
              details: err instanceof Error ? err.message : String(err),
              metadata: emptyMetadata(),
              name: "Error",
              message: err instanceof Error ? err.message : String(err),
            });
          }
        },
        RequestDecision: (
          call: ServerUnaryCall<DecisionRequest, DecisionResponse>,
          callback: sendUnaryData<DecisionResponse>,
        ) => {
          this.decisionCount += 1;
          try {
            const handler = this.hooks.onRequestDecision;
            const response = handler ? handler(call.request) : DEFAULT_DECISION_RESPONSE;
            callback(null, response);
          } catch (err) {
            callback({
              code: GrpcStatus.UNKNOWN,
              details: err instanceof Error ? err.message : String(err),
              metadata: emptyMetadata(),
              name: "Error",
              message: err instanceof Error ? err.message : String(err),
            });
          }
        },
        EmitTraceEvents: (call: ServerDuplexStream<TraceEvent, TraceEventAck>) => {
          // SLICE 5: reset capture array at the start of each new call so the
          // multi-event capture only ever shows one call's-worth of events.
          this.capturedTraceEvents = [];
          const handler = this.hooks.onEmitTraceEvents;
          call.on("data", (event: TraceEvent) => {
            this.traceEventCount += 1;
            this.capturedTraceEvents.push(event);
            try {
              const ack = handler
                ? handler(event)
                : {
                    eventId: `mock-event-${this.traceEventCount}`,
                    status: TraceEventAck_Status.ACCEPTED,
                  };
              call.write(ack);
            } catch (err) {
              call.write({
                eventId: `mock-event-${this.traceEventCount}`,
                status: TraceEventAck_Status.REJECTED,
                error: {
                  code: 13 /* RESERVATION_STATE_CONFLICT — synthetic for tests */,
                  message: err instanceof Error ? err.message : String(err),
                  details: {},
                },
              });
            }
          });
          call.on("end", () => {
            call.end();
          });
          call.on("error", () => {
            // grpc-js may surface CANCELLED here; we don't need to do anything
            // — the call is already torn down. Test cleanup uses
            // `close()` to force-shutdown.
          });
        },
        ReleaseReservation: (
          call: ServerUnaryCall<ReleaseReservationRequest, ReleaseReservationResponse>,
          callback: sendUnaryData<ReleaseReservationResponse>,
        ) => {
          this.releaseCount += 1;
          try {
            const handler = this.hooks.onReleaseReservation;
            const response = handler
              ? handler(call.request)
              : {
                  ...DEFAULT_RELEASE_RESPONSE,
                  releasedReservationIds: [call.request.reservationId],
                };
            callback(null, response);
          } catch (err) {
            if (err instanceof ReleaseGrpcFailure) {
              const trailers = new Metadata();
              if (err.reason !== undefined && !err.suppressTrailer) {
                trailers.set("x-spendguard-reason-code", err.reason);
              }
              callback({
                code: err.code,
                details: err.message,
                metadata: trailers,
                name: err.name,
                message: err.message,
              });
              return;
            }
            callback({
              code: GrpcStatus.UNKNOWN,
              details: err instanceof Error ? err.message : String(err),
              metadata: emptyMetadata(),
              name: "Error",
              message: err instanceof Error ? err.message : String(err),
            });
          }
        },
      },
    );
  }

  private async bind(): Promise<void> {
    return new Promise<void>((resolve, reject) => {
      this.server.bindAsync(
        `unix:${this.socketPath}`,
        ServerCredentials.createInsecure(),
        (err) => {
          if (err) {
            reject(err);
            return;
          }
          this.bound = true;
          resolve();
        },
      );
    });
  }

  /** Whether the server is currently bound. */
  get isBound(): boolean {
    return this.bound;
  }

  /**
   * Stop the server and clean up the temp socket / dir. Idempotent.
   *
   * Implements `[Symbol.asyncDispose]` so callers can write
   * `await using mock = await MockSidecar.start()` and rely on cleanup.
   */
  async close(): Promise<void> {
    if (this.bound) {
      await new Promise<void>((resolve) => {
        this.server.tryShutdown((err) => {
          if (err) {
            this.server.forceShutdown();
          }
          resolve();
        });
      });
      this.bound = false;
    }
    try {
      if (existsSync(this.socketPath)) {
        rmSync(this.socketPath, { force: true });
      }
      rmSync(this.socketDir, { recursive: true, force: true });
    } catch {
      // ignore — cleanup is best-effort.
    }
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.close();
  }
}

/** Build an empty `Metadata` object for synthetic error responses. */
function emptyMetadata(): Metadata {
  return new Metadata();
}
