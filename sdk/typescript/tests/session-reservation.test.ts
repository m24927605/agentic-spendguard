// D41S_01 — session reservation contract/proto skeleton tests.

import { describe, expect, it } from "vitest";

import { SidecarAdapterClient } from "../src/_proto/spendguard/sidecar_adapter/v1/adapter.client.js";
import {
  CommitSessionDeltaRequest,
  CommitSessionDeltaRequest_Outcome,
  ReleaseSessionRequest,
  ReserveSessionRequest,
  SidecarAdapter,
} from "../src/_proto/spendguard/sidecar_adapter/v1/adapter.js";
import type { PricingFreeze, UnitRef } from "../src/index.js";
import { SpendGuardClient } from "../src/index.js";
import type { HandshakeOutcome } from "../src/index.js";
import {
  SessionPendingDeltaLimitError,
  SessionReservationHandle,
  SessionReservationReleasedError,
  SessionReservationReplayMismatchError,
  buildCommitSessionDeltaRequest,
  buildReleaseSessionRequest,
  buildReserveSessionRequest,
} from "../src/session.js";
import type {
  CommitSessionDeltaOutcome,
  ReleaseSessionOutcome,
  ReserveSessionOutcome,
  CommitSessionDeltaRequest as SessionCommitDeltaRequest,
  ReleaseSessionRequest as SessionReleaseRequest,
} from "../src/session.js";

const UNIT: UnitRef = {
  unit: "USD_MICROS",
  denomination: 1,
  unitId: "018ff7d0-2c9a-7f28-8d25-cf9486b08d41",
};

const PRICING: PricingFreeze = {
  pricingVersion: "focus-v1.2-demo",
  pricingHash: new Uint8Array([0xa1, 0xb2, 0xc3]),
  fxRateVersion: "fx-2026-06-12",
  unitConversionVersion: "unitconv-2026-06-12",
};

const HANDSHAKE: HandshakeOutcome = {
  sessionId: "sidecar-handshake-session",
  sidecarVersion: "test-sidecar",
  schemaBundleId: "schema",
  schemaBundleHash: new Uint8Array(),
  contractBundleId: "contract",
  contractBundleHash: new Uint8Array(),
  capabilityRequired: 0,
  signingKeyId: "test-key",
  announcementSignature: new Uint8Array(),
};

function clientWithAdapter(adapter: unknown): SpendGuardClient {
  const client = new SpendGuardClient({
    socketPath: "/tmp/spendguard-session-test.sock",
    tenantId: "tenant-demo",
  });
  const mutable = client as unknown as {
    adapterClient: unknown;
    handshakeResult: HandshakeOutcome | null;
  };
  mutable.adapterClient = adapter;
  mutable.handshakeResult = HANDSHAKE;
  return client;
}

function acceptedCommitOutcome(
  req: SessionCommitDeltaRequest,
  cumulativeCommittedAtomic = req.amountAtomicDelta,
): CommitSessionDeltaOutcome {
  return {
    sessionReservationId: req.sessionReservationId,
    streamingCommitId: req.streamingCommitId,
    ledgerTransactionId: `lt-${req.streamingCommitId}`,
    auditSessionEventId: `audit-${req.streamingCommitId}`,
    committedDeltaAtomic: req.amountAtomicDelta,
    cumulativeCommittedAtomic,
    remainingAmountAtomic: "97500",
    recordedAt: new Date("2026-06-12T03:04:05.678Z"),
  };
}

