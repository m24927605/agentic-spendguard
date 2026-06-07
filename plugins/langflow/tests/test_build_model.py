"""Slice 2 unit tests -- ``build_model`` wires Spendguard SDK lifecycle.

Covers B01..B09 from `tests.md` §2.2.

Each test skips cleanly when ``langflow`` or ``spendguard.integrations.langchain``
isn't importable. The ``mock_client`` fixture is shared via conftest.py.
"""

from __future__ import annotations

import asyncio
import os
from unittest.mock import AsyncMock, MagicMock, patch

import pytest


def _import_sdk_chat_model():
    pytest.importorskip("spendguard.integrations.langchain")
    from spendguard.integrations.langchain import SpendGuardChatModel

    return SpendGuardChatModel


@pytest.fixture
def patch_client(mock_client):
    """Patch ``SpendGuardClient`` so ``_build_async`` uses our mock."""
    with patch("spendguard.SpendGuardClient", return_value=mock_client):
        yield mock_client


def test_B01_build_returns_spendguard_chat_model(
    make_component, fake_inner, patch_client
) -> None:
    """B01: build returns ``SpendGuardChatModel`` wrapping inner."""
    SDKChatModel = _import_sdk_chat_model()
    comp = make_component(
        inner=fake_inner,
        sidecar_uds_path="/tmp/fake.sock",
        tenant_id="00000000-0000-4000-8000-000000000001",
        budget_id="44444444-4444-4444-8444-444444444444",
        window_instance_id="55555555-5555-4555-8555-555555555555",
        unit_token_kind="output_token",
        model_family="gpt-4",
        claim_estimator_chars_per_token=4,
    )
    wrapped = comp.build_model()
    assert isinstance(wrapped, SDKChatModel)
    assert wrapped.inner is fake_inner


def test_B02_build_calls_connect_and_handshake(
    make_component, fake_inner, patch_client
) -> None:
    """B02: connect() + handshake() each awaited exactly once."""
    comp = make_component(
        inner=fake_inner,
        sidecar_uds_path="/tmp/fake.sock",
        tenant_id="00000000-0000-4000-8000-000000000001",
        budget_id="44444444-4444-4444-8444-444444444444",
        window_instance_id="55555555-5555-4555-8555-555555555555",
    )
    comp.build_model()
    assert patch_client.connect.await_count == 1
    assert patch_client.handshake.await_count == 1


def test_B03_build_propagates_canvas_inputs(
    make_component, fake_inner, patch_client
) -> None:
    """B03: budget_id / window_instance_id / unit fields land on the wrapper."""
    comp = make_component(
        inner=fake_inner,
        sidecar_uds_path="/tmp/fake.sock",
        tenant_id="t-uuid",
        budget_id="b-uuid",
        window_instance_id="w-uuid",
        unit_token_kind="output_token",
        model_family="claude-3",
    )
    wrapped = comp.build_model()
    assert wrapped.budget_id == "b-uuid"
    assert wrapped.window_instance_id == "w-uuid"
    assert wrapped.unit.unit_id == "claude-3.output_token"
    assert wrapped.unit.token_kind == "output_token"
    assert wrapped.unit.model_family == "claude-3"


def test_B04_default_estimator_uses_chars_per_token(
    make_component, fake_inner, patch_client
) -> None:
    """B04: estimator divisor = chars_per_token; floor = 50."""
    pytest.importorskip("langchain_core")
    from langchain_core.messages import HumanMessage

    comp = make_component(
        inner=fake_inner,
        sidecar_uds_path="/tmp/fake.sock",
        tenant_id="t-uuid",
        budget_id="b-uuid",
        window_instance_id="w-uuid",
        claim_estimator_chars_per_token=8,
    )
    wrapped = comp.build_model()
    short = [HumanMessage(content="x" * 32)]
    long = [HumanMessage(content="y" * 800)]
    short_claims = wrapped.claim_estimator(short)
    long_claims = wrapped.claim_estimator(long)
    # short = 32 // 8 = 4 -> floored to 50
    assert int(short_claims[0].amount_atomic) == 50
    # long = 800 // 8 = 100 -> wins over floor
    assert int(long_claims[0].amount_atomic) == 100


