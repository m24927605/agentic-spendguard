// D41 session reservation substrate.
//
// This file builds protobuf envelopes and public SDK outcome types for the
// SR-V1/SR-V3 contract. Sidecar RPC bodies live on SpendGuardClient.

import type { Timestamp } from "./_proto/google/protobuf/timestamp.js";
import type { Error as ProtoError } from "./_proto/spendguard/common/v1/common.js";
import type { UnitRef as ProtoUnitRef } from "./_proto/spendguard/common/v1/common.js";
import type {
  CommitSessionDeltaRequest as ProtoCommitSessionDeltaRequest,
  ReleaseSessionRequest as ProtoReleaseSessionRequest,
  ReserveSessionRequest as ProtoReserveSessionRequest,
} from "./_proto/spendguard/sidecar_adapter/v1/adapter.js";
import { CommitSessionDeltaRequest_Outcome } from "./_proto/spendguard/sidecar_adapter/v1/adapter.js";
import type { PricingFreeze, UnitRef } from "./client.js";

export type SessionCommitOutcome = "SUCCESS" | "PROVIDER_ERROR" | "CLIENT_TIMEOUT" | "RUN_ABORTED";

export const DEFAULT_MAX_PENDING_SESSION_DELTAS = 64;

export interface ReserveSessionRequest {
  tenantId: string;
  budgetId: string;
  windowInstanceId: string;
  unit: UnitRef;
  pricing: PricingFreeze;
  sessionId: string;
  route: string;
  estimatedAmountAtomic: string;
  ttlSeconds: number;
  idempotencyKey: string;
}

export interface CommitSessionDeltaRequest {
  sessionReservationId: string;
  streamingCommitId: string;
  amountAtomicDelta: string;
  outcome: SessionCommitOutcome;
  eventTime: Date | number | Timestamp;
  idempotencyKey: string;
}

export interface ReleaseSessionRequest {
  sessionReservationId: string;
  reasonCode: string;
  eventTime: Date | number | Timestamp;
  idempotencyKey: string;
}

export type ReserveSessionOutcome =
  | {
      kind: "accepted";
      sessionReservationId: string;
      ledgerTransactionId: string;
      auditSessionEventId: string;
      ttlExpiresAt: Date | null;
      reservedAmountAtomic: string;
      remainingAmountAtomic: string;
    }
  | {
      kind: "denied";
      auditSessionEventId: string;
      reasonCodes: readonly string[];
      matchedRuleIds: readonly string[];
      error?: ProtoError;
    };

export interface CommitSessionDeltaOutcome {
  sessionReservationId: string;
  streamingCommitId: string;
  ledgerTransactionId: string;
  auditSessionEventId: string;
  committedDeltaAtomic: string;
  cumulativeCommittedAtomic: string;
  remainingAmountAtomic: string;
  recordedAt: Date | null;
}

export interface ReleaseSessionOutcome {
  sessionReservationId: string;
  ledgerTransactionId: string;
  auditSessionEventId: string;
  releasedAmountAtomic: string;
  committedAmountAtomic: string;
  recordedAt: Date | null;
}

export interface SessionDeltaCommitInput {
  amountAtomicDelta: string;
  outcome: SessionCommitOutcome;
  eventTime: Date | number | Timestamp;
  idempotencyKey?: string;
}

export interface SessionReleaseInput {
  reasonCode: string;
  eventTime: Date | number | Timestamp;
  idempotencyKey: string;
}

export interface PendingSessionDelta {
  sequence: number;
  request: CommitSessionDeltaRequest;
}

export interface SessionReservationHandleOptions {
  sessionReservationId: string;
  nextStreamingCommitSequence?: number;
  maxPendingDeltas?: number;
  pendingDeltas?: readonly PendingSessionDelta[];
  released?: boolean;
}

export interface SessionReservationHandleSnapshot {
  sessionReservationId: string;
  nextStreamingCommitSequence: number;
  maxPendingDeltas: number;
  released: boolean;
  pendingDeltas: readonly PendingSessionDelta[];
}

export interface SessionDeltaCommitClient {
  commitSessionDelta(req: CommitSessionDeltaRequest): Promise<CommitSessionDeltaOutcome>;
}

export interface SessionReleaseClient {
  releaseSession(req: ReleaseSessionRequest): Promise<ReleaseSessionOutcome>;
}

export class SessionReservationHandleError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "SessionReservationHandleError";
  }
}

