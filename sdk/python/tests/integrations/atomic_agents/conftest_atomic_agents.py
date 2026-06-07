# ruff: noqa: ANN001, ANN201, ANN401, S101, S106
"""Shared fixtures for D28 Atomic Agents adapter tests.

Provides ``FakeInstructor`` / ``FakeAsyncInstructor`` — subclasses of
the real ``instructor.Instructor`` / ``instructor.AsyncInstructor``
when the package is installed, plain base classes otherwise. Per
review-standards §7.3 unit tests MUST mirror the real
``chat.completions.create`` / ``create_with_completion`` interface
rather than use ``MagicMock(spec=Instructor)``; mock-spec silently
swallows signature changes upstream.

Also provides ``make_client_mock`` for mocking the SpendGuardClient
interface — mirrors ``conftest_autogen.py``'s helper exactly so the
two suites are visually identical.
"""

from __future__ import annotations

from collections.abc import Iterable
from types import SimpleNamespace
from typing import Any
from unittest.mock import AsyncMock, MagicMock

# Detect whether instructor is importable so the fake bases use the
# real ABC. Even when instructor is installed, ``Instructor`` /
# ``AsyncInstructor`` accept private kwargs we can't fake; the fakes
# below skip ``super().__init__()`` for that reason (mirrors the
# production class which never calls super().__init__).
try:  # pragma: no cover — branch chosen at import time
    from instructor import (  # type: ignore[import-not-found]
        AsyncInstructor as _RealAsyncInstructor,
    )
    from instructor import (  # type: ignore[import-not-found]
        Instructor as _RealInstructor,
    )
    from instructor import Mode as _RealMode  # type: ignore[import-not-found]

    INSTRUCTOR_AVAILABLE = True
    DEFAULT_FAKE_MODE: Any = _RealMode.TOOLS
except ImportError:  # pragma: no cover — branch chosen at import time
    _RealInstructor = None  # type: ignore[assignment, misc]
    _RealAsyncInstructor = None  # type: ignore[assignment, misc]
    _RealMode = None  # type: ignore[assignment, misc]
    INSTRUCTOR_AVAILABLE = False
    DEFAULT_FAKE_MODE = "TOOLS"


# Resolve the base class. Real ABC when available so the wrapper's
# ``isinstance(client, Instructor)`` factory dispatch passes; else
# plain object so the suite still runs without the extra installed.
if _RealInstructor is not None:  # pragma: no cover
    _FakeSyncBase: Any = _RealInstructor
else:
    _FakeSyncBase = object

if _RealAsyncInstructor is not None:  # pragma: no cover
    _FakeAsyncBase: Any = _RealAsyncInstructor
else:
    _FakeAsyncBase = object


class _FakeOpenAIClientChatCompletions:
    """``client.chat.completions.create`` stand-in.

    The production proxy reads ``inner.client.chat.completions.create``
    and wraps it. We expose a callable ``create`` that resolves a
    raw completion via the owner's ``_chat_completions._raw_only``
    helper — so the FakeInstructor's scripted-results / raise paths
    flow through.
    """

    def __init__(self, owner: "FakeInstructor") -> None:
        self._owner = owner

    def create(self, **kwargs: Any) -> Any:
        if self._owner._track_calls:
            self._owner.calls.append(("client.chat.completions.create", dict(kwargs)))
        return self._owner._chat_completions._raw_only(kwargs)


class _FakeOpenAIClientChat:
    def __init__(self, owner: "FakeInstructor") -> None:
        self.completions = _FakeOpenAIClientChatCompletions(owner)


class _FakeOpenAIClient:
    """Mimics ``openai.OpenAI`` for the proxy's raw-method resolution.

    Exposes ``self.chat.completions.create`` calling into the
    owner's ``_raw_only`` resolver so scripted-results /
    raise-on-create knobs still drive the test.
    """

    def __init__(self, owner: "FakeInstructor") -> None:
        self.chat = _FakeOpenAIClientChat(owner)


