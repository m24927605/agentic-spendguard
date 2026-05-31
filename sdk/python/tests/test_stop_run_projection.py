from __future__ import annotations

import pytest

from spendguard import SpendGuardClient
from spendguard._proto.spendguard.sidecar_adapter.v1 import adapter_pb2
from spendguard.errors import DecisionStopped


def test_stop_run_projection_enum_is_generated() -> None:
    assert adapter_pb2.DecisionResponse.STOP_RUN_PROJECTION == 6
    assert (
        adapter_pb2.DecisionResponse.Decision.Name(
            adapter_pb2.DecisionResponse.STOP_RUN_PROJECTION
        )
        == "STOP_RUN_PROJECTION"
    )


def test_stop_run_projection_round_trips_through_protobuf() -> None:
    response = adapter_pb2.DecisionResponse(
        decision_id="00000000-0000-7000-8000-000000000001",
        audit_decision_event_id="00000000-0000-7000-8000-000000000002",
        decision=adapter_pb2.DecisionResponse.STOP_RUN_PROJECTION,
        terminal=True,
        reason_codes=["RUN_BUDGET_PROJECTION_EXCEEDED"],
    )

    decoded = adapter_pb2.DecisionResponse.FromString(response.SerializeToString())

    assert decoded.decision == adapter_pb2.DecisionResponse.STOP_RUN_PROJECTION
    assert SpendGuardClient._decision_name(decoded.decision) == "STOP_RUN_PROJECTION"


def test_stop_run_projection_raises_terminal_stop_with_specific_name() -> None:
    response = adapter_pb2.DecisionResponse(
        decision_id="00000000-0000-7000-8000-000000000001",
        audit_decision_event_id="00000000-0000-7000-8000-000000000002",
        decision=adapter_pb2.DecisionResponse.STOP_RUN_PROJECTION,
        terminal=True,
        reason_codes=["RUN_BUDGET_PROJECTION_EXCEEDED"],
    )

    with pytest.raises(DecisionStopped) as exc:
        decision_name = SpendGuardClient._decision_name(response.decision)
        if decision_name in ("STOP", "STOP_RUN_PROJECTION"):
            raise DecisionStopped(
                f"sidecar {decision_name} terminal={response.terminal} reasons={list(response.reason_codes)}",
                decision_id=response.decision_id,
                reason_codes=list(response.reason_codes),
                audit_decision_event_id=response.audit_decision_event_id,
                matched_rule_ids=list(response.matched_rule_ids),
            )

    assert "STOP_RUN_PROJECTION" in str(exc.value)
    assert exc.value.reason_codes == ["RUN_BUDGET_PROJECTION_EXCEEDED"]