export class SessionPendingDeltaLimitError extends SessionReservationHandleError {
  constructor(maxPendingDeltas: number) {
    super(`session pending delta buffer is full: maxPendingDeltas=${maxPendingDeltas}`);
    this.name = "SessionPendingDeltaLimitError";
  }
}

export class SessionReservationReleasedError extends SessionReservationHandleError {
  constructor(sessionReservationId: string) {
    super(`session reservation already released: ${sessionReservationId}`);
    this.name = "SessionReservationReleasedError";
  }
}

export class SessionReservationReplayMismatchError extends SessionReservationHandleError {
  constructor(message: string) {
    super(message);
    this.name = "SessionReservationReplayMismatchError";
  }
}

export class SessionReservationHandle {
  readonly sessionReservationId: string;
  readonly maxPendingDeltas: number;

  private nextStreamingCommitSequenceValue: number;
  private pendingDeltaBuffer: PendingSessionDelta[];
  private releasedValue: boolean;

  constructor(options: SessionReservationHandleOptions) {
    assertNonEmpty(options.sessionReservationId, "sessionReservationId");
    this.sessionReservationId = options.sessionReservationId;
    this.maxPendingDeltas = options.maxPendingDeltas ?? DEFAULT_MAX_PENDING_SESSION_DELTAS;
    assertPositiveInteger(this.maxPendingDeltas, "maxPendingDeltas");

    this.pendingDeltaBuffer = cloneAndValidatePendingDeltas(
      options.pendingDeltas ?? [],
      this.sessionReservationId,
    );
    if (this.pendingDeltaBuffer.length > this.maxPendingDeltas) {
      throw new SessionPendingDeltaLimitError(this.maxPendingDeltas);
    }
    const inferredNextSequence = inferNextSequence(this.pendingDeltaBuffer);
    this.nextStreamingCommitSequenceValue =
      options.nextStreamingCommitSequence ?? inferredNextSequence;
    assertPositiveInteger(this.nextStreamingCommitSequenceValue, "nextStreamingCommitSequence");
    if (this.nextStreamingCommitSequenceValue < inferredNextSequence) {
      throw new SessionReservationReplayMismatchError(
        `nextStreamingCommitSequence must be >= ${inferredNextSequence} for pending deltas`,
      );
    }
    this.releasedValue = options.released ?? false;
  }

  static fromSnapshot(snapshot: SessionReservationHandleSnapshot): SessionReservationHandle {
    return new SessionReservationHandle(snapshot);
  }

  get nextStreamingCommitSequence(): number {
    return this.nextStreamingCommitSequenceValue;
  }

  get released(): boolean {
    return this.releasedValue;
  }

  get pendingDeltas(): readonly PendingSessionDelta[] {
    return this.pendingDeltaBuffer.map(clonePendingDelta);
  }

  snapshot(): SessionReservationHandleSnapshot {
    return {
      sessionReservationId: this.sessionReservationId,
      nextStreamingCommitSequence: this.nextStreamingCommitSequenceValue,
      maxPendingDeltas: this.maxPendingDeltas,
      released: this.releasedValue,
      pendingDeltas: this.pendingDeltas,
    };
  }

  enqueueDelta(input: SessionDeltaCommitInput): PendingSessionDelta {
    this.assertOpen();
    if (this.pendingDeltaBuffer.length >= this.maxPendingDeltas) {
      throw new SessionPendingDeltaLimitError(this.maxPendingDeltas);
    }
    const sequence = this.nextStreamingCommitSequenceValue;
    const streamingCommitId = formatStreamingCommitId(this.sessionReservationId, sequence);
    const request: CommitSessionDeltaRequest = {
      sessionReservationId: this.sessionReservationId,
      streamingCommitId,
      amountAtomicDelta: input.amountAtomicDelta,
      outcome: input.outcome,
      eventTime: cloneEventTime(input.eventTime),
      idempotencyKey: input.idempotencyKey ?? streamingCommitId,
    };
    buildCommitSessionDeltaRequest(request);
    this.nextStreamingCommitSequenceValue += 1;

    const pending = { sequence, request };
    this.pendingDeltaBuffer.push(pending);
    return clonePendingDelta(pending);
  }

  async commitDelta(
    client: SessionDeltaCommitClient,
    input: SessionDeltaCommitInput,
  ): Promise<CommitSessionDeltaOutcome> {
    const pending = this.enqueueDelta(input);
    const outcome = await client.commitSessionDelta(
      cloneCommitSessionDeltaRequest(pending.request),
    );
    this.ackOutcome(outcome);
    return outcome;
  }

