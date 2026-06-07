# ruff: noqa: ANN001, ANN201, ANN401, S101, S106
"""Shared fixtures for D25 SmolAgents adapter tests.

Provides ``FakeSmolModel`` — subclass of the real ``smolagents.Model``
ABC when ``smolagents`` is installed, plain base class otherwise.
Per review-standards §8.3 unit tests MUST subclass the real ABC (not
``MagicMock(spec=...)``) when available; the fallback covers CI
environments where the extra isn't installed and still preserves the
same surface for the unit suite (the wrapper's runtime branch chooses
``_ModelBase`` the same way).

Also provides ``make_client_mock`` for mocking the SpendGuardClient
interface — mirrors ``conftest_autogen.py``'s helper exactly so the
two suites are visually identical.
"""

from __future__ import annotations

from types import SimpleNamespace
from typing import Any
from unittest.mock import AsyncMock, MagicMock

# Try to import the real ABC so FakeSmolModel subclasses it per
# review-standards §8.3 when smolagents is available.
try:  # pragma: no cover — branch chosen at import time
    from smolagents import Model as _RealSmolModel  # type: ignore[attr-defined]

    SMOLAGENTS_AVAILABLE = True
except ImportError:  # pragma: no cover — branch chosen at import time
    _RealSmolModel = None  # type: ignore[assignment, misc]
    SMOLAGENTS_AVAILABLE = False


# Try to import ChatMessage + TokenUsage so the fake returns the real
# shape when smolagents is installed (drives the wrapper's
# `_extract_total_tokens` through the same `token_usage.input_tokens +
# output_tokens` extraction path the production code hits).
try:  # pragma: no cover — branch chosen at import time
    from smolagents.models import ChatMessage, MessageRole  # type: ignore[attr-defined]
    from smolagents.monitoring import TokenUsage  # type: ignore[attr-defined]

    CHATMESSAGE_AVAILABLE = True
except ImportError:  # pragma: no cover — branch chosen at import time
    ChatMessage = None  # type: ignore[assignment, misc]
    MessageRole = None  # type: ignore[assignment, misc]
    TokenUsage = None  # type: ignore[assignment, misc]
    CHATMESSAGE_AVAILABLE = False


# Resolve the base class. Real ABC when available, else plain object so
# the suite still runs without the extra installed. The wrapper class
# in ``_hook.py`` uses the same branch shape so behavior matches.
if _RealSmolModel is not None:  # pragma: no cover
    _FakeBase: Any = _RealSmolModel
else:
    _FakeBase = object


def _make_chat_message(
    *,
    content: str = "hi",
    input_tokens: int = 30,
    output_tokens: int = 70,
    no_usage: bool = False,
) -> Any:
    """Construct a ``ChatMessage`` (or duck-typed stand-in) for tests.

    Drives the wrapper's ``_extract_total_tokens`` through the same
    extraction path the production code hits.
    """
    if CHATMESSAGE_AVAILABLE:
        # smolagents.monitoring.TokenUsage __init__ takes only
        # (input_tokens, output_tokens); total_tokens is a derived
        # post-init field. Verified against smolagents 1.26 wheel.
        usage = (
            None
            if no_usage
            else TokenUsage(
                input_tokens=input_tokens,
                output_tokens=output_tokens,
            )
        )
        return ChatMessage(
            role=MessageRole.ASSISTANT,
            content=content,
            tool_calls=None,
            raw=None,
            token_usage=usage,
        )
    # Duck-typed fallback for environments without smolagents.
    return SimpleNamespace(
        role="assistant",
        content=content,
        tool_calls=None,
        raw=None,
        token_usage=(
            None
            if no_usage
            else SimpleNamespace(
                input_tokens=input_tokens, output_tokens=output_tokens
            )
        ),
    )


class FakeSmolModel(_FakeBase):  # type: ignore[misc, valid-type]
    """Hand-rolled ``smolagents.Model`` subclass for unit tests.

    Per review-standards §8.3 this subclasses the real ABC (when
    importable) so upstream signature changes break at compile time
    instead of silently. Returns configurable ``ChatMessage`` shapes
    via the helper above.

    Methods raise / return based on the constructor knobs:
        - ``raise_on_generate``: when set, ``generate()`` raises this
          exception (FAILURE / CANCELLED unit-test paths).
        - ``usage_input_tokens`` / ``usage_output_tokens``: drive the
          ``ChatMessage.token_usage`` shape ``_extract_total_tokens``
          reads.
        - ``track_calls``: when True, every ``generate()`` call appends
          its ``(messages, stop_sequences, response_format,
          tools_to_call_from, kwargs)`` payload to ``calls`` for
          assertion.
    """

    def __init__(
        self,
        *,
        usage_input_tokens: int = 30,
        usage_output_tokens: int = 70,
        raise_on_generate: BaseException | None = None,
        track_calls: bool = True,
        no_usage: bool = False,
        content: str = "hi",
        model_id: str | None = "test-model",
    ) -> None:
        # NOTE: do not call super().__init__() — the wrapper's
        # __init__ skips it (review-standards §1.2) and so should the
        # test fake; mirrors the production class's contract. Some
        # smolagents internals (logging) look at self.model_id, so we
        # set it directly without invoking the parent ABC __init__.
        self.model_id = model_id
        self._input_tokens = usage_input_tokens
        self._output_tokens = usage_output_tokens
        self._raise_on_generate = raise_on_generate
        self._track_calls = track_calls
        self._no_usage = no_usage
        self._content = content
        self.calls: list[dict[str, Any]] = []
        # Counters for pass-through assertions.
        self.flatten_calls = 0

    def generate(
        self,
        messages: list[Any],
        stop_sequences: list[str] | None = None,
        response_format: Any = None,
        tools_to_call_from: list[Any] | None = None,
        **kwargs: Any,
    ) -> Any:
        if self._track_calls:
            self.calls.append(
                {
                    "messages": messages,
                    "stop_sequences": stop_sequences,
                    "response_format": response_format,
                    "tools_to_call_from": tools_to_call_from,
                    "kwargs": dict(kwargs),
                }
            )
        if self._raise_on_generate is not None:
            raise self._raise_on_generate
        return _make_chat_message(
            content=self._content,
            input_tokens=self._input_tokens,
            output_tokens=self._output_tokens,
            no_usage=self._no_usage,
        )

    # `smolagents<1.5` agents call `model(messages, ...)`. The real
    # ABC's __call__ aliases generate; the fake mirrors this. The
    # wrapper has its own __call__ — this fake __call__ only fires
    # when an unwrapped fake is exercised directly.
    def __call__(self, messages: list[Any], **kwargs: Any) -> Any:
        return self.generate(messages, **kwargs)

    def flatten_messages_as_text(self, messages: list[Any]) -> str:
        """Pass-through helper for __getattr__ forwarding tests."""
        self.flatten_calls += 1
        return "\n".join(str(getattr(m, "content", m)) for m in messages)


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
    # The optional telemetry surface used by spendguard_step_callback.
    # We register the attribute as a plain MagicMock so the callback
    # can detect+call it; tests that exercise the fallback log path
    # explicitly del this attribute.
    client.emit_agent_step_telemetry = MagicMock(return_value=None)
    return client


__all__ = [
    "CHATMESSAGE_AVAILABLE",
    "FakeSmolModel",
    "SMOLAGENTS_AVAILABLE",
    "_make_chat_message",
    "make_client_mock",
]
