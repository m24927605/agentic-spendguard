"""D41 session reservation substrate.

This module builds SR-V1 protobuf envelopes and public SDK outcome dataclasses.
Sidecar RPC bodies live on ``SpendGuardClient``.
"""

from __future__ import annotations

from collections.abc import Iterable
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Protocol

from google.protobuf.timestamp_pb2 import Timestamp

from spendguard._proto.spendguard.common.v1 import common_pb2
from spendguard._proto.spendguard.sidecar_adapter.v1 import adapter_pb2

SessionCommitOutcome = str
DEFAULT_MAX_PENDING_SESSION_DELTAS = 64


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


@dataclass(frozen=True, slots=True)
class ReserveSessionAccepted:
    session_reservation_id: str
    ledger_transaction_id: str
    audit_session_event_id: str
    ttl_expires_at: datetime | None
    reserved_amount_atomic: str
    remaining_amount_atomic: str


@dataclass(frozen=True, slots=True)
class ReserveSessionDenied:
    audit_session_event_id: str
    reason_codes: tuple[str, ...]
    matched_rule_ids: tuple[str, ...]
    error: common_pb2.Error | None = None


ReserveSessionOutcome = ReserveSessionAccepted | ReserveSessionDenied


@dataclass(frozen=True, slots=True)
class CommitSessionDeltaOutcome:
    session_reservation_id: str
    streaming_commit_id: str
    ledger_transaction_id: str
    audit_session_event_id: str
    committed_delta_atomic: str
    cumulative_committed_atomic: str
    remaining_amount_atomic: str
    recorded_at: datetime | None


@dataclass(frozen=True, slots=True)
class ReleaseSessionOutcome:
    session_reservation_id: str
    ledger_transaction_id: str
    audit_session_event_id: str
    released_amount_atomic: str
    committed_amount_atomic: str
    recorded_at: datetime | None


@dataclass(frozen=True, slots=True)
class SessionDeltaCommitInput:
    amount_atomic_delta: str
    outcome: SessionCommitOutcome
    event_time: datetime | Timestamp
    idempotency_key: str | None = None


@dataclass(frozen=True, slots=True)
class SessionReleaseInput:
    reason_code: str
    event_time: datetime | Timestamp
    idempotency_key: str


@dataclass(frozen=True, slots=True)
class PendingSessionDelta:
    sequence: int
    request: CommitSessionDeltaRequest


@dataclass(frozen=True, slots=True)
class SessionReservationHandleSnapshot:
    session_reservation_id: str
    next_streaming_commit_sequence: int
    max_pending_deltas: int
    released: bool
    pending_deltas: tuple[PendingSessionDelta, ...]


class SessionDeltaCommitClient(Protocol):
    async def commit_session_delta(
        self, req: CommitSessionDeltaRequest
    ) -> CommitSessionDeltaOutcome: ...


class SessionReleaseClient(Protocol):
    async def release_session(
        self, req: ReleaseSessionRequest
    ) -> ReleaseSessionOutcome: ...


class SessionReservationHandleError(Exception):
    """Base error for local session replay handle invariants."""


class SessionPendingDeltaLimitError(SessionReservationHandleError):
    def __init__(self, max_pending_deltas: int) -> None:
        super().__init__(
            f"session pending delta buffer is full: max_pending_deltas={max_pending_deltas}"
        )


class SessionReservationReleasedError(SessionReservationHandleError):
    def __init__(self, session_reservation_id: str) -> None:
        super().__init__(f"session reservation already released: {session_reservation_id}")


class SessionReservationReplayMismatchError(SessionReservationHandleError):
    """Raised when an ack/release response does not match local pending state."""


