# ruff: noqa: ANN001, ANN201, ANN401, S101, S106
"""Shared fixtures for D24 AutoGen / AG2 adapter tests.

Provides ``FakeChatCompletionClient`` — subclass of the real
``autogen_core.models.ChatCompletionClient`` ABC when ``autogen-core``
is installed, plain base class otherwise. Per review-standards §7.3
unit tests MUST subclass the real ABC (not ``MagicMock(spec=...)``)
when available; the fallback covers CI environments where the extra
isn't installed and still preserves the same surface for the unit
suite (the wrapper's runtime branch chooses ``_ClientBase`` the same
way).

Also provides ``make_client_mock`` for mocking the SpendGuardClient
interface — mirrors ``test_agno_pre_post.py``'s helper exactly so the
two suites are visually identical.
"""

from __future__ import annotations

from collections.abc import AsyncIterator
from types import SimpleNamespace
from typing import Any
from unittest.mock import AsyncMock, MagicMock

# Try to import the real ABC so FakeChatCompletionClient subclasses it
# per review-standards §7.3 when autogen-core is available.
try:  # pragma: no cover — branch chosen at import time
    from autogen_core.models import (  # type: ignore[import-not-found]
        ChatCompletionClient as _RealChatCompletionClient,
    )

    AUTOGEN_CORE_AVAILABLE = True
except ImportError:  # pragma: no cover — branch chosen at import time
    _RealChatCompletionClient = None  # type: ignore[assignment, misc]
    AUTOGEN_CORE_AVAILABLE = False


# Resolve the base class. Real ABC when available, else plain object so
# the suite still runs without the extra installed. The wrapper class
# in ``_hook.py`` uses the same branch shape so behavior matches.
if _RealChatCompletionClient is not None:  # pragma: no cover
    _FakeBase: Any = _RealChatCompletionClient
else:
    _FakeBase = object


class FakeChatCompletionClient(_FakeBase):
    """Hand-rolled ``ChatCompletionClient`` for unit tests.

    Per review-standards §7.3 this subclasses the real ABC (when
    importable) so upstream signature changes break compile-time
    instead of silently. Returns configurable ``CreateResult``-shaped
    objects via ``SimpleNamespace`` so the unit suite runs without
    pulling in the full ``autogen_core.models`` proto surface.

    Methods raise / return based on the constructor knobs:
        - ``raise_on_create``: when set, ``create()`` raises this
          exception (FAILURE / CANCELLED unit-test paths).
        - ``usage_prompt_tokens`` / ``usage_completion_tokens``: drive
          the ``CreateResult.usage`` shape ``_extract_total_tokens``
          reads.
        - ``track_calls``: when True, every ``create()`` call appends
          its ``(messages, tools, json_output, extra_create_args,
          cancellation_token)`` kwargs to ``calls`` for assertion.
    """

    def __init__(
        self,
        *,
        usage_prompt_tokens: int = 30,
        usage_completion_tokens: int = 70,
        raise_on_create: BaseException | None = None,
        track_calls: bool = True,
        no_usage: bool = False,
        capabilities: Any = None,
        model_info: Any = None,
    ) -> None:
        # NOTE: do not call super().__init__() — the wrapper's
        # __init__ skips it (review-standards §1.2) and so should the
        # test fake; mirrors the production class's contract.
        self._prompt_tokens = usage_prompt_tokens
        self._completion_tokens = usage_completion_tokens
        self._raise_on_create = raise_on_create
        self._track_calls = track_calls
        self._no_usage = no_usage
        self._capabilities = capabilities
        self._model_info = model_info
        self.calls: list[dict[str, Any]] = []
        # Counters for pass-through assertions.
        self.count_tokens_calls = 0
        self.total_usage_calls = 0
        self.actual_usage_calls = 0
        self.remaining_tokens_calls = 0

    async def create(
        self,
        messages: list[Any],
        *,
        tools: Any = (),
        tool_choice: Any = "auto",
        json_output: Any = None,
        extra_create_args: dict[str, Any] | None = None,
        cancellation_token: Any = None,
        **_kwargs: Any,
    ) -> Any:
        if self._track_calls:
            self.calls.append(
                {
                    "messages": messages,
                    "tools": tools,
                    "tool_choice": tool_choice,
                    "json_output": json_output,
                    "extra_create_args": extra_create_args,
                    "cancellation_token": cancellation_token,
                }
            )
        if self._raise_on_create is not None:
            raise self._raise_on_create
        if self._no_usage:
            return SimpleNamespace(content="hi", usage=None)
        return SimpleNamespace(
            content="hi",
            usage=SimpleNamespace(
                prompt_tokens=self._prompt_tokens,
                completion_tokens=self._completion_tokens,
            ),
        )

    def create_stream(
        self,
        messages: list[Any],
        *,
        tools: Any = (),
        tool_choice: Any = "auto",
        json_output: Any = None,
        extra_create_args: dict[str, Any] | None = None,
        cancellation_token: Any = None,
        **_kwargs: Any,
    ) -> AsyncIterator[Any]:
        async def _stream() -> AsyncIterator[Any]:
            yield SimpleNamespace(content="chunk-1")
            yield SimpleNamespace(content="chunk-2")
            # Final ``CreateResult`` shape after the last chunk —
            # matches autogen_core 0.4 / ag2 0.7 stream protocol.
            yield SimpleNamespace(
                content="done",
                usage=SimpleNamespace(
                    prompt_tokens=self._prompt_tokens,
                    completion_tokens=self._completion_tokens,
                ),
            )

        return _stream()

    async def close(self) -> None:
        """Match autogen-core 0.7.x ABC — close is abstract there."""
        # FakeChatCompletionClient has no resources to release; this
        # method exists so the wrapper's pass-through close() (and any
        # AssistantAgent-side close on shutdown) finds a no-op.

    def actual_usage(self) -> Any:
        self.actual_usage_calls += 1
        return SimpleNamespace(prompt_tokens=1, completion_tokens=2)

    def total_usage(self) -> Any:
        self.total_usage_calls += 1
        return SimpleNamespace(prompt_tokens=3, completion_tokens=4)

    def count_tokens(self, messages: list[Any], *, tools: Any = ()) -> int:
        self.count_tokens_calls += 1
        return len(messages) * 10

    def remaining_tokens(
        self, messages: list[Any], *, tools: Any = ()
    ) -> int:
        self.remaining_tokens_calls += 1
        return 1000 - (len(messages) * 10)

    @property
    def capabilities(self) -> Any:
        return self._capabilities or {"vision": False, "function_calling": True}

    @property
    def model_info(self) -> Any:
        return self._model_info or {"family": "test", "vision": False}


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
    "AUTOGEN_CORE_AVAILABLE",
    "FakeChatCompletionClient",
    "make_client_mock",
]
