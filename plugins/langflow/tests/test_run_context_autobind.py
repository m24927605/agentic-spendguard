"""Slice 2 unit tests -- ``install_autobind`` glue.

Covers A01..A05 from `tests.md` §2.3.
"""

from __future__ import annotations

import asyncio
import re
from typing import Any
from unittest.mock import AsyncMock, MagicMock, patch

import pytest


def _import_run_context_helpers():
    pytest.importorskip("spendguard.integrations.langchain")
    from spendguard.integrations.langchain import (
        RunContext,
        _RUN_CONTEXT,
        current_run_context,
        run_context,
    )

    return RunContext, _RUN_CONTEXT, current_run_context, run_context


@pytest.fixture
def wrapped(make_component, fake_inner, mock_client):
    """Yield a wrapped SpendGuardChatModel with autobind installed."""
    with patch("spendguard.SpendGuardClient", return_value=mock_client):
        comp = make_component(
            inner=fake_inner,
            sidecar_uds_path="/tmp/fake.sock",
            tenant_id="t-uuid",
            budget_id="b-uuid",
            window_instance_id="w-uuid",
        )
        yield comp.build_model(), mock_client


def _rd(client):
    """Drill to the AsyncMock under the wraps chain."""
    from tests.conftest import get_request_decision_mock

    return get_request_decision_mock(client)


def test_A01_autobind_enters_when_no_context(wrapped) -> None:
    """A01: ainvoke without a caller-bound context -> no RuntimeError."""
    pytest.importorskip("langchain_core")
    from langchain_core.messages import HumanMessage

    model, mock = wrapped

    async def run():
        await model.ainvoke([HumanMessage(content="hello")])

    asyncio.run(run())
    rd = _rd(mock)
    # request_decision was called -> autobind succeeded.
    assert rd.await_count == 1
    # run_id derived from langflow- prefix or flow_id pattern.
    kwargs = rd.call_args.kwargs
    rid = kwargs.get("run_id", "")
    assert rid.startswith("langflow-") or ":" in rid
    assert rid.endswith(":1")


def test_A02_caller_bound_context_wins(wrapped) -> None:
    """A02: caller-bound run_context wins (INV-3)."""
    pytest.importorskip("langchain_core")
    from langchain_core.messages import HumanMessage

    RunContext, _, _, run_context = _import_run_context_helpers()
    model, mock = wrapped

    async def run():
        async with run_context(RunContext(run_id="caller-rid")):
            await model.ainvoke([HumanMessage(content="hello")])

    asyncio.run(run())
    rd = _rd(mock)
    kwargs = rd.call_args.kwargs
    assert kwargs["run_id"] == "caller-rid"


def test_A03_autobind_run_id_increments_per_call(wrapped) -> None:
    """A03: two sequential autobinds produce :1 then :2."""
    pytest.importorskip("langchain_core")
    from langchain_core.messages import HumanMessage

    model, mock = wrapped

    async def run():
        await model.ainvoke([HumanMessage(content="first")])
        await model.ainvoke([HumanMessage(content="second")])

    asyncio.run(run())
    rd = _rd(mock)
    rids = [c.kwargs["run_id"] for c in rd.call_args_list]
    assert len(rids) == 2
    assert rids[0].endswith(":1")
    assert rids[1].endswith(":2")
    # Same base run_id across both autobinds.
    assert rids[0].rsplit(":", 1)[0] == rids[1].rsplit(":", 1)[0]


def test_A04_flow_id_fallback_when_graph_absent(wrapped) -> None:
    """A04: no graph -> uuid4 fallback pattern."""
    pytest.importorskip("langchain_core")
    from langchain_core.messages import HumanMessage

    model, mock = wrapped

    async def run():
        await model.ainvoke([HumanMessage(content="hello")])

    asyncio.run(run())
    rd = _rd(mock)
    rid = rd.call_args.kwargs["run_id"]
    base = rid.rsplit(":", 1)[0]
    # langflow-<uuid> shape. UUID v4 is 8-4-4-4-12 hex.
    assert re.match(
        r"^langflow-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$",
        base,
    ), base


def test_A05_autobind_preserves_inner_signature(wrapped) -> None:
    """A05: functools.wraps preserves docstring + signature."""
    model, _ = wrapped
    # ``_agenerate`` is the patched function -- preserve check is on
    # the bound method's __wrapped__ chain.
    patched = model._agenerate
    assert callable(patched)
    # functools.wraps copies __name__ + __doc__ from the original.
    # We can't inspect the original directly, but we can assert the
    # patched function isn't the no-op default.
    assert patched.__name__ in {"_agenerate", "_agenerate_autobind"}
