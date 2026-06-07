# ruff: noqa: ANN001, ANN201, ANN401, S101, S106
"""Shared fixtures for D26 Letta adapter tests.

Provides ``FakeLLMClient`` — subclass of the real
``letta.llm_api.llm_client_base.LLMClientBase`` ABC when ``letta`` is
installed, plain base class otherwise. Per review-standards §7.3 unit
tests MUST subclass the real ABC (not ``MagicMock(spec=...)``) when
available; the fallback covers CI environments where the extra isn't
installed and still preserves the same surface for the unit suite (the
wrapper's runtime branch chooses ``_ClientBase`` the same way).

Also provides ``make_client_mock`` for mocking the SpendGuardClient
interface — mirrors ``test_autogen.py``'s helper exactly so the two
suites are visually identical.
"""

from __future__ import annotations

from types import SimpleNamespace
from typing import Any
from unittest.mock import AsyncMock, MagicMock

# Try to import the real ABC so FakeLLMClient subclasses it per
# review-standards §7.3 when letta is available.
try:  # pragma: no cover — branch chosen at import time
    from letta.llm_api.llm_client_base import (  # type: ignore[import-not-found]
        LLMClientBase as _RealLLMClientBase,
    )

    LETTA_AVAILABLE = True
except ImportError:  # pragma: no cover — branch chosen at import time
    _RealLLMClientBase = None  # type: ignore[assignment, misc]
    LETTA_AVAILABLE = False


# Resolve the base class. Real ABC when available, else plain object so
# the suite still runs without the extra installed. The wrapper class
# in ``_hook.py`` uses the same branch shape so behavior matches.
if _RealLLMClientBase is not None:  # pragma: no cover
    _FakeBase: Any = _RealLLMClientBase
else:
    _FakeBase = object


