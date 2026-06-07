# ruff: noqa: ANN401  # LiteLLM kwargs are intentionally ``Any``
"""Patch sync ``litellm.completion`` + ``litellm.text_completion``.

The sync entry points are awkward: SpendGuard's client + the reserve
RPC are pure asyncio, but ``litellm.completion`` is a sync def that
callers ``return`` from outside any loop. We support two scenarios:

  * Sync call from sync context (no running loop) → bridge via
    ``asyncio.run`` to drive the core's reserve / commit asynchronously.
  * Sync call from inside a running loop (e.g. ``pytest.mark.asyncio``)
    → REFUSE with ``SpendGuardShimSyncInAsyncContext``. The alternative
    is ``asyncio.run_until_complete`` on a manually-managed loop, but
    that would (a) deadlock with the running outer loop and (b) bypass
    contextvars. The right answer is for the caller to ``await
    litellm.acompletion`` instead — the error message says so.

Single-source the bridging helper so ``completion`` and
``text_completion`` share guard + bridge logic.
"""

from __future__ import annotations

import asyncio
from typing import Any

from .._state import _IN_FLIGHT, _ShimState
from ..errors import SpendGuardShimSyncInAsyncContext
from ._acompletion import _require_core


def _refuse_if_loop_running(name: str) -> None:
    """Raise ``SpendGuardShimSyncInAsyncContext`` when a loop is running.

    Probes ``asyncio.get_running_loop()``; the raised ``RuntimeError`` is
    the canonical "no loop" signal — we catch that and return cleanly.
    Any *other* exception (extraordinarily rare) propagates so the
    caller sees the genuine error rather than a confusing
    SpendGuard-shaped wrapper.
    """
    try:
        asyncio.get_running_loop()
    except RuntimeError:
        return  # no loop → safe to bridge sync→async
    raise SpendGuardShimSyncInAsyncContext(
        f"litellm.{name}() called from inside a running event loop. "
        "asyncio.run() bridging would deadlock. Use "
        "`await litellm.acompletion(...)` instead.",
    )


async def _async_dispatch(
    state: _ShimState,
    original: Any,
    kwargs: dict[str, Any],
) -> Any:
    """Run the core inside a fresh task scope owned by ``asyncio.run``.

    Sets the recursion guard inside the dispatched coroutine, not in
    the sync wrapper, so the ContextVar lives entirely on the inner
    loop's task and a sibling tester's ``acompletion`` call across
    threads cannot see our token.
    """
    token = _IN_FLIGHT.set(True)
    try:
        return await _require_core(state)(
            _original_acompletion=original,
            **kwargs,
        )
    finally:
        _IN_FLIGHT.reset(token)


def _patch_completion(state: _ShimState) -> None:
    """Replace ``litellm.completion`` with a SpendGuard-gated wrapper."""
    import litellm

    original = litellm.completion
    state.originals.append((litellm, "completion", original))

    # The original ``litellm.completion`` is sync. Build an
    # async-equivalent bridge target so ``_DirectCore`` (which awaits
    # the original) can drive it from inside ``asyncio.run``.
    async def _async_original(**kw: Any) -> Any:
        return original(**kw)

    def _wrapper(**kwargs: Any) -> Any:
        _refuse_if_loop_running("completion")
        if _IN_FLIGHT.get():
            # Inside a re-entry token already (some inner wrapper set
            # it); call the sync original directly without re-bridging.
            return original(**kwargs)
        return asyncio.run(_async_dispatch(state, _async_original, kwargs))

    litellm.completion = _wrapper  # type: ignore[assignment]


def _patch_text_completion(state: _ShimState) -> None:
    """Replace ``litellm.text_completion`` with a SpendGuard-gated wrapper."""
    import litellm

    original = litellm.text_completion
    state.originals.append((litellm, "text_completion", original))

    async def _async_original(**kw: Any) -> Any:
        return original(**kw)

    def _wrapper(**kwargs: Any) -> Any:
        _refuse_if_loop_running("text_completion")
        if _IN_FLIGHT.get():
            return original(**kwargs)
        return asyncio.run(_async_dispatch(state, _async_original, kwargs))

    litellm.text_completion = _wrapper  # type: ignore[assignment]


__all__ = ["_patch_completion", "_patch_text_completion"]
