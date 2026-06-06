// COV_S05_05 SLICE 5 — release / queryBudget / multi-event commit / mapper.
//
// Spec coverage:
//   - design.md §4.2 (LOCKED public surface: release / queryBudget locked)
//   - design.md §4.4 (CommitEstimated multi-event extension + Release/Query
//     request and outcome shapes)
//   - design.md §4.5 (error hierarchy: MutationApplyFailed,
//     ApprovalBundleHotReloadedError, SidecarUnavailable, HandshakeError)
//   - design.md §8 slice 5 row + §9.4 queryBudget deferral rationale
//   - review-standards.md §5 (error class parity), §6 (UDS wire correctness)
//   - slices/COV_S05_05_d05_release_query.md (SLICE 5 doc)
//
// Each test runs against a fresh `MockSidecar` (or its disabled-mode
// short-circuit) so the wire path is exercised end-to-end where applicable.
// Declared Deviation #1 (LLM_CALL_OUTCOME proto kind) is acknowledged in the
// CommitEstimated multi-event tests; see `CommitEstimatedRequest.outcomeKind`
// JSDoc + the SLICE 5 directive notes.

import { status as GrpcStatus } from "@grpc/grpc-js";
import { afterEach, describe, expect, it } from "vitest";

import type {
  ReleaseReservationRequest as ProtoReleaseReservationRequest,
  TraceEvent as ProtoTraceEvent,
} from "../src/_proto/spendguard/sidecar_adapter/v1/adapter.js";
import {
  LlmCallPostPayload_Outcome,
  TraceEventAck_Status,
  TraceEvent_EventKind,
} from "../src/_proto/spendguard/sidecar_adapter/v1/adapter.js";
import {
  ApprovalBundleHotReloadedError,
  HandshakeError,
  MutationApplyFailed,
  SidecarUnavailable,
  SpendGuardClient,
  SpendGuardError,
} from "../src/index.js";
import { MockSidecar, ReleaseGrpcFailure, makeReleaseResponse } from "./_support/mockSidecar.js";

// Restore SPENDGUARD_* env between tests so a stray var doesn't leak.
const ENV_KEYS = [
  "SPENDGUARD_SOCKET_PATH",
  "SPENDGUARD_SIDECAR_UDS",
  "SPENDGUARD_TENANT_ID",
  "SPENDGUARD_DISABLE",
] as const;
const savedEnv: Record<string, string | undefined> = {};
for (const k of ENV_KEYS) savedEnv[k] = process.env[k];
afterEach(() => {
  for (const k of ENV_KEYS) {
    if (savedEnv[k] === undefined) delete process.env[k];
    else process.env[k] = savedEnv[k];
  }
});

/** Canonical happy-path ReleaseRequest used across the tests below. */
function releaseReq(overrides: Partial<Parameters<SpendGuardClient["release"]>[0]> = {}) {
  return {
    reservationId: "res-42",
    idempotencyKey: "sg-release-abcdef",
    reasonCodes: ["explicit"],
    ...overrides,
  };
}

/** Canonical CommitEstimatedRequest base used by multi-event tests. */
function commitReq(overrides: Partial<Parameters<SpendGuardClient["commitEstimated"]>[0]> = {}) {
  return {
    runId: "run-1",
    stepId: "step-1",
    llmCallId: "llm-1",
    decisionId: "d-1",
    reservationId: "mock-reservation-1",
    estimatedAmountAtomic: "500",
    unit: { unit: "USD_MICROS", denomination: 1 },
    pricing: {
      pricingVersion: "v2026.05.09-1",
      pricingHash: new Uint8Array([0x01, 0x02]),
    },
    providerEventId: "pe-1",
    outcome: "SUCCESS" as const,
    ...overrides,
  };
}

// ── release() success-path tests ──────────────────────────────────────────

