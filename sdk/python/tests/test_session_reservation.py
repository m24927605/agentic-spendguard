"""D41S_01 session reservation contract/proto skeleton tests."""

from __future__ import annotations

from datetime import datetime, timezone

import pytest
from google.protobuf.timestamp_pb2 import Timestamp

from spendguard import SpendGuardClient
from spendguard._proto.spendguard.common.v1 import common_pb2
from spendguard._proto.spendguard.sidecar_adapter.v1 import (
    adapter_pb2,
    adapter_pb2_grpc,
)
from spendguard.client import HandshakeOutcome
from spendguard.session import (
    CommitSessionDeltaOutcome,
    CommitSessionDeltaRequest,
    ReleaseSessionOutcome,
    ReleaseSessionRequest,
    ReserveSessionAccepted,
    ReserveSessionRequest,
    SessionDeltaCommitInput,
    SessionPendingDeltaLimitError,
    SessionReleaseInput,
    SessionReservationHandle,
    SessionReservationReleasedError,
    SessionReservationReplayMismatchError,
    build_commit_session_delta_request,
    build_release_session_request,
    build_reserve_session_request,
)

UNIT = common_pb2.UnitRef(
    unit_id="018ff7d0-2c9a-7f28-8d25-cf9486b08d41",
    unit_name="USD_MICROS",
)
PRICING = common_pb2.PricingFreeze(
    pricing_version="focus-v1.2-demo",
    price_snapshot_hash=bytes([0xA1, 0xB2, 0xC3]),
    fx_rate_version="fx-2026-06-12",
    unit_conversion_version="unitconv-2026-06-12",
)
HANDSHAKE = HandshakeOutcome(
    session_id="sidecar-handshake-session",
    sidecar_version="test-sidecar",
    schema_bundle_id="schema",
    schema_bundle_hash=b"",
    contract_bundle_id="contract",
    contract_bundle_hash=b"",
    capability_required=0,
    signing_key_id="test-key",
    announcement_signature=b"",
)


def _accepted_commit_outcome(
    req: CommitSessionDeltaRequest,
    cumulative_committed_atomic: str | None = None,
) -> CommitSessionDeltaOutcome:
    return CommitSessionDeltaOutcome(
        session_reservation_id=req.session_reservation_id,
        streaming_commit_id=req.streaming_commit_id,
        ledger_transaction_id=f"lt-{req.streaming_commit_id}",
        audit_session_event_id=f"audit-{req.streaming_commit_id}",
        committed_delta_atomic=req.amount_atomic_delta,
        cumulative_committed_atomic=(
            cumulative_committed_atomic or req.amount_atomic_delta
        ),
        remaining_amount_atomic="97500",
        recorded_at=datetime(2026, 6, 12, 3, 4, 5, 678000, tzinfo=timezone.utc),
    )


def _ts(seconds: int, nanos: int = 0) -> Timestamp:
    ts = Timestamp()
    ts.seconds = seconds
    ts.nanos = nanos
    return ts


def test_sr_v1_exposes_session_rpcs_on_sidecar_adapter() -> None:
    service = adapter_pb2.DESCRIPTOR.services_by_name["SidecarAdapter"]
    method_names = {m.name for m in service.methods}

    assert {"ReserveSession", "CommitSessionDelta", "ReleaseSession"} <= method_names
    assert hasattr(adapter_pb2_grpc.SidecarAdapterServicer, "ReserveSession")
    assert hasattr(adapter_pb2_grpc.SidecarAdapterServicer, "CommitSessionDelta")
    assert hasattr(adapter_pb2_grpc.SidecarAdapterServicer, "ReleaseSession")


