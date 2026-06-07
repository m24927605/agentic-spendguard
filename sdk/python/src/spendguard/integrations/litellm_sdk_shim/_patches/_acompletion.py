# ruff: noqa: ANN401  # LiteLLM kwargs are intentionally ``Any``
"""Patch ``litellm.acompletion`` + ``litellm.atext_completion``.

Both async entry points share the same wrapper shape because they have
identical signatures from the caller's perspective (``**kwargs``-only)
and both return objects with the ``usage.completion_tokens`` field that
``_DirectCore`` reads on the commit path. ``atext_completion`` uses
``prompt=`` instead of ``messages=`` — the default estimator handles
both because it falls back to chars/4 on a missing ``messages`` key.
"""

from __future__ import annotations

from typing import Any

from .._state import _IN_FLIGHT, _ShimState


def _require_core(state: _ShimState) -> Any:
    """Defensive accessor: ``install_shim`` MUST populate ``state.core``
    before any patch helper runs. The raise here surfaces an install
    ordering bug loudly instead of a confusing ``NoneType has no
    __call__`` at request time."""
    if state.core is None:
        raise RuntimeError(
            "shim state.core not initialized; install_shim must "
            "construct _DirectCore before invoking patches.",
        )
    return state.core


def _patch_acompletion(state: _ShimState) -> None:
    """Replace ``litellm.acompletion`` with a SpendGuard-gated wrapper."""
    import litellm

    original = litellm.acompletion
    state.originals.append((litellm, "acompletion", original))

    async def _wrapper(**kwargs: Any) -> Any:
        # Re-entry guard FIRST so a LiteLLM-internal fallback chain that
        # calls back into ``litellm.acompletion`` hits the saved original
        # directly. No double-reserve.
        if _IN_FLIGHT.get():
            return await original(**kwargs)
        token = _IN_FLIGHT.set(True)
        try:
            return await _require_core(state)(
                _original_acompletion=original,
                **kwargs,
            )
        finally:
            _IN_FLIGHT.reset(token)

    litellm.acompletion = _wrapper  # type: ignore[assignment]


def _patch_atext_completion(state: _ShimState) -> None:
    """Replace ``litellm.atext_completion`` with a SpendGuard-gated wrapper."""
    import litellm

    original = litellm.atext_completion
    state.originals.append((litellm, "atext_completion", original))

    async def _wrapper(**kwargs: Any) -> Any:
        if _IN_FLIGHT.get():
            return await original(**kwargs)
        token = _IN_FLIGHT.set(True)
        try:
            return await _require_core(state)(
                _original_acompletion=original,
                **kwargs,
            )
        finally:
            _IN_FLIGHT.reset(token)

    litellm.atext_completion = _wrapper  # type: ignore[assignment]


__all__ = ["_patch_acompletion", "_patch_atext_completion"]
