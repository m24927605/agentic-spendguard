"""Unit tests for ``with_run_plan`` → ``DecisionRequest.planned_steps_hint`` wire.

Verifies the Phase C wiring change in ``client.py::request_decision``
reads the ``with_run_plan`` context-var and stamps the right
``planned_steps_hint`` value on the outgoing gRPC request.

We intercept the wire-level ``DecisionRequest`` by stubbing the gRPC
stub so we can assert on the field without running a real sidecar.
"""

from __future__ import annotations

from typing import Any
from unittest.mock import AsyncMock, MagicMock

import pytest

from spendguard import (
    SpendGuardClient,
    current_run_plan,
    derive_idempotency_key,
    with_run_plan,
)
from spendguard._proto.spendguard.common.v1 import common_pb2
from spendguard._proto.spendguard.sidecar_adapter.v1 import adapter_pb2


def _build_response_continue() -> adapter_pb2.DecisionResponse:
    """Minimal CONTINUE response."""
    resp = adapter_pb2.DecisionResponse()
    resp.decision_id = "dec-1"
    resp.audit_decision_event_id = "evt-1"
    resp.decision = adapter_pb2.DecisionResponse.CONTINUE
    return resp


@pytest.fixture
def client_with_stub() -> tuple[SpendGuardClient, MagicMock]:
    """SpendGuardClient with a mocked stub that captures the request."""
    c = SpendGuardClient(socket_path="/dev/null", tenant_id="t1")
    stub = MagicMock()
    stub.RequestDecision = AsyncMock(return_value=_build_response_continue())
    c._stub = stub  # type: ignore[attr-defined]
    # Inject a fake handshake outcome so .session_id resolves without
    # touching the network.
    from spendguard.client import HandshakeOutcome

    c._handshake = HandshakeOutcome(  # type: ignore[attr-defined]
        session_id="s1",
        sidecar_version="test",
        schema_bundle_id="",
        schema_bundle_hash=b"",
        contract_bundle_id="",
        contract_bundle_hash=b"",
        capability_required=0x40,
        signing_key_id="",
        announcement_signature=b"",
    )
    return c, stub


async def _invoke_request_decision(client: SpendGuardClient) -> Any:
    return await client.request_decision(
        trigger="LLM_CALL_PRE",
        run_id="00000000-0000-7000-8000-000000000001",
        step_id="step-1",
        llm_call_id="00000000-0000-7000-8000-000000000002",
        tool_call_id="",
        decision_id="00000000-0000-7000-8000-000000000003",
        route="llm.call",
        projected_claims=[
            common_pb2.BudgetClaim(
                budget_id="b1",
                unit=common_pb2.UnitRef(unit_id="usd_micros"),
                amount_atomic="100",
                direction=common_pb2.BudgetClaim.DEBIT,
                window_instance_id="w1",
            )
        ],
        idempotency_key="idem-1",
    )


class TestPlannedStepsHintWire:
    @pytest.mark.asyncio
    async def test_no_run_plan_sends_zero_hint(
        self, client_with_stub: tuple[SpendGuardClient, MagicMock]
    ) -> None:
        client, stub = client_with_stub
        # Sanity: no plan active
        assert current_run_plan() is None
        await _invoke_request_decision(client)
        # Inspect the captured DecisionRequest
        assert stub.RequestDecision.await_count == 1
        captured: adapter_pb2.DecisionRequest = (
            stub.RequestDecision.await_args.args[0]
        )
        assert captured.planned_steps_hint == 0

    @pytest.mark.asyncio
    async def test_with_run_plan_sets_hint(
        self, client_with_stub: tuple[SpendGuardClient, MagicMock]
    ) -> None:
        client, stub = client_with_stub

        @with_run_plan(planned_calls=8, planned_tools=2)
        async def run() -> None:
            await _invoke_request_decision(client)

        await run()
        captured: adapter_pb2.DecisionRequest = (
            stub.RequestDecision.await_args.args[0]
        )
        assert captured.planned_steps_hint == 10

    @pytest.mark.asyncio
    async def test_with_run_plan_zero_tools_sets_hint_to_calls(
        self, client_with_stub: tuple[SpendGuardClient, MagicMock]
    ) -> None:
        client, stub = client_with_stub

        @with_run_plan(planned_calls=5)
        async def run() -> None:
            await _invoke_request_decision(client)

        await run()
        captured = stub.RequestDecision.await_args.args[0]
        assert captured.planned_steps_hint == 5

    @pytest.mark.asyncio
    async def test_with_run_plan_nested_outer_wins_on_wire(
        self, client_with_stub: tuple[SpendGuardClient, MagicMock]
    ) -> None:
        client, stub = client_with_stub

        @with_run_plan(planned_calls=100, planned_tools=50)
        async def outer() -> None:
            @with_run_plan(planned_calls=1, planned_tools=1)
            async def inner() -> None:
                await _invoke_request_decision(client)

            await inner()

        await outer()
        captured = stub.RequestDecision.await_args.args[0]
        # Outer wins: 100 + 50 = 150
        assert captured.planned_steps_hint == 150

    @pytest.mark.asyncio
    async def test_after_with_run_plan_exits_hint_clears(
        self, client_with_stub: tuple[SpendGuardClient, MagicMock]
    ) -> None:
        client, stub = client_with_stub

        @with_run_plan(planned_calls=5)
        async def run() -> None:
            await _invoke_request_decision(client)

        await run()

        # Reset mock + invoke OUTSIDE the decorator → hint=0 again
        stub.RequestDecision.reset_mock()
        stub.RequestDecision.return_value = _build_response_continue()

        await _invoke_request_decision(client)
        captured = stub.RequestDecision.await_args.args[0]
        assert captured.planned_steps_hint == 0