class SessionReservationHandle:
    def __init__(
        self,
        *,
        session_reservation_id: str,
        next_streaming_commit_sequence: int | None = None,
        max_pending_deltas: int = DEFAULT_MAX_PENDING_SESSION_DELTAS,
        pending_deltas: Iterable[PendingSessionDelta] = (),
        released: bool = False,
    ) -> None:
        _assert_non_empty(session_reservation_id, "session_reservation_id")
        _assert_positive_int(max_pending_deltas, "max_pending_deltas")
        self.session_reservation_id = session_reservation_id
        self.max_pending_deltas = max_pending_deltas
        self._pending_deltas = _clone_and_validate_pending_deltas(
            tuple(pending_deltas), session_reservation_id
        )
        if len(self._pending_deltas) > self.max_pending_deltas:
            raise SessionPendingDeltaLimitError(self.max_pending_deltas)
        inferred_next_sequence = _infer_next_sequence(self._pending_deltas)
        self._next_streaming_commit_sequence = (
            next_streaming_commit_sequence
            if next_streaming_commit_sequence is not None
            else inferred_next_sequence
        )
        _assert_positive_int(
            self._next_streaming_commit_sequence,
            "next_streaming_commit_sequence",
        )
        if self._next_streaming_commit_sequence < inferred_next_sequence:
            raise SessionReservationReplayMismatchError(
                "next_streaming_commit_sequence must be "
                f">= {inferred_next_sequence} for pending deltas"
            )
        self._released = released

    @classmethod
    def from_snapshot(
        cls, snapshot: SessionReservationHandleSnapshot
    ) -> SessionReservationHandle:
        return cls(
            session_reservation_id=snapshot.session_reservation_id,
            next_streaming_commit_sequence=snapshot.next_streaming_commit_sequence,
            max_pending_deltas=snapshot.max_pending_deltas,
            pending_deltas=snapshot.pending_deltas,
            released=snapshot.released,
        )

    @property
    def next_streaming_commit_sequence(self) -> int:
        return self._next_streaming_commit_sequence

    @property
    def released(self) -> bool:
        return self._released

    @property
    def pending_deltas(self) -> tuple[PendingSessionDelta, ...]:
        return tuple(_clone_pending_delta(pending) for pending in self._pending_deltas)

    def snapshot(self) -> SessionReservationHandleSnapshot:
        return SessionReservationHandleSnapshot(
            session_reservation_id=self.session_reservation_id,
            next_streaming_commit_sequence=self._next_streaming_commit_sequence,
            max_pending_deltas=self.max_pending_deltas,
            released=self._released,
            pending_deltas=self.pending_deltas,
        )

    def enqueue_delta(self, input: SessionDeltaCommitInput) -> PendingSessionDelta:
        self._assert_open()
        if len(self._pending_deltas) >= self.max_pending_deltas:
            raise SessionPendingDeltaLimitError(self.max_pending_deltas)
        sequence = self._next_streaming_commit_sequence
        streaming_commit_id = _format_streaming_commit_id(
            self.session_reservation_id, sequence
        )
        req = CommitSessionDeltaRequest(
            session_reservation_id=self.session_reservation_id,
            streaming_commit_id=streaming_commit_id,
            amount_atomic_delta=input.amount_atomic_delta,
            outcome=input.outcome,
            event_time=_clone_event_time(input.event_time),
            idempotency_key=input.idempotency_key or streaming_commit_id,
        )
        build_commit_session_delta_request(req)
        self._next_streaming_commit_sequence += 1
        pending = PendingSessionDelta(
            sequence=sequence, request=_clone_commit_session_delta_request(req)
        )
        self._pending_deltas = (*self._pending_deltas, pending)
        return _clone_pending_delta(pending)

    async def commit_delta(
        self,
        client: SessionDeltaCommitClient,
        input: SessionDeltaCommitInput,
    ) -> CommitSessionDeltaOutcome:
        pending = self.enqueue_delta(input)
        outcome = await client.commit_session_delta(
            _clone_commit_session_delta_request(pending.request)
        )
        self._ack_outcome(outcome)
        return outcome

    async def replay_pending(
        self, client: SessionDeltaCommitClient
    ) -> tuple[CommitSessionDeltaOutcome, ...]:
        outcomes: list[CommitSessionDeltaOutcome] = []
        for pending in tuple(self._pending_deltas):
            outcome = await client.commit_session_delta(
                _clone_commit_session_delta_request(pending.request)
            )
            self._ack_outcome(outcome)
            outcomes.append(outcome)
        return tuple(outcomes)

    async def release(
        self,
        client: SessionReleaseClient,
        input: SessionReleaseInput,
    ) -> ReleaseSessionOutcome:
        self._assert_open()
        req = ReleaseSessionRequest(
            session_reservation_id=self.session_reservation_id,
            reason_code=input.reason_code,
            event_time=input.event_time,
            idempotency_key=input.idempotency_key,
        )
        build_release_session_request(req)
        outcome = await client.release_session(req)
        if outcome.session_reservation_id != self.session_reservation_id:
            raise SessionReservationReplayMismatchError(
                "release outcome session_reservation_id mismatch: "
                f"expected {self.session_reservation_id} "
                f"got {outcome.session_reservation_id}"
            )
        self._pending_deltas = ()
        self._released = True
        return outcome

    def _ack_outcome(self, outcome: CommitSessionDeltaOutcome) -> None:
        if outcome.session_reservation_id != self.session_reservation_id:
            raise SessionReservationReplayMismatchError(
                "commit outcome session_reservation_id mismatch: "
                f"expected {self.session_reservation_id} "
                f"got {outcome.session_reservation_id}"
            )
        index = next(
            (
                idx
                for idx, pending in enumerate(self._pending_deltas)
                if pending.request.streaming_commit_id == outcome.streaming_commit_id
            ),
            -1,
        )
        if index < 0:
            raise SessionReservationReplayMismatchError(
                "commit outcome streaming_commit_id is not pending: "
                f"{outcome.streaming_commit_id}"
            )
        self._pending_deltas = (
            self._pending_deltas[:index] + self._pending_deltas[index + 1 :]
        )

    def _assert_open(self) -> None:
        if self._released:
            raise SessionReservationReleasedError(self.session_reservation_id)


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


