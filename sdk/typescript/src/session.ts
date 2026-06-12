// D41 session reservation substrate skeleton.
//
// This file builds protobuf envelopes for the SR-V1 contract. It intentionally
// does not perform sidecar RPCs or ledger semantics; D41S_02/D41S_03 own those
// bodies once the substrate transaction path lands.

import type { Timestamp } from "./_proto/google/protobuf/timestamp.js";
import type { UnitRef as ProtoUnitRef } from "./_proto/spendguard/common/v1/common.js";
import type {
  CommitSessionDeltaRequest as ProtoCommitSessionDeltaRequest,
  ReleaseSessionRequest as ProtoReleaseSessionRequest,
  ReserveSessionRequest as ProtoReserveSessionRequest,
} from "./_proto/spendguard/sidecar_adapter/v1/adapter.js";
import { CommitSessionDeltaRequest_Outcome } from "./_proto/spendguard/sidecar_adapter/v1/adapter.js";
import type { PricingFreeze, UnitRef } from "./client.js";

export type SessionCommitOutcome = "SUCCESS" | "PROVIDER_ERROR" | "CLIENT_TIMEOUT" | "RUN_ABORTED";

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