class _FakeAsyncOpenAIClientChatCompletions:
    """Async sibling — exposes an async ``create``."""

    def __init__(self, owner: "FakeAsyncInstructor") -> None:
        self._owner = owner

    async def create(self, **kwargs: Any) -> Any:
        if self._owner._track_calls:
            self._owner.calls.append(("client.chat.completions.create", dict(kwargs)))
        return self._owner._chat_completions._raw_only(kwargs)


class _FakeAsyncOpenAIClientChat:
    def __init__(self, owner: "FakeAsyncInstructor") -> None:
        self.completions = _FakeAsyncOpenAIClientChatCompletions(owner)


class _FakeAsyncOpenAIClient:
    def __init__(self, owner: "FakeAsyncInstructor") -> None:
        self.chat = _FakeAsyncOpenAIClientChat(owner)


def _build_chat_completion(
    *,
    prompt_tokens: int,
    completion_tokens: int,
    total_tokens: int | None = None,
    completion_id: str = "chatcmpl-fake-0",
    content: str = "fake assistant content",
    no_usage: bool = False,
) -> Any:
    """Build an OpenAI-shaped ``ChatCompletion``-like object.

    Uses ``SimpleNamespace`` so we don't pull in the openai SDK type
    surface; the wrapper only reads ``.id``, ``.usage.total_tokens``,
    ``.usage.prompt_tokens``, ``.usage.completion_tokens`` so a duck
    type is sufficient (and matches what Instructor actually returns
    when you ask for ``raw_completion``).
    """
    if no_usage:
        return SimpleNamespace(id=completion_id, usage=None, content=content)
    if total_tokens is None:
        total_tokens = prompt_tokens + completion_tokens
    usage = SimpleNamespace(
        prompt_tokens=prompt_tokens,
        completion_tokens=completion_tokens,
        total_tokens=total_tokens,
    )
    return SimpleNamespace(id=completion_id, usage=usage, content=content)