def timestamp_to_datetime(value: Timestamp | None) -> datetime | None:
    if value is None or (value.seconds == 0 and value.nanos == 0):
        return None
    return value.ToDatetime(tzinfo=timezone.utc)


def _assert_positive_decimal(value: str, field: str) -> None:
    if not value.isdecimal():
        raise ValueError(f"{field} must be a positive decimal string")
    if int(value) <= 0:
        raise ValueError(f"{field} must be greater than zero")


def _assert_positive_int(value: int, field: str) -> None:
    if value <= 0:
        raise ValueError(f"{field} must be a positive integer")


def _assert_non_empty(value: str, field: str) -> None:
    if not value:
        raise ValueError(f"{field} must be non-empty")


def _format_streaming_commit_id(session_reservation_id: str, sequence: int) -> str:
    return f"{session_reservation_id}/delta/{sequence:06d}"


def _infer_next_sequence(pending_deltas: tuple[PendingSessionDelta, ...]) -> int:
    if not pending_deltas:
        return 1
    return max(pending.sequence for pending in pending_deltas) + 1


def _clone_and_validate_pending_deltas(
    pending_deltas: tuple[PendingSessionDelta, ...],
    session_reservation_id: str,
) -> tuple[PendingSessionDelta, ...]:
    seen_sequences: set[int] = set()
    cloned: list[PendingSessionDelta] = []
    for pending in pending_deltas:
        _assert_positive_int(pending.sequence, "pending_delta.sequence")
        if pending.sequence in seen_sequences:
            raise SessionReservationReplayMismatchError(
                f"duplicate pending delta sequence: {pending.sequence}"
            )
        seen_sequences.add(pending.sequence)

        request = _clone_commit_session_delta_request(pending.request)
        if request.session_reservation_id != session_reservation_id:
            raise SessionReservationReplayMismatchError(
                "pending delta session_reservation_id mismatch: "
                f"expected {session_reservation_id} "
                f"got {request.session_reservation_id}"
            )
        expected_streaming_commit_id = _format_streaming_commit_id(
            session_reservation_id, pending.sequence
        )
        if request.streaming_commit_id != expected_streaming_commit_id:
            raise SessionReservationReplayMismatchError(
                "pending delta streaming_commit_id mismatch: "
                f"expected {expected_streaming_commit_id} "
                f"got {request.streaming_commit_id}"
            )
        build_commit_session_delta_request(request)
        cloned.append(PendingSessionDelta(sequence=pending.sequence, request=request))
    return tuple(sorted(cloned, key=lambda pending: pending.sequence))


def _clone_pending_delta(pending: PendingSessionDelta) -> PendingSessionDelta:
    return PendingSessionDelta(
        sequence=pending.sequence,
        request=_clone_commit_session_delta_request(pending.request),
    )


def _clone_commit_session_delta_request(
    req: CommitSessionDeltaRequest,
) -> CommitSessionDeltaRequest:
    return CommitSessionDeltaRequest(
        session_reservation_id=req.session_reservation_id,
        streaming_commit_id=req.streaming_commit_id,
        amount_atomic_delta=req.amount_atomic_delta,
        outcome=req.outcome,
        event_time=_clone_event_time(req.event_time),
        idempotency_key=req.idempotency_key,
    )


def _clone_event_time(value: datetime | Timestamp) -> datetime | Timestamp:
    if isinstance(value, Timestamp):
        cloned = Timestamp()
        cloned.CopyFrom(value)
        return cloned
    return value
