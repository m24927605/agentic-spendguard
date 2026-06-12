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


@pytest.mark.parametrize("amount", ["0", "-1", "1.5"])
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