class FakeLLMClient(_FakeBase):
    """Hand-rolled ``LLMClientBase`` for unit tests.

    Per review-standards §7.3 this subclasses the real ABC (when
    importable) so upstream signature changes break compile-time
    instead of silently. Returns configurable
    ``ChatCompletionResponse``-shaped objects via ``SimpleNamespace``
    so the unit suite runs without pulling in the full
    ``letta.schemas`` proto surface.

    Methods raise / return based on the constructor knobs:
        - ``raise_on_send``: when set, ``send_llm_request()`` /
          ``send_llm_request_sync()`` raises this exception (FAILURE /
          CANCELLED unit-test paths).
        - ``usage_total_tokens`` / ``usage_prompt_tokens`` /
          ``usage_completion_tokens``: drive the
          ``ChatCompletionResponse.usage`` shape ``_extract_total_tokens``
          reads.
        - ``response_id``: drive the response ``id`` field
          ``_extract_provider_event_id`` reads.
        - ``track_calls``: when True, every ``send_llm_request()``
          call appends its ``(request_data, llm_config, tools,
          force_tool_use)`` kwargs to ``calls`` for assertion.
        - ``llm_config_value`` / ``provider_value``: drive
          ``__getattr__`` delegation tests.
    """

    def __init__(
        self,
        *,
        usage_prompt_tokens: int = 30,
        usage_completion_tokens: int = 70,
        usage_total_tokens: int | None = 100,
        response_id: str = "chatcmpl-fake-1",
        raise_on_send: BaseException | None = None,
        track_calls: bool = True,
        no_usage: bool = False,
        llm_config_value: Any = None,
        provider_value: str = "fake-provider",
        build_request_data_value: Any = None,
    ) -> None:
        # NOTE: do not call super().__init__() — the wrapper's
        # __init__ skips it (review-standards §1.2) and so should the
        # test fake; mirrors the production class's contract.
        self._prompt_tokens = usage_prompt_tokens
        self._completion_tokens = usage_completion_tokens
        self._total_tokens = usage_total_tokens
        self._response_id = response_id
        self._raise_on_send = raise_on_send
        self._track_calls = track_calls
        self._no_usage = no_usage
        # Public attrs the wrapper's __getattr__ should delegate to.
        self.llm_config = llm_config_value or SimpleNamespace(
            model="gpt-4o-mini",
            model_endpoint_type="openai",
        )
        self.provider = provider_value
        self._build_request_data_value = build_request_data_value or {
            "messages": [],
            "model": "gpt-4o-mini",
        }
        self.calls: list[dict[str, Any]] = []
        # Counter for build_request_data delegation tests.
        self.build_request_data_calls = 0

    async def send_llm_request(
        self,
        request_data: Any,
        llm_config: Any,
        tools: Any = None,
        force_tool_use: bool = False,
        **_kwargs: Any,
    ) -> Any:
        if self._track_calls:
            self.calls.append(
                {
                    "request_data": request_data,
                    "llm_config": llm_config,
                    "tools": tools,
                    "force_tool_use": force_tool_use,
                }
            )
        if self._raise_on_send is not None:
            raise self._raise_on_send
        if self._no_usage:
            return SimpleNamespace(
                id=self._response_id, content="hi", usage=None
            )
        return SimpleNamespace(
            id=self._response_id,
            content="hi",
            usage=SimpleNamespace(
                prompt_tokens=self._prompt_tokens,
                completion_tokens=self._completion_tokens,
                total_tokens=self._total_tokens,
            ),
        )

    def send_llm_request_sync(
        self,
        request_data: Any,
        llm_config: Any,
        tools: Any = None,
        force_tool_use: bool = False,
        **_kwargs: Any,
    ) -> Any:
        """Sync sibling — older Letta code uses this."""
        if self._track_calls:
            self.calls.append(
                {
                    "request_data": request_data,
                    "llm_config": llm_config,
                    "tools": tools,
                    "force_tool_use": force_tool_use,
                }
            )
        if self._raise_on_send is not None:
            raise self._raise_on_send
        return SimpleNamespace(
            id=self._response_id,
            content="hi-sync",
            usage=SimpleNamespace(
                prompt_tokens=self._prompt_tokens,
                completion_tokens=self._completion_tokens,
                total_tokens=self._total_tokens,
            ),
        )

    # ─────────────────────────────────────────────────────────────────
    # Pass-through helpers — used by tests asserting ``__getattr__``
    # delegation. Mirrors the real LLMClientBase surface enough for
    # review-standards §4 pass-through coverage.
    # ─────────────────────────────────────────────────────────────────

    def build_request_data(self, *args: Any, **kwargs: Any) -> Any:
        """Mirrors ``LLMClientBase.build_request_data``."""
        self.build_request_data_calls += 1
        return self._build_request_data_value

    def convert_response_to_chat_completion(
        self, response: Any, *_args: Any, **_kwargs: Any
    ) -> Any:
        """Mirrors ``LLMClientBase.convert_response_to_chat_completion``."""
        return response


def make_client_mock(
    *,
    tenant_id: str = "tenant-1",
    session_id: str = "session-1",
    decision_id: str = "dec-1",
    reservation_ids: tuple[str, ...] = ("res-1",),
    decision: str = "CONTINUE",
    request_decision_side_effect: Any = None,
) -> MagicMock:
    """Build an ``AsyncMock`` shaped like a connected SpendGuardClient."""
    client = MagicMock()
    client.tenant_id = tenant_id
    client.session_id = session_id

    outcome = SimpleNamespace(
        decision_id=decision_id,
        reservation_ids=reservation_ids,
        audit_decision_event_id="audit-1",
        decision=decision,
    )
    if request_decision_side_effect is not None:
        client.request_decision = AsyncMock(
            side_effect=request_decision_side_effect
        )
    else:
        client.request_decision = AsyncMock(return_value=outcome)
    client.emit_llm_call_post = AsyncMock(return_value=None)
    return client


__all__ = [
    "LETTA_AVAILABLE",
    "FakeLLMClient",
    "make_client_mock",
]