  async replayPending(client: SessionDeltaCommitClient): Promise<CommitSessionDeltaOutcome[]> {
    const outcomes: CommitSessionDeltaOutcome[] = [];
    for (const pending of [...this.pendingDeltaBuffer]) {
      const outcome = await client.commitSessionDelta(
        cloneCommitSessionDeltaRequest(pending.request),
      );
      this.ackOutcome(outcome);
      outcomes.push(outcome);
    }
    return outcomes;
  }

  async release(
    client: SessionReleaseClient,
    input: SessionReleaseInput,
  ): Promise<ReleaseSessionOutcome> {
    this.assertOpen();
    const request: ReleaseSessionRequest = {
      sessionReservationId: this.sessionReservationId,
      reasonCode: input.reasonCode,
      eventTime: input.eventTime,
      idempotencyKey: input.idempotencyKey,
    };
    buildReleaseSessionRequest(request);
    const outcome = await client.releaseSession(request);
    if (outcome.sessionReservationId !== this.sessionReservationId) {
      throw new SessionReservationReplayMismatchError(
        `release outcome session_reservation_id mismatch: expected ${this.sessionReservationId} got ${outcome.sessionReservationId}`,
      );
    }
    this.pendingDeltaBuffer = [];
    this.releasedValue = true;
    return outcome;
  }

  private ackOutcome(outcome: CommitSessionDeltaOutcome): void {
    if (outcome.sessionReservationId !== this.sessionReservationId) {
      throw new SessionReservationReplayMismatchError(
        `commit outcome session_reservation_id mismatch: expected ${this.sessionReservationId} got ${outcome.sessionReservationId}`,
      );
    }
    const index = this.pendingDeltaBuffer.findIndex(
      (pending) => pending.request.streamingCommitId === outcome.streamingCommitId,
    );
    if (index < 0) {
      throw new SessionReservationReplayMismatchError(
        `commit outcome streaming_commit_id is not pending: ${outcome.streamingCommitId}`,
      );
    }
    this.pendingDeltaBuffer.splice(index, 1);
  }

  private assertOpen(): void {
    if (this.releasedValue) {
      throw new SessionReservationReleasedError(this.sessionReservationId);
    }
  }
}

export function buildReserveSessionRequest(req: ReserveSessionRequest): ProtoReserveSessionRequest {
  assertPositiveDecimal(req.estimatedAmountAtomic, "estimatedAmountAtomic");
  assertPositiveInteger(req.ttlSeconds, "ttlSeconds");
  return {
    tenantId: req.tenantId,
    budgetId: req.budgetId,
    windowInstanceId: req.windowInstanceId,
    unit: mapUnitRef(req.unit),
    pricing: {
      pricingVersion: req.pricing.pricingVersion,
      priceSnapshotHash: req.pricing.pricingHash,
      fxRateVersion: req.pricing.fxRateVersion ?? "",
      unitConversionVersion: req.pricing.unitConversionVersion ?? "",
    },
    sessionId: req.sessionId,
    route: req.route,
    estimatedAmountAtomic: req.estimatedAmountAtomic,
    ttlSeconds: req.ttlSeconds,
    idempotencyKey: req.idempotencyKey,
  };
}

export function buildCommitSessionDeltaRequest(
  req: CommitSessionDeltaRequest,
): ProtoCommitSessionDeltaRequest {
  assertPositiveDecimal(req.amountAtomicDelta, "amountAtomicDelta");
  return {
    sessionReservationId: req.sessionReservationId,
    streamingCommitId: req.streamingCommitId,
    amountAtomicDelta: req.amountAtomicDelta,
    outcome: commitOutcomeEnumOf(req.outcome),
    eventTime: toTimestamp(req.eventTime),
    idempotencyKey: req.idempotencyKey,
  };
}

export function buildReleaseSessionRequest(req: ReleaseSessionRequest): ProtoReleaseSessionRequest {
  return {
    sessionReservationId: req.sessionReservationId,
    reasonCode: req.reasonCode,
    eventTime: toTimestamp(req.eventTime),
    idempotencyKey: req.idempotencyKey,
  };
}