class _FakeChatCompletions:
    """Mimics ``Instructor.chat.completions``.

    Sync version. Mirrors the real ``.create`` / ``.create_with_completion``
    interface — review-standards §7.3 explicitly forbids
    ``MagicMock(spec=...)`` for this surface.

    Per DEVIATION-C the gate lives at ``Instructor.create_fn`` (the
    per-attempt callable), NOT here. ``create_with_completion`` drives
    ``self._owner.create_fn(...)`` per attempt to mirror Instructor's
    actual retry loop shape.
    """

    def __init__(
        self,
        *,
        parsed_factory: Any,
        raw_factory: Any,
        raise_on_create: BaseException | None,
        scripted_results: Iterable[Any] | None,
        track_calls: bool,
        owner: "FakeInstructor",
    ) -> None:
        self._parsed_factory = parsed_factory
        self._raw_factory = raw_factory
        self._raise_on_create = raise_on_create
        # When ``scripted_results`` is supplied, each call pops one
        # entry; supports per-attempt drive for validation-retry tests
        # ("call #1 raises ValidationError, call #2 succeeds").
        self._scripted_results = (
            list(scripted_results) if scripted_results is not None else None
        )
        self._track_calls = track_calls
        self._owner = owner

    def _resolve_result(
        self, method_name: str, kwargs: dict[str, Any]
    ) -> Any:
        if self._raise_on_create is not None:
            raise self._raise_on_create
        if self._scripted_results is not None:
            # Each entry: either an exception (raise it) or a tuple
            # (parsed, raw_completion); single value treated as raw.
            if not self._scripted_results:
                raise RuntimeError(
                    "FakeInstructor: scripted_results exhausted"
                )
            entry = self._scripted_results.pop(0)
            if isinstance(entry, BaseException):
                raise entry
            if isinstance(entry, tuple) and len(entry) == 2:
                parsed_v, raw_v = entry
            else:
                # Treat as raw completion; derive parsed from raw.
                raw_v = entry
                parsed_v = SimpleNamespace(_raw_response=raw_v)
        else:
            raw_v = self._raw_factory(kwargs)
            parsed_v = self._parsed_factory(kwargs, raw_v)
        if method_name == "create_with_completion":
            return (parsed_v, raw_v)
        # ``.create()`` returns parsed-only with raw on ``_raw_response``.
        # If the supplied parsed doesn't carry _raw_response, attach it.
        if not hasattr(parsed_v, "_raw_response") and raw_v is not None:
            try:
                parsed_v._raw_response = raw_v
            except AttributeError:
                parsed_v = SimpleNamespace(
                    parsed=parsed_v, _raw_response=raw_v
                )
        return parsed_v

    def create(self, **kwargs: Any) -> Any:
        """Drive ``create_fn`` per attempt — mirrors Instructor's retry shape.

        In real Instructor, ``Instructor.create_with_completion`` calls
        ``self.create_fn(*args, **kwargs)`` per retry attempt. We
        mirror that contract so wrapping ``create_fn`` in the proxy
        gates each attempt naturally.
        """
        if self._track_calls:
            self._owner.calls.append(("create", dict(kwargs)))
        # Drive create_fn per attempt (mirrors instructor.core.retry).
        return self._owner.create_fn(**kwargs)

    def create_with_completion(self, **kwargs: Any) -> Any:
        """Drive ``create_fn`` then wrap as ``(parsed, raw)``."""
        if self._track_calls:
            self._owner.calls.append(("create_with_completion", dict(kwargs)))
        raw_completion = self._owner.create_fn(**kwargs)
        # Build the parsed object from the raw + the response_model.
        parsed = self._parsed_factory(kwargs, raw_completion)
        return (parsed, raw_completion)

    # Expose the raw-resolver so the FakeInstructor.create_fn can call
    # it without us re-implementing the scripted-results logic.
    def _raw_only(self, kwargs: dict[str, Any]) -> Any:
        if self._raise_on_create is not None:
            raise self._raise_on_create
        if self._scripted_results is not None:
            if not self._scripted_results:
                raise RuntimeError(
                    "FakeInstructor: scripted_results exhausted"
                )
            entry = self._scripted_results.pop(0)
            if isinstance(entry, BaseException):
                raise entry
            if isinstance(entry, tuple) and len(entry) == 2:
                _parsed_v, raw_v = entry
                return raw_v
            return entry
        return self._raw_factory(kwargs)