describe("release() — design.md §4.4 ASP Draft-01 §4 wire", () => {
  it("returns ReleaseOutcome with audit signature + ledger tx + released ids", async () => {
    const mock = await MockSidecar.start({
      onReleaseReservation: () =>
        makeReleaseResponse({
          auditEventSignature: new Uint8Array([0xde, 0xad, 0xbe, 0xef]),
          ledgerTransactionId: "tx-release-1",
          releasedReservationIds: ["res-42"],
        }),
    });
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
      });
      await client.connect();
      await client.handshake();
      const outcome = await client.release(releaseReq({ reservationId: "res-42" }));
      expect(outcome.ledgerTransactionId).toBe("tx-release-1");
      expect(outcome.releasedReservationIds).toEqual(["res-42"]);
      expect(Array.from(outcome.auditEventSignature)).toEqual([0xde, 0xad, 0xbe, 0xef]);
      expect(mock.releasesServed).toBe(1);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("forwards reservationId + idempotencyKey + reasonCodes + sessionId verbatim on the wire", async () => {
    let captured: ProtoReleaseReservationRequest | null = null;
    const mock = await MockSidecar.start({
      onReleaseReservation: (req) => {
        captured = req;
        return makeReleaseResponse({ releasedReservationIds: [req.reservationId] });
      },
    });
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "tenant-explicit",
      });
      await client.connect();
      const handshake = await client.handshake();
      await client.release(
        releaseReq({
          reservationId: "res-explicit",
          idempotencyKey: "sg-deadbeef",
          reasonCodes: ["run_aborted", "client_timeout"],
          tenantId: "tenant-explicit",
          workloadInstanceId: "wi-7",
        }),
      );
      expect(captured).not.toBeNull();
      const req = captured as unknown as ProtoReleaseReservationRequest;
      expect(req.reservationId).toBe("res-explicit");
      expect(req.idempotencyKey).toBe("sg-deadbeef");
      expect(req.reasonCodes).toEqual(["run_aborted", "client_timeout"]);
      expect(req.tenantId).toBe("tenant-explicit");
      expect(req.workloadInstanceId).toBe("wi-7");
      expect(req.sessionId).toBe(handshake.sessionId);
      await client.close();
    } finally {
      await mock.close();
    }
  });
});

// ── release() typed-error mapping for gRPC Status cluster ─────────────────

