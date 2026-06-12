"""D41 session reservation substrate skeleton.

This module builds SR-V1 protobuf envelopes only. It intentionally does not
perform sidecar RPCs or ledger semantics; those land in later D41 substrate
slices.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone

from google.protobuf.timestamp_pb2 import Timestamp

from spendguard._proto.spendguard.common.v1 import common_pb2
from spendguard._proto.spendguard.sidecar_adapter.v1 import adapter_pb2

SessionCommitOutcome = str


@dataclass(frozen=True, slots=True)
class ReserveSessionRequest:
    tenant_id: str
    budget_id: str
    window_instance_id: str
    unit: common_pb2.UnitRef
    pricing: common_pb2.PricingFreeze
    session_id: str
    route: str
    estimated_amount_atomic: str
    ttl_seconds: int
    idempotency_key: str


@dataclass(frozen=True, slots=True)
class CommitSessionDeltaRequest:
    session_reservation_id: str
    streaming_commit_id: str
    amount_atomic_delta: str
    outcome: SessionCommitOutcome
    event_time: datetime | Timestamp
    idempotency_key: str


@dataclass(frozen=True, slots=True)
class ReleaseSessionRequest:
    session_reservation_id: str
    reason_code: str
    event_time: datetime | Timestamp
    idempotency_key: str


def build_reserve_session_request(
    req: ReserveSessionRequest,
) -> adapter_pb2.ReserveSessionRequest:
    _assert_positive_decimal(req.estimated_amount_atomic, "estimated_amount_atomic")
    _assert_positive_int(req.ttl_seconds, "ttl_seconds")
    msg = adapter_pb2.ReserveSessionRequest(
        tenant_id=req.tenant_id,
        budget_id=req.budget_id,
        window_instance_id=req.window_instance_id,
        session_id=req.session_id,
        route=req.route,
        estimated_amount_atomic=req.estimated_amount_atomic,
        ttl_seconds=req.ttl_seconds,
        idempotency_key=req.idempotency_key,
    )
    msg.unit.CopyFrom(req.unit)
    msg.pricing.CopyFrom(req.pricing)
    return msg


def build_commit_session_delta_request(
    req: CommitSessionDeltaRequest,
) -> adapter_pb2.CommitSessionDeltaRequest:
    _assert_positive_decimal(req.amount_atomic_delta, "amount_atomic_delta")
    msg = adapter_pb2.CommitSessionDeltaRequest(
        session_reservation_id=req.session_reservation_id,
        streaming_commit_id=req.streaming_commit_id,
        amount_atomic_delta=req.amount_atomic_delta,
        outcome=_commit_outcome_enum(req.outcome),
        idempotency_key=req.idempotency_key,
    )
    msg.event_time.CopyFrom(_to_timestamp(req.event_time))
    return msg


def build_release_session_request(
    req: ReleaseSessionRequest,
) -> adapter_pb2.ReleaseSessionRequest:
    msg = adapter_pb2.ReleaseSessionRequest(
        session_reservation_id=req.session_reservation_id,
        reason_code=req.reason_code,
        idempotency_key=req.idempotency_key,
    )
    msg.event_time.CopyFrom(_to_timestamp(req.event_time))
    return msg


def _commit_outcome_enum(outcome: SessionCommitOutcome) -> int:
    try:
        return {
            "SUCCESS": adapter_pb2.CommitSessionDeltaRequest.SUCCESS,
            "PROVIDER_ERROR": adapter_pb2.CommitSessionDeltaRequest.PROVIDER_ERROR,
            "CLIENT_TIMEOUT": adapter_pb2.CommitSessionDeltaRequest.CLIENT_TIMEOUT,
            "RUN_ABORTED": adapter_pb2.CommitSessionDeltaRequest.RUN_ABORTED,
        }[outcome]
    except KeyError as exc:
        raise ValueError(f"unknown session commit outcome: {outcome}") from exc


def _to_timestamp(value: datetime | Timestamp) -> Timestamp:
    if isinstance(value, Timestamp):
        return value
    dt = value if value.tzinfo is not None else value.replace(tzinfo=timezone.utc)
    ts = Timestamp()
    ts.FromDatetime(dt.astimezone(timezone.utc))
    return ts


def _assert_positive_decimal(value: str, field: str) -> None:
    if not value.isdecimal():
        raise ValueError(f"{field} must be a positive decimal string")
    if int(value) <= 0:
        raise ValueError(f"{field} must be greater than zero")


def _assert_positive_int(value: int, field: str) -> None:
    if value <= 0:
        raise ValueError(f"{field} must be a positive integer")