def test_tp_d41s_11_builds_reserve_session_request() -> None:
    req = build_reserve_session_request(
        ReserveSessionRequest(
            tenant_id="tenant-demo",
            budget_id="budget-voice",
            window_instance_id="018ff7d0-2c9a-7f28-8d25-cf9486b08d42",
            unit=UNIT,
            pricing=PRICING,
            session_id="sidecar-handshake-session",
            route="pipecat|openai-realtime|gpt-4o-mini-transcribe",
            estimated_amount_atomic="100000",
            ttl_seconds=600,
            idempotency_key="sg-d41s-reserve-1",
        )
    )

    decoded = adapter_pb2.ReserveSessionRequest.FromString(req.SerializeToString())

    assert decoded.tenant_id == "tenant-demo"
    assert decoded.budget_id == "budget-voice"
    assert decoded.window_instance_id == "018ff7d0-2c9a-7f28-8d25-cf9486b08d42"
    assert decoded.unit.unit_id == UNIT.unit_id
    assert decoded.unit.unit_name == "USD_MICROS"
    assert decoded.pricing.pricing_version == "focus-v1.2-demo"
    assert decoded.pricing.price_snapshot_hash == bytes([0xA1, 0xB2, 0xC3])
    assert decoded.pricing.fx_rate_version == "fx-2026-06-12"
    assert decoded.pricing.unit_conversion_version == "unitconv-2026-06-12"
    assert decoded.session_id == "sidecar-handshake-session"
    assert decoded.route == "pipecat|openai-realtime|gpt-4o-mini-transcribe"
    assert decoded.estimated_amount_atomic == "100000"
    assert decoded.ttl_seconds == 600
    assert decoded.idempotency_key == "sg-d41s-reserve-1"


def test_tp_d41s_11_builds_commit_session_delta_request() -> None:
    req = build_commit_session_delta_request(
        CommitSessionDeltaRequest(
            session_reservation_id="sr-voice-1",
            streaming_commit_id="sr-voice-1/delta/000001",
            amount_atomic_delta="2500",
            outcome="SUCCESS",
            event_time=datetime(2026, 6, 12, 3, 4, 5, 678000, tzinfo=timezone.utc),
            idempotency_key="sg-d41s-commit-1",
        )
    )

    decoded = adapter_pb2.CommitSessionDeltaRequest.FromString(req.SerializeToString())

    assert decoded.session_reservation_id == "sr-voice-1"
    assert decoded.streaming_commit_id == "sr-voice-1/delta/000001"
    assert decoded.amount_atomic_delta == "2500"
    assert decoded.outcome == adapter_pb2.CommitSessionDeltaRequest.SUCCESS
    assert decoded.event_time.seconds == 1781233445
    assert decoded.event_time.nanos == 678_000_000
    assert decoded.idempotency_key == "sg-d41s-commit-1"


def test_tp_d41s_11_builds_release_session_request() -> None:
    req = build_release_session_request(
        ReleaseSessionRequest(
            session_reservation_id="sr-voice-1",
            reason_code="session_completed",
            event_time=datetime(2026, 6, 12, 3, 5, 0, tzinfo=timezone.utc),
            idempotency_key="sg-d41s-release-1",
        )
    )

    decoded = adapter_pb2.ReleaseSessionRequest.FromString(req.SerializeToString())

    assert decoded.session_reservation_id == "sr-voice-1"
    assert decoded.reason_code == "session_completed"
    assert decoded.event_time.seconds == 1781233500
    assert decoded.event_time.nanos == 0
    assert decoded.idempotency_key == "sg-d41s-release-1"


@pytest.mark.asyncio
async def test_tp_d41s_11_client_reserve_session_fills_handshake_session() -> None:
    captured: adapter_pb2.ReserveSessionRequest | None = None

    class Stub:
        async def ReserveSession(
            self,
            req: adapter_pb2.ReserveSessionRequest,
            *,
            timeout: float,
        ) -> adapter_pb2.ReserveSessionOutcome:
            nonlocal captured
            captured = req
            assert timeout == pytest.approx(0.250)
            return adapter_pb2.ReserveSessionOutcome(
                accepted=adapter_pb2.ReserveSessionAccepted(
                    session_reservation_id="sr-voice-1",
                    ledger_transaction_id="lt-session-reserve-1",
                    audit_session_event_id="audit-session-reserve-1",
                    ttl_expires_at=_ts(1781233500),
                    reserved_amount_atomic="100000",
                    remaining_amount_atomic="100000",
                )
            )

    client = SpendGuardClient(
        socket_path="/var/run/spendguard/session-test.sock",
        tenant_id="tenant-demo",
    )
    client._stub = Stub()  # type: ignore[assignment]
    client._handshake = HANDSHAKE

    outcome = await client.reserve_session(
        ReserveSessionRequest(
            tenant_id="tenant-demo",
            budget_id="budget-voice",
            window_instance_id="018ff7d0-2c9a-7f28-8d25-cf9486b08d42",
            unit=UNIT,
            pricing=PRICING,
            session_id="",
            route="pipecat|openai-realtime|gpt-4o-mini-transcribe",
            estimated_amount_atomic="100000",
            ttl_seconds=600,
            idempotency_key="sg-d41s-reserve-client-1",
        )
    )

    assert captured is not None
    assert captured.session_id == "sidecar-handshake-session"
    assert isinstance(outcome, ReserveSessionAccepted)
    assert outcome.session_reservation_id == "sr-voice-1"
    assert outcome.ledger_transaction_id == "lt-session-reserve-1"
    assert outcome.ttl_expires_at is not None
    assert outcome.ttl_expires_at.isoformat() == "2026-06-12T03:05:00+00:00"