class _FakeAsyncChatCompletions:
    """Async sibling of ``_FakeChatCompletions``."""

    def __init__(
        self,
        *,
        parsed_factory: Any,
        raw_factory: Any,
        raise_on_create: BaseException | None,
        scripted_results: Iterable[Any] | None,
        track_calls: bool,
        owner: "FakeAsyncInstructor",
    ) -> None:
        self._parsed_factory = parsed_factory
        self._raw_factory = raw_factory
        self._raise_on_create = raise_on_create
        self._scripted_results = (
            list(scripted_results) if scripted_results is not None else None
        )
        self._track_calls = track_calls
        self._owner = owner

    def _resolve_result(
        self, method_name: str, kwargs: dict[str, Any]
    ) -> Any:
        if self._raise_on_create is not None:
            raise self._raise_on_create
        if self._scripted_results is not None:
            if not self._scripted_results:
                raise RuntimeError(
                    "FakeAsyncInstructor: scripted_results exhausted"
                )
            entry = self._scripted_results.pop(0)
            if isinstance(entry, BaseException):
                raise entry
            if isinstance(entry, tuple) and len(entry) == 2:
                parsed_v, raw_v = entry
            else:
                raw_v = entry
                parsed_v = SimpleNamespace(_raw_response=raw_v)
        else:
            raw_v = self._raw_factory(kwargs)
            parsed_v = self._parsed_factory(kwargs, raw_v)
        if method_name == "create_with_completion":
            return (parsed_v, raw_v)
        if not hasattr(parsed_v, "_raw_response") and raw_v is not None:
            try:
                parsed_v._raw_response = raw_v
            except AttributeError:
                parsed_v = SimpleNamespace(
                    parsed=parsed_v, _raw_response=raw_v
                )
        return parsed_v

    async def create(self, **kwargs: Any) -> Any:
        """Drive ``create_fn`` per attempt — async mirror of Instructor."""
        if self._track_calls:
            self._owner.calls.append(("create", dict(kwargs)))
        return await self._owner.create_fn(**kwargs)

    async def create_with_completion(self, **kwargs: Any) -> Any:
        if self._track_calls:
            self._owner.calls.append(("create_with_completion", dict(kwargs)))
        raw_completion = await self._owner.create_fn(**kwargs)
        parsed = self._parsed_factory(kwargs, raw_completion)
        return (parsed, raw_completion)

    def _raw_only(self, kwargs: dict[str, Any]) -> Any:
        if self._raise_on_create is not None:
            raise self._raise_on_create
        if self._scripted_results is not None:
            if not self._scripted_results:
                raise RuntimeError(
                    "FakeAsyncInstructor: scripted_results exhausted"
                )
            entry = self._scripted_results.pop(0)
            if isinstance(entry, BaseException):
                raise entry
            if isinstance(entry, tuple) and len(entry) == 2:
                _parsed_v, raw_v = entry
                return raw_v
            return entry
        return self._raw_factory(kwargs)


class _FakeChatNamespace:
    """Mimics ``Instructor.chat`` (holds ``.completions``)."""

    def __init__(
        self, completions: _FakeChatCompletions | _FakeAsyncChatCompletions
    ) -> None:
        self.completions = completions


