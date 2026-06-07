"""LiteLLM SDK shim-specific exceptions.

Both inherit ``SpendGuardConfigError`` so operators with broad
``except SpendGuardError`` handlers (e.g. wrapping ``install_shim`` in
a startup probe) catch them uniformly.
"""

from __future__ import annotations

from ...errors import SpendGuardConfigError


class SpendGuardShimAlreadyInstalled(SpendGuardConfigError):
    """Raised when ``install_shim()`` is called while the shim is
    already active with a *different* config signature.

    Same-config re-install is treated as an idempotent no-op (no
    exception). The error message names the conflicting field where
    practical so operators can diagnose drift.
    """


class SpendGuardShimSyncInAsyncContext(SpendGuardConfigError):
    """Raised when ``litellm.completion()`` (sync) is called from
    inside a running event loop.

    The shim refuses to bridge via ``asyncio.run`` because that would
    deadlock the running loop. The fix is for the caller to use
    ``await litellm.acompletion(...)`` instead — the error message
    spells that out.
    """


__all__ = [
    "SpendGuardShimAlreadyInstalled",
    "SpendGuardShimSyncInAsyncContext",
]
