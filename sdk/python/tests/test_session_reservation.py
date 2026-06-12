"""D41S_01 session reservation contract/proto skeleton tests."""

from __future__ import annotations

from datetime import datetime, timezone

import pytest

from spendguard._proto.spendguard.common.v1 import common_pb2
from spendguard._proto.spendguard.sidecar_adapter.v1 import (
    adapter_pb2,
    adapter_pb2_grpc,
)
from spendguard.session import (
    CommitSessionDeltaRequest,
    ReleaseSessionRequest,
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