@pytest.mark.asyncio
async def test_tp_d41s_11_client_commit_and_release_session_map_outcomes() -> None:
    captured_commit: adapter_pb2.CommitSessionDeltaRequest | None = None
    captured_release: adapter_pb2.ReleaseSessionRequest | None = None

    class Stub:
        async def CommitSessionDelta(
            self,
            req: adapter_pb2.CommitSessionDeltaRequest,
            *,
            timeout: float,
        ) -> adapter_pb2.CommitSessionDeltaOutcome:
            nonlocal captured_commit
            captured_commit = req
            assert timeout == pytest.approx(0.500)
            return adapter_pb2.CommitSessionDeltaOutcome(
                accepted=adapter_pb2.CommitSessionDeltaAccepted(
                    session_reservation_id="sr-voice-1",
                    streaming_commit_id="sr-voice-1/delta/000001",
                    ledger_transaction_id="lt-session-commit-1",
                    audit_session_event_id="audit-session-commit-1",
                    committed_delta_atomic="2500",
                    cumulative_committed_atomic="2500",
                    remaining_amount_atomic="97500",
                    recorded_at=_ts(1781233445, 678_000_000),
                )
            )

        async def ReleaseSession(
            self,
            req: adapter_pb2.ReleaseSessionRequest,
            *,
            timeout: float,
        ) -> adapter_pb2.ReleaseSessionOutcome:
            nonlocal captured_release
            captured_release = req
            assert timeout == pytest.approx(0.150)
            return adapter_pb2.ReleaseSessionOutcome(
                accepted=adapter_pb2.ReleaseSessionAccepted(
                    session_reservation_id="sr-voice-1",
                    ledger_transaction_id="lt-session-release-1",
                    audit_session_event_id="audit-session-release-1",
                    released_amount_atomic="97500",
                    committed_amount_atomic="2500",
                    recorded_at=_ts(1781233500),
                )
            )

    client = SpendGuardClient(
        socket_path="/var/run/spendguard/session-test.sock",
        tenant_id="tenant-demo",
    )
    client._stub = Stub()  # type: ignore[assignment]
    client._handshake = HANDSHAKE

    commit = await client.commit_session_delta(
        CommitSessionDeltaRequest(
            session_reservation_id="sr-voice-1",
            streaming_commit_id="sr-voice-1/delta/000001",
            amount_atomic_delta="2500",
            outcome="SUCCESS",
            event_time=datetime(2026, 6, 12, 3, 4, 5, 678000, tzinfo=timezone.utc),
            idempotency_key="sg-d41s-commit-client-1",
        )
    )
    release = await client.release_session(
        ReleaseSessionRequest(
            session_reservation_id="sr-voice-1",
            reason_code="session_completed",
            event_time=datetime(2026, 6, 12, 3, 5, 0, tzinfo=timezone.utc),
            idempotency_key="sg-d41s-release-client-1",
        )
    )

    assert captured_commit is not None
    assert captured_commit.outcome == adapter_pb2.CommitSessionDeltaRequest.SUCCESS
    assert captured_commit.amount_atomic_delta == "2500"
    assert isinstance(commit, CommitSessionDeltaOutcome)
    assert commit.remaining_amount_atomic == "97500"
    assert commit.recorded_at is not None
    assert commit.recorded_at.isoformat() == "2026-06-12T03:04:05.678000+00:00"
    assert captured_release is not None
    assert captured_release.reason_code == "session_completed"
    assert isinstance(release, ReleaseSessionOutcome)
    assert release.released_amount_atomic == "97500"
    assert release.recorded_at is not None
    assert release.recorded_at.isoformat() == "2026-06-12T03:05:00+00:00"