describe("D41S_01 session reservation SR-V1 proto contract", () => {
  it("exposes ReserveSession, CommitSessionDelta, and ReleaseSession on SidecarAdapter", () => {
    const methodNames = SidecarAdapter.methods.map((m) => m.name);

    expect(methodNames).toContain("ReserveSession");
    expect(methodNames).toContain("CommitSessionDelta");
    expect(methodNames).toContain("ReleaseSession");
    expect(typeof SidecarAdapterClient.prototype.reserveSession).toBe("function");
    expect(typeof SidecarAdapterClient.prototype.commitSessionDelta).toBe("function");
    expect(typeof SidecarAdapterClient.prototype.releaseSession).toBe("function");
  });

  it("TP-D41S-10: builds ReserveSessionRequest with handshake session id and tuple", () => {
    const req = buildReserveSessionRequest({
      tenantId: "tenant-demo",
      budgetId: "budget-voice",
      windowInstanceId: "018ff7d0-2c9a-7f28-8d25-cf9486b08d42",
      unit: UNIT,
      pricing: PRICING,
      sessionId: "sidecar-handshake-session",
      route: "livekit|openai-realtime|gpt-4o-mini-transcribe",
      estimatedAmountAtomic: "100000",
      ttlSeconds: 600,
      idempotencyKey: "sg-d41s-reserve-1",
    });

    const decoded = ReserveSessionRequest.fromBinary(ReserveSessionRequest.toBinary(req));

    expect(decoded.tenantId).toBe("tenant-demo");
    expect(decoded.budgetId).toBe("budget-voice");
    expect(decoded.windowInstanceId).toBe("018ff7d0-2c9a-7f28-8d25-cf9486b08d42");
    expect(decoded.unit?.unitId).toBe(UNIT.unitId);
    expect(decoded.unit?.unitName).toBe("USD_MICROS");
    expect(decoded.pricing?.pricingVersion).toBe("focus-v1.2-demo");
    expect(decoded.pricing?.priceSnapshotHash).toEqual(PRICING.pricingHash);
    expect(decoded.pricing?.fxRateVersion).toBe("fx-2026-06-12");
    expect(decoded.pricing?.unitConversionVersion).toBe("unitconv-2026-06-12");
    expect(decoded.sessionId).toBe("sidecar-handshake-session");
    expect(decoded.route).toBe("livekit|openai-realtime|gpt-4o-mini-transcribe");
    expect(decoded.estimatedAmountAtomic).toBe("100000");
    expect(decoded.ttlSeconds).toBe(600);
    expect(decoded.idempotencyKey).toBe("sg-d41s-reserve-1");
  });

  it("TP-D41S-10: builds CommitSessionDeltaRequest with positive delta and event time", () => {
    const req = buildCommitSessionDeltaRequest({
      sessionReservationId: "sr-voice-1",
      streamingCommitId: "sr-voice-1/delta/000001",
      amountAtomicDelta: "2500",
      outcome: "SUCCESS",
      eventTime: new Date("2026-06-12T03:04:05.678Z"),
      idempotencyKey: "sg-d41s-commit-1",
    });

    const decoded = CommitSessionDeltaRequest.fromBinary(CommitSessionDeltaRequest.toBinary(req));

    expect(decoded.sessionReservationId).toBe("sr-voice-1");
    expect(decoded.streamingCommitId).toBe("sr-voice-1/delta/000001");
    expect(decoded.amountAtomicDelta).toBe("2500");
    expect(decoded.outcome).toBe(CommitSessionDeltaRequest_Outcome.SUCCESS);
    expect(decoded.eventTime).toEqual({ seconds: "1781233445", nanos: 678_000_000 });
    expect(decoded.idempotencyKey).toBe("sg-d41s-commit-1");
  });

  it("TP-D41S-10: builds ReleaseSessionRequest with reason and event time", () => {
    const req = buildReleaseSessionRequest({
      sessionReservationId: "sr-voice-1",
      reasonCode: "session_completed",
      eventTime: { seconds: "1781233500", nanos: 0 },
      idempotencyKey: "sg-d41s-release-1",
    });

    const decoded = ReleaseSessionRequest.fromBinary(ReleaseSessionRequest.toBinary(req));

    expect(decoded.sessionReservationId).toBe("sr-voice-1");
    expect(decoded.reasonCode).toBe("session_completed");
    expect(decoded.eventTime).toEqual({ seconds: "1781233500", nanos: 0 });
    expect(decoded.idempotencyKey).toBe("sg-d41s-release-1");
  });

  it("TP-D41S-13: rejects zero, negative, and non-decimal commit deltas", () => {
    const base = {
      sessionReservationId: "sr-voice-1",
      streamingCommitId: "sr-voice-1/delta/000002",
      outcome: "SUCCESS" as const,
      eventTime: 1_781_233_500_000,
      idempotencyKey: "sg-d41s-commit-2",
    };

    expect(() => buildCommitSessionDeltaRequest({ ...base, amountAtomicDelta: "0" })).toThrow(
      /greater than zero/,
    );
    expect(() => buildCommitSessionDeltaRequest({ ...base, amountAtomicDelta: "-1" })).toThrow(
      /positive decimal string/,
    );
    expect(() => buildCommitSessionDeltaRequest({ ...base, amountAtomicDelta: "1.5" })).toThrow(
      /positive decimal string/,
    );
  });

  it("TP-D41S-10: client reserveSession fills handshake session id and maps accepted outcome", async () => {
    const captured: { reserve?: ReturnType<typeof buildReserveSessionRequest> } = {};
    const client = clientWithAdapter({
      reserveSession(input: ReturnType<typeof buildReserveSessionRequest>) {
        captured.reserve = input;
        return {
          response: Promise.resolve({
            outcome: {
              oneofKind: "accepted",
              accepted: {
                sessionReservationId: "sr-voice-1",
                ledgerTransactionId: "lt-session-reserve-1",
                auditSessionEventId: "audit-session-reserve-1",
                ttlExpiresAt: { seconds: "1781233500", nanos: 0 },
                reservedAmountAtomic: "100000",
                remainingAmountAtomic: "100000",
              },
            },
          }),
        };
      },
    });

    const outcome = (await client.reserveSession({
      tenantId: "tenant-demo",
      budgetId: "budget-voice",
      windowInstanceId: "018ff7d0-2c9a-7f28-8d25-cf9486b08d42",
      unit: UNIT,
      pricing: PRICING,
      sessionId: "",
      route: "livekit|openai-realtime|gpt-4o-mini-transcribe",
      estimatedAmountAtomic: "100000",
      ttlSeconds: 600,
      idempotencyKey: "sg-d41s-reserve-client-1",
    })) as ReserveSessionOutcome;

    expect(captured.reserve?.sessionId).toBe("sidecar-handshake-session");
    expect(outcome.kind).toBe("accepted");
    if (outcome.kind !== "accepted") throw new Error("expected accepted");
    expect(outcome.sessionReservationId).toBe("sr-voice-1");
    expect(outcome.ledgerTransactionId).toBe("lt-session-reserve-1");
    expect(outcome.ttlExpiresAt?.toISOString()).toBe("2026-06-12T03:05:00.000Z");
  });

  it("TP-D41S-10: client commitSessionDelta and releaseSession map accepted outcomes", async () => {
    const captured: {
      commit?: ReturnType<typeof buildCommitSessionDeltaRequest>;
      release?: ReturnType<typeof buildReleaseSessionRequest>;
    } = {};
    const client = clientWithAdapter({
      commitSessionDelta(input: ReturnType<typeof buildCommitSessionDeltaRequest>) {
        captured.commit = input;
        return {
          response: Promise.resolve({
            outcome: {
              oneofKind: "accepted",
              accepted: {
                sessionReservationId: "sr-voice-1",
                streamingCommitId: "sr-voice-1/delta/000001",
                ledgerTransactionId: "lt-session-commit-1",
                auditSessionEventId: "audit-session-commit-1",
                committedDeltaAtomic: "2500",
                cumulativeCommittedAtomic: "2500",
                remainingAmountAtomic: "97500",
                recordedAt: { seconds: "1781233445", nanos: 678_000_000 },
              },
            },
          }),
        };
      },
      releaseSession(input: ReturnType<typeof buildReleaseSessionRequest>) {
        captured.release = input;
        return {
          response: Promise.resolve({
            outcome: {
              oneofKind: "accepted",
              accepted: {
                sessionReservationId: "sr-voice-1",
                ledgerTransactionId: "lt-session-release-1",
                auditSessionEventId: "audit-session-release-1",
                releasedAmountAtomic: "97500",
                committedAmountAtomic: "2500",
                recordedAt: { seconds: "1781233500", nanos: 0 },
              },
            },
          }),
        };
      },
    });

    const commit = (await client.commitSessionDelta({
      sessionReservationId: "sr-voice-1",
      streamingCommitId: "sr-voice-1/delta/000001",
      amountAtomicDelta: "2500",
      outcome: "SUCCESS",
      eventTime: new Date("2026-06-12T03:04:05.678Z"),
      idempotencyKey: "sg-d41s-commit-client-1",
    })) as CommitSessionDeltaOutcome;
    const release = (await client.releaseSession({
      sessionReservationId: "sr-voice-1",
      reasonCode: "session_completed",
      eventTime: { seconds: "1781233500", nanos: 0 },
      idempotencyKey: "sg-d41s-release-client-1",
    })) as ReleaseSessionOutcome;

    expect(captured.commit?.outcome).toBe(CommitSessionDeltaRequest_Outcome.SUCCESS);
    expect(captured.commit?.amountAtomicDelta).toBe("2500");
    expect(commit.remainingAmountAtomic).toBe("97500");
    expect(commit.recordedAt?.toISOString()).toBe("2026-06-12T03:04:05.678Z");
    expect(captured.release?.reasonCode).toBe("session_completed");
    expect(release.releasedAmountAtomic).toBe("97500");
    expect(release.recordedAt?.toISOString()).toBe("2026-06-12T03:05:00.000Z");
  });

  it("TP-D41S-13: disabled client still rejects zero commit deltas", async () => {
    const client = new SpendGuardClient({
      socketPath: "/dev/null",
      tenantId: "tenant-demo",
      disabled: true,
    });

    await expect(
      client.commitSessionDelta({
        sessionReservationId: "sr-voice-1",
        streamingCommitId: "sr-voice-1/delta/zero",
        amountAtomicDelta: "0",
        outcome: "SUCCESS",
        eventTime: new Date("2026-06-12T03:04:05.678Z"),
        idempotencyKey: "sg-d41s-zero-disabled",
      }),
    ).rejects.toThrow(/greater than zero/);
  });

  it("SR-V4: handle keeps a failed delta pending and replays the same commit id", async () => {
    const seen: SessionCommitDeltaRequest[] = [];
    let failNext = true;
    const client = {
      async commitSessionDelta(req: SessionCommitDeltaRequest): Promise<CommitSessionDeltaOutcome> {
        seen.push(req);
        if (failNext) {
          failNext = false;
          throw new Error("simulated network drop");
        }
        return acceptedCommitOutcome(req);
      },
    };
    const handle = new SessionReservationHandle({
      sessionReservationId: "sr-voice-1",
      maxPendingDeltas: 2,
    });

    await expect(
      handle.commitDelta(client, {
        amountAtomicDelta: "2500",
        outcome: "SUCCESS",
        eventTime: new Date("2026-06-12T03:04:05.678Z"),
      }),
    ).rejects.toThrow(/network drop/);

    expect(handle.pendingDeltas).toHaveLength(1);
    expect(handle.pendingDeltas[0]?.request.streamingCommitId).toBe("sr-voice-1/delta/000001");
    expect(handle.nextStreamingCommitSequence).toBe(2);

    const replayed = await handle.replayPending(client);

    expect(replayed).toHaveLength(1);
    expect(seen).toHaveLength(2);
    expect(seen[1]?.streamingCommitId).toBe(seen[0]?.streamingCommitId);
    expect(seen[1]?.idempotencyKey).toBe(seen[0]?.idempotencyKey);
    expect(handle.pendingDeltas).toHaveLength(0);
  });

  it("SR-V4: handle enforces a bounded pending-delta buffer", async () => {
    const seen: SessionCommitDeltaRequest[] = [];
    const client = {
      async commitSessionDelta(req: SessionCommitDeltaRequest): Promise<CommitSessionDeltaOutcome> {
        seen.push(req);
        throw new Error("sidecar unavailable");
      },
    };
    const handle = new SessionReservationHandle({
      sessionReservationId: "sr-voice-1",
      maxPendingDeltas: 1,
    });

    await expect(
      handle.commitDelta(client, {
        amountAtomicDelta: "1000",
        outcome: "SUCCESS",
        eventTime: new Date("2026-06-12T03:04:05.678Z"),
      }),
    ).rejects.toThrow(/sidecar unavailable/);
    expect(handle.pendingDeltas).toHaveLength(1);

    await expect(
      handle.commitDelta(client, {
        amountAtomicDelta: "2000",
        outcome: "SUCCESS",
        eventTime: new Date("2026-06-12T03:04:06.678Z"),
      }),
    ).rejects.toBeInstanceOf(SessionPendingDeltaLimitError);
    expect(seen).toHaveLength(1);
  });

  it("SR-V4: handle rejects corrupted restore snapshots and rewinded sequence state", () => {
    const handle = new SessionReservationHandle({
      sessionReservationId: "sr-voice-1",
      maxPendingDeltas: 2,
    });
    const pending = handle.enqueueDelta({
      amountAtomicDelta: "1000",
      outcome: "SUCCESS",
      eventTime: new Date("2026-06-12T03:04:05.678Z"),
    });

    expect(
      () =>
        new SessionReservationHandle({
          sessionReservationId: "sr-other",
          pendingDeltas: [pending],
        }),
    ).toThrow(SessionReservationReplayMismatchError);
    expect(
      () =>
        new SessionReservationHandle({
          sessionReservationId: "sr-voice-1",
          pendingDeltas: [pending],
          nextStreamingCommitSequence: 1,
        }),
    ).toThrow(SessionReservationReplayMismatchError);
  });

  it("SR-V4: handle stores pending requests by value for exact replay", async () => {
    const originalTime = "2026-06-12T03:04:05.678Z";
    const mutatedTime = "2026-06-12T03:09:05.678Z";
    const eventTime = new Date(originalTime);
    const seen: SessionCommitDeltaRequest[] = [];
    const sentIso: string[] = [];
    let failNext = true;
    const client = {
      async commitSessionDelta(req: SessionCommitDeltaRequest): Promise<CommitSessionDeltaOutcome> {
        seen.push(req);
        if (req.eventTime instanceof Date) {
          sentIso.push(req.eventTime.toISOString());
        }
        if (req.eventTime instanceof Date) {
          req.eventTime.setTime(new Date(mutatedTime).getTime());
        }
        if (failNext) {
          failNext = false;
          throw new Error("simulated network drop");
        }
        return acceptedCommitOutcome(req);
      },
    };
    const handle = new SessionReservationHandle({
      sessionReservationId: "sr-voice-1",
      maxPendingDeltas: 2,
    });

    await expect(
      handle.commitDelta(client, {
        amountAtomicDelta: "2500",
        outcome: "SUCCESS",
        eventTime,
      }),
    ).rejects.toThrow(/network drop/);
    eventTime.setTime(new Date(mutatedTime).getTime());

    const pendingEventTime = handle.pendingDeltas[0]?.request.eventTime;
    expect(pendingEventTime).toBeInstanceOf(Date);
    expect((pendingEventTime as Date).toISOString()).toBe(originalTime);

    await handle.replayPending(client);

    expect(seen).toHaveLength(2);
    expect(sentIso).toEqual([originalTime, originalTime]);
    const replayEventTime = seen[1]?.eventTime;
    expect(replayEventTime).toBeInstanceOf(Date);
    expect((replayEventTime as Date).toISOString()).toBe(mutatedTime);
    const storedAfterReplay = handle.snapshot().pendingDeltas;
    expect(storedAfterReplay).toHaveLength(0);
  });

  it("SR-V4: release finalizes the handle and blocks further deltas", async () => {
    const captured: { release?: SessionReleaseRequest } = {};
    const handle = new SessionReservationHandle({
      sessionReservationId: "sr-voice-1",
      maxPendingDeltas: 2,
    });
    handle.enqueueDelta({
      amountAtomicDelta: "1000",
      outcome: "SUCCESS",
      eventTime: new Date("2026-06-12T03:04:05.678Z"),
    });
    const client = {
      async releaseSession(req: SessionReleaseRequest): Promise<ReleaseSessionOutcome> {
        captured.release = req;
        return {
          sessionReservationId: req.sessionReservationId,
          ledgerTransactionId: "lt-session-release-1",
          auditSessionEventId: "audit-session-release-1",
          releasedAmountAtomic: "99000",
          committedAmountAtomic: "1000",
          recordedAt: new Date("2026-06-12T03:05:00.000Z"),
        };
      },
    };

    const release = await handle.release(client, {
      reasonCode: "session_completed",
      eventTime: new Date("2026-06-12T03:05:00.000Z"),
      idempotencyKey: "sg-d41s-release-handle-1",
    });

    expect(captured.release?.sessionReservationId).toBe("sr-voice-1");
    expect(release.releasedAmountAtomic).toBe("99000");
    expect(handle.released).toBe(true);
    expect(handle.pendingDeltas).toHaveLength(0);
    await expect(
      handle.commitDelta(
        { commitSessionDelta: async (req) => acceptedCommitOutcome(req) },
        {
          amountAtomicDelta: "1",
          outcome: "SUCCESS",
          eventTime: new Date("2026-06-12T03:05:01.000Z"),
        },
      ),
    ).rejects.toBeInstanceOf(SessionReservationReleasedError);
  });
});