function mapUnitRef(unit: UnitRef): ProtoUnitRef {
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

function commitOutcomeEnumOf(outcome: SessionCommitOutcome): CommitSessionDeltaRequest_Outcome {
  switch (outcome) {
    case "SUCCESS":
      return CommitSessionDeltaRequest_Outcome.SUCCESS;
    case "PROVIDER_ERROR":
      return CommitSessionDeltaRequest_Outcome.PROVIDER_ERROR;
    case "CLIENT_TIMEOUT":
      return CommitSessionDeltaRequest_Outcome.CLIENT_TIMEOUT;
    case "RUN_ABORTED":
      return CommitSessionDeltaRequest_Outcome.RUN_ABORTED;
  }
}

function assertPositiveDecimal(value: string, field: string): void {
  if (!/^[0-9]+$/.test(value)) {
    throw new RangeError(`${field} must be a positive decimal string`);
  }
  if (BigInt(value) <= 0n) {
    throw new RangeError(`${field} must be greater than zero`);
  }
}

function assertPositiveInteger(value: number, field: string): void {
  if (!Number.isInteger(value) || value <= 0) {
    throw new RangeError(`${field} must be a positive integer`);
  }
}

function assertNonEmpty(value: string, field: string): void {
  if (value.length === 0) {
    throw new RangeError(`${field} must be non-empty`);
  }
}

function formatStreamingCommitId(sessionReservationId: string, sequence: number): string {
  return `${sessionReservationId}/delta/${String(sequence).padStart(6, "0")}`;
}

function inferNextSequence(pendingDeltas: readonly PendingSessionDelta[]): number {
  if (pendingDeltas.length === 0) return 1;
  return Math.max(...pendingDeltas.map((pending) => pending.sequence)) + 1;
}

function clonePendingDelta(pending: PendingSessionDelta): PendingSessionDelta {
  return {
    sequence: pending.sequence,
    request: cloneCommitSessionDeltaRequest(pending.request),
  };
}

function cloneAndValidatePendingDeltas(
  pendingDeltas: readonly PendingSessionDelta[],
  sessionReservationId: string,
): PendingSessionDelta[] {
  const seenSequences = new Set<number>();
  const cloned = pendingDeltas.map((pending) => {
    assertPositiveInteger(pending.sequence, "pendingDelta.sequence");
    if (seenSequences.has(pending.sequence)) {
      throw new SessionReservationReplayMismatchError(
        `duplicate pending delta sequence: ${pending.sequence}`,
      );
    }
    seenSequences.add(pending.sequence);

    const request = cloneCommitSessionDeltaRequest(pending.request);
    if (request.sessionReservationId !== sessionReservationId) {
      throw new SessionReservationReplayMismatchError(
        `pending delta session_reservation_id mismatch: expected ${sessionReservationId} got ${request.sessionReservationId}`,
      );
    }
    const expectedStreamingCommitId = formatStreamingCommitId(
      sessionReservationId,
      pending.sequence,
    );
    if (request.streamingCommitId !== expectedStreamingCommitId) {
      throw new SessionReservationReplayMismatchError(
        `pending delta streaming_commit_id mismatch: expected ${expectedStreamingCommitId} got ${request.streamingCommitId}`,
      );
    }
    buildCommitSessionDeltaRequest(request);
    return { sequence: pending.sequence, request };
  });
  return cloned.sort((left, right) => left.sequence - right.sequence);
}

function cloneCommitSessionDeltaRequest(req: CommitSessionDeltaRequest): CommitSessionDeltaRequest {
  return {
    sessionReservationId: req.sessionReservationId,
    streamingCommitId: req.streamingCommitId,
    amountAtomicDelta: req.amountAtomicDelta,
    outcome: req.outcome,
    eventTime: cloneEventTime(req.eventTime),
    idempotencyKey: req.idempotencyKey,
  };
}

function cloneEventTime(value: Date | number | Timestamp): Date | number | Timestamp {
  if (value instanceof Date) return new Date(value.getTime());
  if (typeof value === "number") return value;
  return { seconds: value.seconds, nanos: value.nanos };
}

function toTimestamp(value: Date | number | Timestamp): Timestamp {
  if (value instanceof Date) return epochMsToTimestamp(value.getTime());
  if (typeof value === "number") return epochMsToTimestamp(value);
  return value;
}

function epochMsToTimestamp(epochMs: number): Timestamp {
  if (!Number.isFinite(epochMs)) {
    throw new RangeError("eventTime must be finite");
  }
  const seconds = Math.floor(epochMs / 1000);
  const nanos = (epochMs - seconds * 1000) * 1_000_000;
  return { seconds: seconds.toString(), nanos };
}

export function timestampToDate(value: Timestamp | undefined): Date | null {
  if (value === undefined) return null;
  const seconds =
    typeof value.seconds === "bigint" ? Number(value.seconds) : Number.parseInt(value.seconds, 10);
  if (!Number.isFinite(seconds)) return null;
  return new Date(seconds * 1000 + Math.floor(value.nanos / 1_000_000));
}