@pytest.mark.parametrize(
    "amount",
    [
        "0",
        "-1",
        "1.5",
        # Non-ASCII Unicode decimal digits: isdecimal() accepts these
        # but the Rust ledger only takes b'0'..=b'9'. Must fail closed.
        "٢٥٠٠",  # Arabic-Indic ٢٥٠٠
        "१००",  # Devanagari १००
        "²",  # superscript two
    ],
)
def test_tp_d41s_13_rejects_non_positive_commit_delta(amount: str) -> None:
    with pytest.raises(ValueError):
        build_commit_session_delta_request(
            CommitSessionDeltaRequest(
                session_reservation_id="sr-voice-1",
                streaming_commit_id="sr-voice-1/delta/000002",
                amount_atomic_delta=amount,
                outcome="SUCCESS",
                event_time=datetime.now(timezone.utc),
                idempotency_key="sg-d41s-commit-2",
            )
        )


@pytest.mark.asyncio
async def test_sr_v4_handle_keeps_failed_delta_pending_and_replays_same_id() -> None:
    seen: list[CommitSessionDeltaRequest] = []
    fail_next = True

    class Client:
        async def commit_session_delta(
            self, req: CommitSessionDeltaRequest
        ) -> CommitSessionDeltaOutcome:
            nonlocal fail_next
            seen.append(req)
            if fail_next:
                fail_next = False
                raise RuntimeError("simulated network drop")
            return _accepted_commit_outcome(req)

    handle = SessionReservationHandle(
        session_reservation_id="sr-voice-1",
        max_pending_deltas=2,
    )

    with pytest.raises(RuntimeError, match="network drop"):
        await handle.commit_delta(
            Client(),
            SessionDeltaCommitInput(
                amount_atomic_delta="2500",
                outcome="SUCCESS",
                event_time=datetime(
                    2026, 6, 12, 3, 4, 5, 678000, tzinfo=timezone.utc
                ),
            ),
        )

    assert len(handle.pending_deltas) == 1
    assert (
        handle.pending_deltas[0].request.streaming_commit_id
        == "sr-voice-1/delta/000001"
    )
    assert handle.next_streaming_commit_sequence == 2

    replayed = await handle.replay_pending(Client())

    assert len(replayed) == 1
    assert len(seen) == 2
    assert seen[1].streaming_commit_id == seen[0].streaming_commit_id
    assert seen[1].idempotency_key == seen[0].idempotency_key
    assert handle.pending_deltas == ()


@pytest.mark.asyncio
async def test_sr_v4_handle_enforces_bounded_pending_delta_buffer() -> None:
    seen: list[CommitSessionDeltaRequest] = []

    class Client:
        async def commit_session_delta(
            self, req: CommitSessionDeltaRequest
        ) -> CommitSessionDeltaOutcome:
            seen.append(req)
            raise RuntimeError("sidecar unavailable")

    handle = SessionReservationHandle(
        session_reservation_id="sr-voice-1",
        max_pending_deltas=1,
    )

    with pytest.raises(RuntimeError, match="sidecar unavailable"):
        await handle.commit_delta(
            Client(),
            SessionDeltaCommitInput(
                amount_atomic_delta="1000",
                outcome="SUCCESS",
                event_time=datetime(
                    2026, 6, 12, 3, 4, 5, 678000, tzinfo=timezone.utc
                ),
            ),
        )
    assert len(handle.pending_deltas) == 1

    with pytest.raises(SessionPendingDeltaLimitError):
        await handle.commit_delta(
            Client(),
            SessionDeltaCommitInput(
                amount_atomic_delta="2000",
                outcome="SUCCESS",
                event_time=datetime(
                    2026, 6, 12, 3, 4, 6, 678000, tzinfo=timezone.utc
                ),
            ),
        )
    assert len(seen) == 1


def test_sr_v4_handle_rejects_corrupted_restore_snapshots_and_rewinded_sequence() -> None:
    handle = SessionReservationHandle(
        session_reservation_id="sr-voice-1",
        max_pending_deltas=2,
    )
    pending = handle.enqueue_delta(
        SessionDeltaCommitInput(
            amount_atomic_delta="1000",
            outcome="SUCCESS",
            event_time=datetime(2026, 6, 12, 3, 4, 5, 678000, tzinfo=timezone.utc),
        )
    )

    with pytest.raises(SessionReservationReplayMismatchError):
        SessionReservationHandle(
            session_reservation_id="sr-other",
            pending_deltas=(pending,),
        )
    with pytest.raises(SessionReservationReplayMismatchError):
        SessionReservationHandle(
            session_reservation_id="sr-voice-1",
            pending_deltas=(pending,),
            next_streaming_commit_sequence=1,
        )


