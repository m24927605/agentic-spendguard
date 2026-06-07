# ruff: noqa: ANN401  # LiteLLM Router method kwargs are intentionally ``Any``
"""Patch ``litellm.Router.acompletion`` + walk live subclasses.

LiteLLM's ``Router`` is the framework-level dispatcher used by every
agent stack (CrewAI, DSPy, etc.) under the hood when the operator
supplies a ``model_list``. Patching ``litellm.acompletion`` alone is
not enough — the Router has its own ``acompletion`` method that calls
into the provider client directly.

Two cases we need to cover:

  * Subclasses that *inherit* ``Router.acompletion`` via MRO — they
    pick up the patched parent automatically at attribute lookup time;
    no per-subclass work needed.
  * Subclasses that *override* ``acompletion`` in their own ``__dict__``
    — we walk ``Router.__subclasses__()`` at install time and patch
    each override individually. Each patched subclass goes into
    ``state.patched_subclasses`` so we hold a strong ref (weakly-held
    entries from ``__subclasses__()`` can be GC'd between install and
    uninstall otherwise).

Subclasses created *after* ``install_shim()`` returns inherit the
patched parent via normal MRO. We don't intercept ``__init_subclass__``
because (a) that would require a metaclass change which is brittle and
(b) the spec explicitly documents this as the contract.
"""

from __future__ import annotations

from typing import Any

from .._state import _IN_FLIGHT, _ShimState
from ._acompletion import _require_core


def _build_router_acompletion_wrapper(
    state: _ShimState,
    original_unbound: Any,
) -> Any:
    """Return an ``async def`` method-style wrapper for the patched
    ``acompletion``.

    ``original_unbound`` is the class-level function (NOT a bound
    method); the wrapper takes ``self`` explicitly and forwards it via
    the inner ``_bound_original`` closure so the core sees a kwargs-only
    callable.
    """

    async def _router_acompletion(self: Any, **kwargs: Any) -> Any:
        if _IN_FLIGHT.get():
            return await original_unbound(self, **kwargs)
        token = _IN_FLIGHT.set(True)
        try:
            async def _bound_original(**kw: Any) -> Any:
                return await original_unbound(self, **kw)

            return await _require_core(state)(
                _original_acompletion=_bound_original,
                **kwargs,
            )
        finally:
            _IN_FLIGHT.reset(token)

    return _router_acompletion


def _patch_router(state: _ShimState) -> None:
    """Patch ``litellm.Router.acompletion`` + every override on live
    subclasses. ``Router.completion`` (sync) is intentionally NOT
    patched here in slice 4 scope — it routes through
    ``litellm.completion`` which the sync-patch slice already covers.
    """
    import litellm

    Router = litellm.Router  # noqa: N806 — proto class alias

    # Patch the parent. Subclasses that don't override inherit this via
    # MRO on next attribute lookup.
    original_acompletion = Router.acompletion
    state.originals.append((Router, "acompletion", original_acompletion))
    Router.acompletion = _build_router_acompletion_wrapper(  # type: ignore[assignment]
        state, original_acompletion,
    )

    # Walk live subclasses. Only re-patch those that overrode the
    # parent method (i.e. have ``acompletion`` in their own __dict__).
    for sub in Router.__subclasses__():
        if "acompletion" not in sub.__dict__:
            # Inherits via MRO → already covered by the parent patch.
            continue
        sub_original = sub.__dict__["acompletion"]
        state.originals.append((sub, "acompletion", sub_original))
        sub.acompletion = _build_router_acompletion_wrapper(  # type: ignore[assignment]
            state, sub_original,
        )
        state.patched_subclasses.append(sub)


__all__ = ["_patch_router"]