class FakeInstructor(_FakeSyncBase):
    """Hand-rolled sync ``Instructor`` for unit tests.

    Per review-standards §7.3 this subclasses the real ABC (when
    importable) so upstream signature changes break compile-time
    instead of silently. We skip ``super().__init__()`` for the same
    reason ``SpendGuardInstructorProxy`` does — ``Instructor.__init__``
    accepts private kwargs we don't want to model in tests.

    The real ``instructor.Instructor.chat`` is a ``property``, so we
    redefine ``chat`` as a class-level property on the fake that
    returns the per-instance ``_chat_namespace`` (rather than
    assigning ``self.chat = ...`` which would AttributeError).

    Methods:
        - ``chat.completions.create(**kwargs)`` returns a parsed
          model whose ``_raw_response`` is the configured raw
          ``ChatCompletion``-shaped object.
        - ``chat.completions.create_with_completion(**kwargs)``
          returns ``(parsed, raw_completion)``.

    Knobs:
        - ``usage_prompt_tokens`` / ``usage_completion_tokens`` /
          ``usage_total_tokens``: shape the ``raw_completion.usage``.
        - ``no_usage``: emit a raw completion with ``usage=None``
          (POC fallback test).
        - ``raise_on_create``: always raise (FAILURE / CANCELLED
          paths).
        - ``scripted_results``: per-call deterministic returns —
          each call pops one. Mix exceptions and tuples for the
          validation-retry test.
        - ``track_calls``: when True, every call appends
          ``(method_name, kwargs)`` to ``self.calls`` for assertion.
    """

    # Carry forward Instructor's introspection attributes a wrapper
    # might delegate to via __getattr__. Tests assert these reach
    # through unchanged.
    default_model: str = "gpt-4o-mini"

    def __init__(  # noqa: PLR0913
        self,
        *,
        usage_prompt_tokens: int = 11,
        usage_completion_tokens: int = 22,
        usage_total_tokens: int | None = None,
        completion_id: str = "chatcmpl-fake-0",
        content: str = "fake assistant content",
        no_usage: bool = False,
        raise_on_create: BaseException | None = None,
        scripted_results: Iterable[Any] | None = None,
        track_calls: bool = True,
        parsed_factory: Any = None,
        mode: Any = None,
        create_kwargs: dict[str, Any] | None = None,
    ) -> None:
        # NOTE: do not call super().__init__() — the wrapper's
        # __init__ skips it (review-standards §1.2) and so should the
        # test fake; mirrors the production class's contract.
        # CRITICAL: set ``client`` FIRST. The production
        # SpendGuardInstructorProxy resolves the raw method via
        # ``inner.client.chat.completions.create`` so we build a
        # minimal stand-in that exposes the same call surface.
        self.client = _FakeOpenAIClient(self)
        self.hooks = None
        self._usage_prompt_tokens = usage_prompt_tokens
        self._usage_completion_tokens = usage_completion_tokens
        self._usage_total_tokens = usage_total_tokens
        self._completion_id = completion_id
        self._content = content
        self._no_usage = no_usage
        self.calls: list[tuple[str, dict[str, Any]]] = []
        # mode / create_kwargs are exposed via __getattr__ delegation
        # paths; tests assert proxy.mode / proxy.create_kwargs reach
        # the inner fake.
        self.mode = mode if mode is not None else DEFAULT_FAKE_MODE
        self.create_kwargs = create_kwargs if create_kwargs is not None else {}

        def default_raw(_kwargs: dict[str, Any]) -> Any:
            return _build_chat_completion(
                prompt_tokens=usage_prompt_tokens,
                completion_tokens=usage_completion_tokens,
                total_tokens=usage_total_tokens,
                completion_id=completion_id,
                content=content,
                no_usage=no_usage,
            )

        def default_parsed(kwargs: dict[str, Any], raw: Any) -> Any:
            response_model = kwargs.get("response_model")
            if response_model is None:
                # No schema → parsed is just the raw.
                return SimpleNamespace(_raw_response=raw)
            # Instantiate the response_model with empty defaults.
            try:
                inst = response_model()
            except Exception:  # noqa: BLE001
                inst = SimpleNamespace()
            # Attach raw via the private attr (matches Instructor 1.5+).
            try:
                inst._raw_response = raw
            except AttributeError:
                inst = SimpleNamespace(parsed=inst, _raw_response=raw)
            return inst

        self._chat_completions = _FakeChatCompletions(
            parsed_factory=parsed_factory or default_parsed,
            raw_factory=default_raw,
            raise_on_create=raise_on_create,
            scripted_results=scripted_results,
            track_calls=track_calls,
            owner=self,
        )
        # ``chat`` is a property on the real Instructor ABC; store the
        # per-instance namespace in a private attr that the property
        # below returns. Assigning ``self.chat = ...`` would
        # AttributeError because the real class defines ``chat`` as a
        # property without a setter.
        self._fake_chat_namespace = _FakeChatNamespace(self._chat_completions)
        # ``create_fn`` is the per-attempt callable Instructor's retry
        # loop drives. Real Instructor sets this to
        # ``openai_client.chat.completions.create``; we point it at our
        # raw-only resolver so wrapping ``create_fn`` in the proxy
        # gates each attempt at the same point as production.
        self._track_calls = track_calls
        self.create_fn = self._default_create_fn

    @property
    def chat(self) -> _FakeChatNamespace:  # type: ignore[override]
        """Override Instructor's ``chat`` property with our test namespace."""
        return self._fake_chat_namespace

    def _default_create_fn(self, **kwargs: Any) -> Any:
        """Default sync ``create_fn`` — returns a raw ``ChatCompletion``."""
        if self._track_calls:
            self.calls.append(("create_fn", dict(kwargs)))
        return self._chat_completions._raw_only(kwargs)