def test_B05_missing_uds_raises_valueerror(
    make_component, fake_inner, patch_client, monkeypatch
) -> None:
    """B05: empty UDS canvas input + missing env -> ValueError naming both."""
    monkeypatch.delenv("SPENDGUARD_SIDECAR_UDS", raising=False)
    comp = make_component(
        inner=fake_inner,
        sidecar_uds_path="",
        tenant_id="t-uuid",
        budget_id="b-uuid",
        window_instance_id="w-uuid",
    )
    with pytest.raises(ValueError) as ei:
        comp.build_model()
    msg = str(ei.value)
    assert "SpendGuard Sidecar UDS Path" in msg
    assert "SPENDGUARD_SIDECAR_UDS" in msg


def test_B06_uds_env_fallback(
    make_component, fake_inner, patch_client, monkeypatch
) -> None:
    """B06: env var fills in when canvas UDS is blank."""
    monkeypatch.setenv("SPENDGUARD_SIDECAR_UDS", "/tmp/env-fallback.sock")
    comp = make_component(
        inner=fake_inner,
        sidecar_uds_path="",
        tenant_id="t-uuid",
        budget_id="b-uuid",
        window_instance_id="w-uuid",
    )
    wrapped = comp.build_model()
    assert wrapped is not None


def test_B07_running_loop_raises(make_component, fake_inner, patch_client) -> None:
    """B07: build_model inside a running loop -> RuntimeError with version hint."""
    pytest.importorskip("langflow")
    comp = make_component(
        inner=fake_inner,
        sidecar_uds_path="/tmp/fake.sock",
        tenant_id="t-uuid",
        budget_id="b-uuid",
        window_instance_id="w-uuid",
    )

    async def runner():
        comp.build_model()

    with pytest.raises(RuntimeError) as ei:
        asyncio.run(runner())
    msg = str(ei.value)
    assert "running event loop" in msg
    assert "Langflow" in msg


def test_B08_invoke_after_build_routes_through_sidecar(
    make_component, fake_inner, patch_client
) -> None:
    """B08: ainvoke() -> request_decision PRE + emit_llm_call_post POST."""
    pytest.importorskip("langchain_core")
    from langchain_core.messages import HumanMessage

    from tests.conftest import get_request_decision_mock

    comp = make_component(
        inner=fake_inner,
        sidecar_uds_path="/tmp/fake.sock",
        tenant_id="t-uuid",
        budget_id="b-uuid",
        window_instance_id="w-uuid",
    )
    wrapped = comp.build_model()

    async def run() -> None:
        await wrapped.ainvoke([HumanMessage(content="hello")])

    asyncio.run(run())
    rd_mock = get_request_decision_mock(patch_client)
    assert rd_mock.await_count == 1
    # POST emit fires only when reservation_ids present, which our
    # mock outcome sets. emit_llm_call_post should be awaited once.
    assert patch_client.emit_llm_call_post.await_count == 1


def test_B09_invoke_deny_does_not_call_inner(
    make_component, fake_inner, patch_client
) -> None:
    """B09: DENY raised by sidecar -> inner._agenerate NEVER fires (INV-1)."""
    pytest.importorskip("langchain_core")
    pytest.importorskip("spendguard")
    from langchain_core.messages import HumanMessage
    from spendguard.errors import DecisionDenied

    patch_client.request_decision = AsyncMock(
        side_effect=DecisionDenied(
            "budget exhausted",
            decision_id="decision-deny-1",
            reason_codes=["BUDGET_EXHAUSTED"],
        )
    )

    # Track inner _agenerate calls. Use MagicMock wrapping the bound
    # method so the assertion is unambiguous.
    inner_spy = MagicMock(wraps=fake_inner._agenerate)
    object.__setattr__(fake_inner, "_agenerate", AsyncMock(wraps=inner_spy))

    comp = make_component(
        inner=fake_inner,
        sidecar_uds_path="/tmp/fake.sock",
        tenant_id="t-uuid",
        budget_id="b-uuid",
        window_instance_id="w-uuid",
    )
    wrapped = comp.build_model()

    async def run() -> None:
        with pytest.raises(DecisionDenied):
            await wrapped.ainvoke([HumanMessage(content="denied")])

    asyncio.run(run())
    assert fake_inner._agenerate.await_count == 0
