"""Install state machine + recursion guard for the LiteLLM SDK shim.

Module-level singletons:

* ``_INSTALL_STATE`` — the only mutable global; ``None`` when the shim
  is not installed, otherwise the active ``_ShimState`` snapshot.
* ``_IN_FLIGHT``     — per-asyncio-task ``ContextVar[bool]`` that the
  patch wrappers consult to short-circuit re-entry (avoid shim calling
  shim).

The ``ContextVar`` choice is deliberate: a ``threading.local`` would
leak across ``asyncio.gather`` siblings on the same OS thread, and a
plain module-level bool would corrupt across concurrent tasks. The
contextvars module handles asyncio task copy-on-enter correctly so
each ``await`` chain sees its own re-entry state.
"""

from __future__ import annotations

import contextvars
import hashlib
from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Any

from ._options import SpendGuardShimOptions

if TYPE_CHECKING:
    from ._core import _DirectCore


# Re-entry guard. Set inside every patched wrapper; checked at the
# wrapper's top so any LiteLLM-internal call that routes back through
# a patched entry point (fallback chain, Router internal dispatch)
# short-circuits to the saved original.
_IN_FLIGHT: contextvars.ContextVar[bool] = contextvars.ContextVar(
    "spendguard_shim_in_flight",
    default=False,
)


@dataclass(slots=True)
class _ShimState:
    """Snapshot of an active ``install_shim()`` call.

    Fields:
      * ``options``           — the ``SpendGuardShimOptions`` passed in.
      * ``config_signature``  — stable hash of the options dict; used
        for idempotent re-install detection.
      * ``originals``         — list of ``(owner, attr, original)``
        triples captured at patch time. ``uninstall_shim`` walks this
        in REVERSE order so subclass restores precede ``Router``
        restores (matches Spec §5).
      * ``patched_subclasses``— strong references to ``Router``
        subclasses captured at install time so a weakref'd subclass
        from ``Router.__subclasses__()`` cannot be GC'd between install
        and uninstall.
    """

    options: SpendGuardShimOptions
    config_signature: str
    originals: list[tuple[Any, str, Any]] = field(default_factory=list)
    patched_subclasses: list[type] = field(default_factory=list)
    # Populated by ``install_shim`` BEFORE patches run. The patches
    # access it via ``state.core``. Holding it on the state object
    # (not module-level) means a stale ``_INSTALL_STATE`` cleared
    # by ``uninstall_shim`` releases the core too.
    core: _DirectCore | None = None


# The ONLY module-level mutable global. ``is_installed()`` is the
# canonical truthiness check; ``install_shim()`` is the only writer.
_INSTALL_STATE: _ShimState | None = None


def _current_state() -> _ShimState | None:
    """Test-friendly accessor; production code reads ``_INSTALL_STATE``
    directly."""
    return _INSTALL_STATE


def _set_state(state: _ShimState | None) -> None:
    """Internal mutator for the install / uninstall flow. Tests should
    NEVER call this — they should drive ``install_shim`` /
    ``uninstall_shim`` instead so the patch lifecycle stays
    observable."""
    global _INSTALL_STATE
    _INSTALL_STATE = state


def _compute_config_signature(options: SpendGuardShimOptions) -> str:
    """Deterministic signature for idempotent-install detection.

    Hashes the identity of the client object plus the scalar fields.
    Two ``install_shim()`` calls with the SAME options instance (or
    options carrying the same field values + same client) produce the
    same signature → idempotent no-op. Different signature → caller
    must ``uninstall_shim()`` first.

    ``id(options.client)`` is intentional: two different client
    instances with bit-identical config are still treated as a config
    change because their channel state is independent.
    """
    parts: list[str] = [
        f"client:{id(options.client)}",
        f"tenant:{options.tenant_id}",
        f"budget:{options.budget_id or ''}",
        f"fail_open:{int(options.fail_open)}",
    ]
    canonical = "\x1f".join(parts).encode("utf-8")
    return hashlib.blake2b(canonical, digest_size=16).hexdigest()


def is_installed() -> bool:
    """Public predicate. Side-effect free, safe to call at any time."""
    return _INSTALL_STATE is not None


__all__ = [
    "_IN_FLIGHT",
    "_INSTALL_STATE",
    "_ShimState",
    "_compute_config_signature",
    "_current_state",
    "_set_state",
    "is_installed",
]