class FakeAsyncInstructor(_FakeAsyncBase):
    """Hand-rolled async ``AsyncInstructor`` for unit tests.

    Mirrors ``FakeInstructor`` field-for-field; only
    ``.chat.completions.create*`` differ in being async.

    ``chat`` is a property override (real ``AsyncInstructor.chat`` is
    a no-setter property) — same compromise as ``FakeInstructor``.
    """

    default_model: str = "gpt-4o-mini"

    def __init__(  # noqa: PLR0913
        self,
        *,
        usage_prompt_tokens: int = 11,
        usage_completion_tokens: int = 22,
        usage_total_tokens: int | None = None,
        completion_id: str = "chatcmpl-fake-0",
        content: str = "fake assistant content",
        no_usage: bool = False,
        raise_on_create: BaseException | None = None,
        scripted_results: Iterable[Any] | None = None,
        track_calls: bool = True,
        parsed_factory: Any = None,
        mode: Any = None,
        create_kwargs: dict[str, Any] | None = None,
    ) -> None:
        # Set client/hooks FIRST. AsyncOpenAI-shaped client stand-in so
        # the production proxy can locate the raw async create method.
        self.client = _FakeAsyncOpenAIClient(self)
        self.hooks = None
        self._usage_prompt_tokens = usage_prompt_tokens
        self._usage_completion_tokens = usage_completion_tokens
        self._usage_total_tokens = usage_total_tokens
        self._completion_id = completion_id
        self._content = content
        self._no_usage = no_usage
        self.calls: list[tuple[str, dict[str, Any]]] = []
        self.mode = mode if mode is not None else DEFAULT_FAKE_MODE
        self.create_kwargs = create_kwargs if create_kwargs is not None else {}

        def default_raw(_kwargs: dict[str, Any]) -> Any:
            return _build_chat_completion(
                prompt_tokens=usage_prompt_tokens,
                completion_tokens=usage_completion_tokens,
                total_tokens=usage_total_tokens,
                completion_id=completion_id,
                content=content,
                no_usage=no_usage,
            )

        def default_parsed(kwargs: dict[str, Any], raw: Any) -> Any:
            response_model = kwargs.get("response_model")
            if response_model is None:
                return SimpleNamespace(_raw_response=raw)
            try:
                inst = response_model()
            except Exception:  # noqa: BLE001
                inst = SimpleNamespace()
            try:
                inst._raw_response = raw
            except AttributeError:
                inst = SimpleNamespace(parsed=inst, _raw_response=raw)
            return inst

        self._chat_completions = _FakeAsyncChatCompletions(
            parsed_factory=parsed_factory or default_parsed,
            raw_factory=default_raw,
            raise_on_create=raise_on_create,
            scripted_results=scripted_results,
            track_calls=track_calls,
            owner=self,
        )
        self._fake_chat_namespace = _FakeChatNamespace(self._chat_completions)
        # Async ``create_fn`` — Instructor's retry_async loop awaits this.
        self._track_calls = track_calls
        self.create_fn = self._default_create_fn

    @property
    def chat(self) -> _FakeChatNamespace:  # type: ignore[override]
        """Override AsyncInstructor's ``chat`` property with our namespace."""
        return self._fake_chat_namespace

    async def _default_create_fn(self, **kwargs: Any) -> Any:
        """Default async ``create_fn`` — returns a raw ``ChatCompletion``."""
        if self._track_calls:
            self.calls.append(("create_fn", dict(kwargs)))
        return self._chat_completions._raw_only(kwargs)


def make_client_mock(
    *,
    tenant_id: str = "tenant-1",
    session_id: str = "session-1",
    decision_id: str = "dec-1",
    reservation_ids: tuple[str, ...] = ("res-1",),
    decision: str = "CONTINUE",
    request_decision_side_effect: Any = None,
) -> MagicMock:
    """Build an ``AsyncMock`` shaped like a connected SpendGuardClient.

    Matches the autogen / agno conftest exactly so the two suites are
    visually identical.
    """
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
    "FakeAsyncInstructor",
    "FakeInstructor",
    "INSTRUCTOR_AVAILABLE",
    "_build_chat_completion",
    "make_client_mock",
]