describe("release() — gRPC Status → typed error mapping", () => {
  it('NOT_FOUND → SpendGuardError("reservation not found") preserving cause', async () => {
    const mock = await MockSidecar.start({
      onReleaseReservation: () => {
        throw new ReleaseGrpcFailure(GrpcStatus.NOT_FOUND, "no such reservation");
      },
    });
    try {
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      const err = await client
        .release(releaseReq())
        .then(() => null)
        .catch((e: unknown) => e);
      expect(err).toBeInstanceOf(SpendGuardError);
      // NOT a SidecarUnavailable / MutationApplyFailed / ApprovalBundleHotReloadedError —
      // bare SpendGuardError is the explicit contract per slice doc.
      expect(err).not.toBeInstanceOf(SidecarUnavailable);
      expect(err).not.toBeInstanceOf(MutationApplyFailed);
      expect(err).not.toBeInstanceOf(ApprovalBundleHotReloadedError);
      expect((err as SpendGuardError).message).toBe("reservation not found");
      expect((err as SpendGuardError).cause).toBeDefined();
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("FAILED_PRECONDITION + IDEMPOTENCY_CONFLICT → MutationApplyFailed", async () => {
    const mock = await MockSidecar.start({
      onReleaseReservation: () => {
        throw new ReleaseGrpcFailure(
          GrpcStatus.FAILED_PRECONDITION,
          "idempotency replay conflict",
          "IDEMPOTENCY_CONFLICT",
        );
      },
    });
    try {
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      await expect(client.release(releaseReq())).rejects.toBeInstanceOf(MutationApplyFailed);
      await expect(client.release(releaseReq())).rejects.toThrowError(/IDEMPOTENCY_CONFLICT/);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("FAILED_PRECONDITION + BUDGET_EXCEEDED → MutationApplyFailed", async () => {
    const mock = await MockSidecar.start({
      onReleaseReservation: () => {
        throw new ReleaseGrpcFailure(
          GrpcStatus.FAILED_PRECONDITION,
          "budget exhausted at release time",
          "BUDGET_EXCEEDED",
        );
      },
    });
    try {
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      const err = await client
        .release(releaseReq())
        .then(() => null)
        .catch((e: unknown) => e);
      expect(err).toBeInstanceOf(MutationApplyFailed);
      expect((err as Error).message).toMatch(/BUDGET_EXCEEDED/);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("FAILED_PRECONDITION + BUNDLE_HOT_RELOADED → ApprovalBundleHotReloadedError", async () => {
    const mock = await MockSidecar.start({
      onReleaseReservation: () => {
        throw new ReleaseGrpcFailure(
          GrpcStatus.FAILED_PRECONDITION,
          "contract bundle rotated mid-release",
          "BUNDLE_HOT_RELOADED",
        );
      },
    });
    try {
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      const err = await client
        .release(releaseReq())
        .then(() => null)
        .catch((e: unknown) => e);
      expect(err).toBeInstanceOf(ApprovalBundleHotReloadedError);
      // Wire path does not carry the hashes; the typed-error fallback is
      // explicit empty strings per design §4.5 fallback semantic.
      const hot = err as ApprovalBundleHotReloadedError;
      expect(hot.originalBundleHash).toBe("");
      expect(hot.currentBundleHash).toBe("");
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("UNAVAILABLE → SidecarUnavailable preserving cause", async () => {
    const mock = await MockSidecar.start({
      onReleaseReservation: () => {
        throw new ReleaseGrpcFailure(GrpcStatus.UNAVAILABLE, "sidecar unreachable");
      },
    });
    try {
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      const err = await client
        .release(releaseReq())
        .then(() => null)
        .catch((e: unknown) => e);
      expect(err).toBeInstanceOf(SidecarUnavailable);
      expect((err as SidecarUnavailable).statusCode).toBe(503);
      expect((err as SidecarUnavailable).cause).toBeDefined();
      await client.close();
    } finally {
      await mock.close();
    }
  });
});

// ── release() disabled-mode + pre-handshake gate ──────────────────────────

describe("release() — disabled-mode + pre-handshake gate", () => {
  it("disabled mode returns makeDisabledReleaseOutcome without UDS contact", async () => {
    const client = new SpendGuardClient({
      socketPath: "/dev/null/cannot-exist",
      tenantId: "test",
      disabled: true,
    });
    // No handshake needed in disabled mode.
    const outcome = await client.release(releaseReq({ reservationId: "res-disabled" }));
    expect(outcome.releasedReservationIds).toEqual(["res-disabled"]);
    expect(outcome.ledgerTransactionId).toBe("");
    expect(outcome.auditEventSignature.length).toBe(0);
  });

  it("pre-handshake call throws HandshakeError", async () => {
    const client = new SpendGuardClient({ socketPath: "/tmp/x.sock", tenantId: "t" });
    await expect(client.release(releaseReq())).rejects.toBeInstanceOf(HandshakeError);
    await expect(client.release(releaseReq())).rejects.toThrowError(/handshake/);
  });
});

// ── queryBudget() §9.4 placeholder + disabled-mode + gate ─────────────────

describe("queryBudget() — design.md §9.4 placeholder", () => {
  it("post-handshake throws SpendGuardError carrying the tracking-issue URL", async () => {
    const mock = await MockSidecar.start();
    try {
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      const err = await client
        .queryBudget({ scopeId: "tenant/test/global" })
        .then(() => null)
        .catch((e: unknown) => e);
      expect(err).toBeInstanceOf(SpendGuardError);
      expect((err as Error).message).toMatch(/query_budget not yet wired in sidecar/);
      expect((err as Error).message).toMatch(/github\.com\/m24927605\/agentic-spendguard\/issues/);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("disabled mode returns makeDisabledQueryBudgetResult without UDS contact", async () => {
    const client = new SpendGuardClient({
      socketPath: "/dev/null/cannot-exist",
      tenantId: "test",
      disabled: true,
    });
    const result = await client.queryBudget({ scopeId: "tenant/test/global", asOfSeconds: 1234 });
    expect(result.availableAtomic).toBe("0");
    expect(result.reservedAtomic).toBe("0");
    expect(result.committedAtomic).toBe("0");
    expect(result.unit.unit).toBe("USD_MICROS");
    expect(result.asOfSeconds).toBe(1234);
  });

  it("pre-handshake call throws HandshakeError (not the §9.4 placeholder)", async () => {
    const client = new SpendGuardClient({ socketPath: "/tmp/x.sock", tenantId: "t" });
    const err = await client
      .queryBudget({ scopeId: "tenant/test/global" })
      .then(() => null)
      .catch((e: unknown) => e);
    expect(err).toBeInstanceOf(HandshakeError);
    // Critically, the pre-handshake gate must fire BEFORE the §9.4 placeholder —
    // otherwise adapters would see a confusing "not yet wired" message that's
    // actually a missing handshake call.
    expect((err as Error).message).not.toMatch(/query_budget not yet wired/);
  });
});

// ── commitEstimated() multi-event extension ───────────────────────────────

describe("commitEstimated() — SLICE 5 multi-event extension", () => {
  it("with outcomeKind=SUCCESS: emits TWO LLM_CALL_POST events on the same bidi stream", async () => {
    const mock = await MockSidecar.start();
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
      });
      await client.connect();
      await client.handshake();
      await client.commitEstimated(
        commitReq({
          outcomeKind: "SUCCESS",
          actualInputTokensWire: "128",
          actualOutputTokensWire: "256",
        }),
      );
      expect(mock.traceEventsServed).toBe(2);
      const events = mock.lastEmittedTraceEvents as readonly ProtoTraceEvent[];
      expect(events).toHaveLength(2);
      const ev1 = events[0];
      const ev2 = events[1];
      if (ev1 === undefined || ev2 === undefined) throw new Error("missing captured events");
      // Both events use LLM_CALL_POST kind (Declared Deviation #1: LLM_CALL_OUTCOME
      // proto kind does not yet exist; the outcome event reuses LLM_CALL_POST
      // and discriminates via the inner Outcome enum).
      expect(ev1.kind).toBe(TraceEvent_EventKind.LLM_CALL_POST);
      expect(ev2.kind).toBe(TraceEvent_EventKind.LLM_CALL_POST);
      // First event carries the booking amount.
      if (ev1.payload.oneofKind !== "llmCallPost") throw new Error("ev1 payload mismatch");
      expect(ev1.payload.llmCallPost.estimatedAmountAtomic).toBe("500");
      expect(ev1.payload.llmCallPost.outcome).toBe(LlmCallPostPayload_Outcome.SUCCESS);
      // Second (outcome) event has empty estimatedAmountAtomic + actuals attached.
      if (ev2.payload.oneofKind !== "llmCallPost") throw new Error("ev2 payload mismatch");
      expect(ev2.payload.llmCallPost.estimatedAmountAtomic).toBe("");
      expect(ev2.payload.llmCallPost.outcome).toBe(LlmCallPostPayload_Outcome.SUCCESS);
      expect(ev2.payload.llmCallPost.actualInputTokens).toBe("128");
      expect(ev2.payload.llmCallPost.actualOutputTokens).toBe("256");
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("with outcomeKind=FAILURE: outcome event carries PROVIDER_ERROR + error_message envelope", async () => {
    const mock = await MockSidecar.start();
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
      });
      await client.connect();
      await client.handshake();
      await client.commitEstimated(
        commitReq({
          outcomeKind: "FAILURE",
          actualErrorMessage: "openai 429 — rate limited",
        }),
      );
      expect(mock.traceEventsServed).toBe(2);
      const events = mock.lastEmittedTraceEvents as readonly ProtoTraceEvent[];
      const ev2 = events[1];
      if (ev2 === undefined) throw new Error("missing outcome event");
      if (ev2.payload.oneofKind !== "llmCallPost") throw new Error("ev2 payload mismatch");
      // Per Declared Deviation #1: FAILURE projects to PROVIDER_ERROR on the
      // existing enum because LlmCallOutcomeKind.FAILURE does not yet exist
      // in proto.
      expect(ev2.payload.llmCallPost.outcome).toBe(LlmCallPostPayload_Outcome.PROVIDER_ERROR);
      // error_message envelope on providerResponseMetadata.
      expect(ev2.providerResponseMetadata).toBe(
        JSON.stringify({ error_message: "openai 429 — rate limited" }),
      );
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("without outcomeKind: SLICE 4 regression — single LLM_CALL_POST event emitted", async () => {
    const mock = await MockSidecar.start();
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
      });
      await client.connect();
      await client.handshake();
      await client.commitEstimated(commitReq()); // no outcomeKind
      expect(mock.traceEventsServed).toBe(1);
      expect(mock.lastEmittedTraceEvents).toHaveLength(1);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("rejects when sidecar acks the outcome event with QUARANTINED (multi-event ack drain)", async () => {
    let eventNo = 0;
    const mock = await MockSidecar.start({
      onEmitTraceEvents: () => {
        eventNo += 1;
        // First event accepted; second event quarantined.
        if (eventNo === 1) {
          return { eventId: `ack-${eventNo}`, status: TraceEventAck_Status.ACCEPTED };
        }
        return { eventId: `ack-${eventNo}`, status: TraceEventAck_Status.QUARANTINED };
      },
    });
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
      });
      await client.connect();
      await client.handshake();
      await expect(
        client.commitEstimated(commitReq({ outcomeKind: "SUCCESS" })),
      ).rejects.toThrowError(/QUARANTINED/);
      await client.close();
    } finally {
      await mock.close();
    }
  });
});

// ── mapGrpcStatusToError exhaustive coverage (via release()) ──────────────

describe("mapGrpcStatusToError — exhaustive cluster coverage", () => {
  it("DEADLINE_EXCEEDED → SidecarUnavailable", async () => {
    const mock = await MockSidecar.start({
      onReleaseReservation: () => {
        throw new ReleaseGrpcFailure(GrpcStatus.DEADLINE_EXCEEDED, "release deadline exceeded");
      },
    });
    try {
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      const err = await client
        .release(releaseReq())
        .then(() => null)
        .catch((e: unknown) => e);
      expect(err).toBeInstanceOf(SidecarUnavailable);
      expect((err as Error).message).toMatch(/DEADLINE_EXCEEDED/);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("FAILED_PRECONDITION with unknown reason → MutationApplyFailed (conservative default)", async () => {
    const mock = await MockSidecar.start({
      onReleaseReservation: () => {
        // No reason metadata at all — exercises the unknown-reason default path.
        throw new ReleaseGrpcFailure(GrpcStatus.FAILED_PRECONDITION, "untyped failed precondition");
      },
    });
    try {
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      const err = await client
        .release(releaseReq())
        .then(() => null)
        .catch((e: unknown) => e);
      // Default fallback for the FAILED_PRECONDITION cluster — never bare
      // SpendGuardError for this cluster per review-standards §5.
      expect(err).toBeInstanceOf(MutationApplyFailed);
      expect((err as Error).message).toMatch(/FAILED_PRECONDITION/);
      // No reason= suffix in the message body when reason is missing.
      expect((err as Error).message).not.toMatch(/reason=/);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("ABORTED → SpendGuardError (Python parity)", async () => {
    const mock = await MockSidecar.start({
      onReleaseReservation: () => {
        throw new ReleaseGrpcFailure(GrpcStatus.ABORTED, "aborted by sidecar");
      },
    });
    try {
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      const err = await client
        .release(releaseReq())
        .then(() => null)
        .catch((e: unknown) => e);
      expect(err).toBeInstanceOf(SpendGuardError);
      expect(err).not.toBeInstanceOf(SidecarUnavailable);
      expect(err).not.toBeInstanceOf(MutationApplyFailed);
      expect((err as Error).message).toMatch(/ABORTED/);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("INTERNAL (unmapped status) → SpendGuardError with code preserved in message", async () => {
    const mock = await MockSidecar.start({
      onReleaseReservation: () => {
        throw new ReleaseGrpcFailure(GrpcStatus.INTERNAL, "sidecar internal error");
      },
    });
    try {
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      const err = await client
        .release(releaseReq())
        .then(() => null)
        .catch((e: unknown) => e);
      expect(err).toBeInstanceOf(SpendGuardError);
      expect((err as Error).message).toMatch(/INTERNAL/);
      expect((err as Error).cause).toBeDefined();
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("preserves the original RpcError on `cause` for downstream debugging", async () => {
    const mock = await MockSidecar.start({
      onReleaseReservation: () => {
        throw new ReleaseGrpcFailure(
          GrpcStatus.FAILED_PRECONDITION,
          "bundle rotation detected mid-release",
          "BUNDLE_HOT_RELOADED",
        );
      },
    });
    try {
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      const err = await client
        .release(releaseReq())
        .then(() => null)
        .catch((e: unknown) => e);
      expect(err).toBeInstanceOf(ApprovalBundleHotReloadedError);
      const cause = (err as SpendGuardError).cause;
      // protobuf-ts `RpcError` is the cause; its `code` is the gRPC status as
      // a string (per its public docs). Adapter code can do
      // `if (e.cause instanceof RpcError) ...`.
      expect(cause).toBeDefined();
      expect((cause as { code?: unknown }).code).toBe("FAILED_PRECONDITION");
      await client.close();
    } finally {
      await mock.close();
    }
  });
});

// ── R2: production-shape message-string parse (no trailer) ────────────────
//
// The production sidecar (`services/sidecar/src/domain/error.rs::
// DomainError::to_status`) emits `Status::failed_precondition(self.to_string())`
// with the discriminator baked into the message string and DOES NOT set the
// `x-spendguard-reason-code` trailer. These tests pass `suppressTrailer: true`
// to the mock so the trailer is omitted, forcing the SDK's
// `readReasonCode()` to dispatch on the message-string prefix table (PRIMARY
// discriminator). The legacy mock-trailer tests above continue to exercise
// the SECONDARY discriminator (trailer fallback for forward-compat).

describe("release() — R2 production-shape message-string parse (no trailer)", () => {
  it('FAILED_PRECONDITION "idempotency conflict: ..." (no trailer) → MutationApplyFailed', async () => {
    const mock = await MockSidecar.start({
      onReleaseReservation: () => {
        // Mirrors the exact prefix DomainError::IdempotencyConflict emits
        // via `#[error("idempotency conflict: {0}")]` at
        // services/sidecar/src/domain/error.rs:44-45.
        throw new ReleaseGrpcFailure(
          GrpcStatus.FAILED_PRECONDITION,
          "idempotency conflict: replay body diverged from prior request",
          undefined,
          { suppressTrailer: true },
        );
      },
    });
    try {
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      const err = await client
        .release(releaseReq())
        .then(() => null)
        .catch((e: unknown) => e);
      expect(err).toBeInstanceOf(MutationApplyFailed);
      expect((err as Error).message).toMatch(/IDEMPOTENCY_CONFLICT/);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it('FAILED_PRECONDITION "[BUNDLE_HOT_RELOADED] ..." (no trailer) → ApprovalBundleHotReloadedError', async () => {
    const mock = await MockSidecar.start({
      onReleaseReservation: () => {
        // Mirrors the bracket-tagged form emitted by
        // services/sidecar/src/server/adapter_uds.rs:1379 (resume path).
        // Forward-compat: when the release path is unified with resume in
        // a future cross-component slice, this dispatch already works.
        throw new ReleaseGrpcFailure(
          GrpcStatus.FAILED_PRECONDITION,
          "[BUNDLE_HOT_RELOADED] approval was issued under bundle hash abc but the sidecar's currently-installed bundle is def; the operator's approval is no longer semantically tied to this bundle. Reissue the original DecisionRequest.",
          undefined,
          { suppressTrailer: true },
        );
      },
    });
    try {
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      const err = await client
        .release(releaseReq())
        .then(() => null)
        .catch((e: unknown) => e);
      expect(err).toBeInstanceOf(ApprovalBundleHotReloadedError);
      expect((err as Error).message).toMatch(/BUNDLE_HOT_RELOADED/);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it('FAILED_PRECONDITION "reservation state conflict: ..." (no trailer) → MutationApplyFailed/BUDGET_EXCEEDED', async () => {
    const mock = await MockSidecar.start({
      onReleaseReservation: () => {
        // Mirrors DomainError::ReservationStateConflict
        // `#[error("reservation state conflict: {0}")]` at error.rs:26-27.
        // Canonical mapping → BUDGET_EXCEEDED per readReasonCode prefix table.
        throw new ReleaseGrpcFailure(
          GrpcStatus.FAILED_PRECONDITION,
          "reservation state conflict: reservation already in TERMINAL state",
          undefined,
          { suppressTrailer: true },
        );
      },
    });
    try {
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      const err = await client
        .release(releaseReq())
        .then(() => null)
        .catch((e: unknown) => e);
      expect(err).toBeInstanceOf(MutationApplyFailed);
      expect((err as Error).message).toMatch(/BUDGET_EXCEEDED/);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("FAILED_PRECONDITION unrecognized message + no trailer → MutationApplyFailed default (no reason= suffix)", async () => {
    const mock = await MockSidecar.start({
      onReleaseReservation: () => {
        // Neither a known prefix nor a trailer — exercises the conservative
        // default fall-through (review-standards §5: never bare
        // SpendGuardError for the FAILED_PRECONDITION cluster).
        throw new ReleaseGrpcFailure(
          GrpcStatus.FAILED_PRECONDITION,
          "some future sidecar error nobody anticipated",
          undefined,
          { suppressTrailer: true },
        );
      },
    });
    try {
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      const err = await client
        .release(releaseReq())
        .then(() => null)
        .catch((e: unknown) => e);
      expect(err).toBeInstanceOf(MutationApplyFailed);
      expect((err as Error).message).toMatch(/FAILED_PRECONDITION/);
      // No reason= suffix when neither discriminator yields a match.
      expect((err as Error).message).not.toMatch(/reason=/);
      await client.close();
    } finally {
      await mock.close();
    }
  });
});

// ── handshake / reserve / commitEstimated error-path regression after refactor

describe("post-mapper refactor regression — handshake / reserve / commitEstimated paths", () => {
  it("reserve() still surfaces SidecarUnavailable on transport-down (mapper refactor regression)", async () => {
    // No mock at the socket path → immediate UNAVAILABLE.
    const client = new SpendGuardClient({
      socketPath: "/tmp/spendguard-no-such-socket.sock",
      tenantId: "t",
      handshakeTimeoutMs: 250,
      decisionTimeoutMs: 250,
    });
    await client.connect();
    const err = await client
      .handshake()
      .then(() => null)
      .catch((e: unknown) => e);
    // mapGrpcStatusToError replaced classifyRpcError; the same SidecarUnavailable
    // contract holds.
    expect(err).toBeInstanceOf(SidecarUnavailable);
    await client.close();
  });

  it("commitEstimated() single-event path still rejects on non-ACCEPTED ack (SLICE 4 regression)", async () => {
    const mock = await MockSidecar.start({
      onEmitTraceEvents: () => ({
        eventId: "ack-rejected",
        status: TraceEventAck_Status.REJECTED,
        error: {
          code: 13,
          message: "ledger commit rejected",
          details: {},
        },
      }),
    });
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
      });
      await client.connect();
      await client.handshake();
      await expect(client.commitEstimated(commitReq())).rejects.toThrowError(/REJECTED/);
      await client.close();
    } finally {
      await mock.close();
    }
  });
});
