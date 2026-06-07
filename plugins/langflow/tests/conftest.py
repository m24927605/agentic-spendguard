"""Shared fixtures for plugins/langflow tests.

Skip the entire suite when ``langflow`` isn't importable so unit tests
run cleanly in dev environments without the full Langflow install.
Tests that exercise the SDK fake-sidecar pattern also need
``spendguard-sdk[langchain]`` available; that's covered by a separate
``importorskip`` in each test module.
"""

from __future__ import annotations

import asyncio
import sys
from pathlib import Path
from typing import Any
from unittest.mock import AsyncMock, MagicMock

import pytest


# Make ``spendguard_langflow`` importable without an editable install
# so the unit suite runs straight from the source checkout.
_PKG_ROOT = Path(__file__).resolve().parent.parent / "src"
if str(_PKG_ROOT) not in sys.path:
    sys.path.insert(0, str(_PKG_ROOT))


def get_request_decision_mock(client: Any) -> Any:
    """Walk the ``functools.wraps`` chain to find the underlying AsyncMock.

    ``install_decision_context`` (and the demo's ``_tag_stream_context``)
    wraps ``client.request_decision`` with a thin tagging closure
    decorated via ``functools.wraps``. ``functools.wraps`` sets
    ``__wrapped__`` so test code can drill back to the original
    AsyncMock and read ``call_args``/``await_count``/``call_args_list``.
    """
    target = client.request_decision
    while hasattr(target, "__wrapped__"):
        target = target.__wrapped__
    return target


@pytest.fixture
def mock_client() -> Any:
    """Return a fake :class:`SpendGuardClient` shaped for unit tests.

    Constructs a real :class:`SpendGuardClient` (so Pydantic's strict
    isinstance check on ``SpendGuardChatModel.client`` is satisfied)
    then monkey-patches the IO surface (``connect``, ``handshake``,
    ``request_decision``, ``emit_llm_call_post``) with ``AsyncMock``.

    Records every ``request_decision`` and ``emit_llm_call_post`` call
    so tests can assert call ordering + payloads.
    """
    pytest.importorskip("spendguard.integrations.langchain")
    from spendguard import SpendGuardClient
    from spendguard.client import DecisionOutcome

    client = SpendGuardClient(
        socket_path="/dev/null",
        tenant_id="00000000-0000-4000-8000-000000000001",
    )
    # Forge a handshake sentinel so .session_id property returns instead
    # of raising HandshakeError. The SDK reads `_handshake.session_id`.
    from types import SimpleNamespace

    client._handshake = SimpleNamespace(session_id="test-session-id")
    client.connect = AsyncMock(return_value=None)
    client.handshake = AsyncMock(return_value=None)
    outcome = DecisionOutcome(
        decision_id="decision-1",
        audit_decision_event_id="audit-1",
        decision="CONTINUE",
        mutation_patch_json="",
        effect_hash=b"",
        ledger_transaction_id="ledger-1",
        reservation_ids=("reservation-1",),
        ttl_expires_at_seconds=0,
        reason_codes=(),
        matched_rule_ids=(),
    )
    client.request_decision = AsyncMock(return_value=outcome)
    client.emit_llm_call_post = AsyncMock(return_value=None)
    return client


@pytest.fixture
def fake_inner() -> Any:
    """Return a ``FakeListChatModel`` shaped for unit tests."""
    pytest.importorskip("langchain_core")
    from langchain_core.language_models.fake_chat_models import (
        FakeListChatModel,
    )

    return FakeListChatModel(responses=["hi from fake inner"])


@pytest.fixture
def make_component() -> Any:
    """Factory for SpendGuardChatModelWrapper instances under test.

    Avoids the full Langflow component runtime -- we instantiate the
    class then patch attributes directly, mirroring how the Component
    base resolves canvas inputs at runtime.
    """
    pytest.importorskip("langflow")

    def _factory(**kwargs: Any) -> Any:
        from spendguard_langflow.component import SpendGuardChatModelWrapper

        comp = SpendGuardChatModelWrapper()
        for k, v in kwargs.items():
            setattr(comp, k, v)
        return comp

    return _factory


@pytest.fixture
def run_async() -> Any:
    """Run an async function via asyncio.run for sync-test ergonomics."""

    def _runner(coro: Any) -> Any:
        return asyncio.run(coro)

    return _runner