@pytest.mark.asyncio
async def test_sr_v4_handle_stores_pending_requests_by_value_for_exact_replay() -> None:
    original_seconds = 1781233445
    mutated_seconds = 1781233745
    event_time = _ts(original_seconds, 678_000_000)
    seen: list[CommitSessionDeltaRequest] = []
    sent_seconds: list[int] = []
    fail_next = True

    class Client:
        async def commit_session_delta(
            self, req: CommitSessionDeltaRequest
        ) -> CommitSessionDeltaOutcome:
            nonlocal fail_next
            seen.append(req)
            if isinstance(req.event_time, Timestamp):
                sent_seconds.append(req.event_time.seconds)
                req.event_time.seconds = mutated_seconds
            if fail_next:
                fail_next = False
                raise RuntimeError("simulated network drop")
            return _accepted_commit_outcome(req)

    handle = SessionReservationHandle(
        session_reservation_id="sr-voice-1",
        max_pending_deltas=2,
    )

    with pytest.raises(RuntimeError, match="network drop"):
        await handle.commit_delta(
            Client(),
            SessionDeltaCommitInput(
                amount_atomic_delta="2500",
                outcome="SUCCESS",
                event_time=event_time,
            ),
        )
    event_time.seconds = mutated_seconds

    pending_event_time = handle.pending_deltas[0].request.event_time
    assert isinstance(pending_event_time, Timestamp)
    assert pending_event_time.seconds == original_seconds

    await handle.replay_pending(Client())

    assert len(seen) == 2
    assert sent_seconds == [original_seconds, original_seconds]
    replay_event_time = seen[1].event_time
    assert isinstance(replay_event_time, Timestamp)
    assert replay_event_time.seconds == mutated_seconds
    assert handle.snapshot().pending_deltas == ()


@pytest.mark.asyncio
async def test_sr_v4_release_finalizes_handle_and_blocks_further_deltas() -> None:
    captured: ReleaseSessionRequest | None = None
    handle = SessionReservationHandle(
        session_reservation_id="sr-voice-1",
        max_pending_deltas=2,
    )
    handle.enqueue_delta(
        SessionDeltaCommitInput(
            amount_atomic_delta="1000",
            outcome="SUCCESS",
            event_time=datetime(2026, 6, 12, 3, 4, 5, 678000, tzinfo=timezone.utc),
        )
    )

    class ReleaseClient:
        async def release_session(
            self, req: ReleaseSessionRequest
        ) -> ReleaseSessionOutcome:
            nonlocal captured
            captured = req
            return ReleaseSessionOutcome(
                session_reservation_id=req.session_reservation_id,
                ledger_transaction_id="lt-session-release-1",
                audit_session_event_id="audit-session-release-1",
                released_amount_atomic="99000",
                committed_amount_atomic="1000",
                recorded_at=datetime(2026, 6, 12, 3, 5, tzinfo=timezone.utc),
            )

    release = await handle.release(
        ReleaseClient(),
        SessionReleaseInput(
            reason_code="session_completed",
            event_time=datetime(2026, 6, 12, 3, 5, tzinfo=timezone.utc),
            idempotency_key="sg-d41s-release-handle-1",
        ),
    )

    assert captured is not None
    assert captured.session_reservation_id == "sr-voice-1"
    assert release.released_amount_atomic == "99000"
    assert handle.released is True
    assert handle.pending_deltas == ()

    class CommitClient:
        async def commit_session_delta(
            self, req: CommitSessionDeltaRequest
        ) -> CommitSessionDeltaOutcome:
            return _accepted_commit_outcome(req)

    with pytest.raises(SessionReservationReleasedError):
        await handle.commit_delta(
            CommitClient(),
            SessionDeltaCommitInput(
                amount_atomic_delta="1",
                outcome="SUCCESS",
                event_time=datetime(2026, 6, 12, 3, 5, 1, tzinfo=timezone.utc),
            ),
        )
