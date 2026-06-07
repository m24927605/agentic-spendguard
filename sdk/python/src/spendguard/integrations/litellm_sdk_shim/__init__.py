# ruff: noqa: ANN401  # LiteLLM SDK entry points use kwargs-only ``Any``
"""LiteLLM SDK monkey-patch shim.

Closes the gap left open by LiteLLM Issue #8842: ``async_pre_call_hook``
only fires on the proxy path. Direct ``litellm.acompletion()`` callers
(and every transitive caller — CrewAI, DSPy, SmolAgents, Strands, BeeAI,
AutoGen, Atomic Agents) have no pre-call gate. This shim monkey-patches
the SDK entry points so SpendGuard reserves BEFORE the provider HTTP
request leaves the process, regardless of how the call was issued.

Public API:

    from spendguard import SpendGuardClient
    from spendguard.integrations.litellm_sdk_shim import (
        SpendGuardShimOptions, install_shim, uninstall_shim, is_installed,
    )

    client = SpendGuardClient(socket_path="/run/spendguard.sock",
                              tenant_id="tenant-1")
    await client.connect(); await client.handshake()

    install_shim(SpendGuardShimOptions(
        client=client,
        tenant_id="tenant-1",
        budget_id="b1",
        fail_open=False,
    ))
    # All subsequent ``litellm.acompletion(...)`` / ``litellm.Router(...)``
    # / ``litellm.completion(...)`` calls are SpendGuard-gated.
    try:
        await crew.kickoff_async()  # transitive coverage, no CrewAI changes
    finally:
        uninstall_shim()

Lifecycle:

    * ``install_shim`` is idempotent for the same options signature —
      calling twice returns cleanly. Different signature raises
      ``SpendGuardShimAlreadyInstalled``.
    * ``uninstall_shim`` walks the captured originals in REVERSE so
      subclass restores precede ``Router`` restores.
    * Recursion guard via ``contextvars.ContextVar`` short-circuits any
      LiteLLM-internal fallback / Router re-entry that comes back
      through a patched entry point — single reserve per logical call.
"""

from __future__ import annotations

import logging

from ._options import SpendGuardShimOptions
from ._state import (
    _compute_config_signature,
    _current_state,
    _set_state,
    _ShimState,
    is_installed,
)
from .errors import (
    SpendGuardShimAlreadyInstalled,
    SpendGuardShimSyncInAsyncContext,
)

log = logging.getLogger("spendguard.integrations.litellm_sdk_shim")


def install_shim(options: SpendGuardShimOptions) -> None:
    """Install the shim. Idempotent for identical options.

    Calling twice with a *different* options object (different client,
    different budget_id, different fail_open) raises
    ``SpendGuardShimAlreadyInstalled``. Operators must
    ``uninstall_shim()`` first.

    Patches in this order (the SAME order the originals are stacked,
    which the reverse-uninstall relies on):

      1. ``litellm.acompletion``       (slice 2)
      2. ``litellm.atext_completion``  (slice 2)
      3. ``litellm.completion``        (slice 3)
      4. ``litellm.text_completion``   (slice 3)
      5. ``litellm.Router.acompletion`` + subclass walk (slice 4)

    Operator can subset via the ``options`` if needed; the current
    slice ships all-or-nothing (all surfaces patched). Future slice
    can expose ``patch_router=False`` / ``patch_sync=False``
    granularity per the design doc.
    """
    if not isinstance(options, SpendGuardShimOptions):
        raise TypeError(
            "install_shim requires a SpendGuardShimOptions instance; "
            f"got {type(options).__name__}.",
        )

    new_sig = _compute_config_signature(options)
    existing = _current_state()
    if existing is not None:
        if existing.config_signature == new_sig:
            # Idempotent no-op — same config, already active.
            log.debug(
                "spendguard_litellm_shim: install_shim no-op (same "
                "config_signature already installed).",
            )
            return
        raise SpendGuardShimAlreadyInstalled(
            "spendguard_litellm_shim is already installed with a "
            "different config_signature. Call uninstall_shim() first "
            "before re-installing with new options.",
        )

    # Build the state object BEFORE touching litellm so a construction
    # error (bad options, missing proto) leaves the global state clean.
    state = _ShimState(
        options=options,
        config_signature=new_sig,
    )

    # Construct the core. This is the heavy import path (proto +
    # estimators); doing it inside ``install_shim`` keeps module import
    # cheap and surfaces any proto / tokenizer dep issue at install
    # time, not at first request.
    from ._core import _DirectCore

    state.core = _DirectCore(options)

    # Apply patches. Each helper validates the litellm attribute exists,
    # captures the original into ``state.originals``, then assigns the
    # wrapper. Any failure here leaves a partially-patched litellm —
    # we cope by triggering the uninstall walk on the partial state
    # before re-raising, so the user can fix + retry.
    from ._patches._acompletion import (
        _patch_acompletion,
        _patch_atext_completion,
    )
    from ._patches._completion import _patch_completion, _patch_text_completion
    from ._patches._router import _patch_router

    try:
        _patch_acompletion(state)
        _patch_atext_completion(state)
        _patch_completion(state)
        _patch_text_completion(state)
        _patch_router(state)
    except Exception:
        # Roll back any partial patching so the litellm module returns
        # to its pre-install state, then re-raise so the caller sees
        # the original error.
        _restore_originals(state)
        raise

    _set_state(state)
    log.info(
        "spendguard_litellm_shim installed: %d entry points patched "
        "(%d Router subclasses)",
        len(state.originals),
        len(state.patched_subclasses),
    )


def uninstall_shim() -> None:
    """Restore every patched entry point. No-op when not installed.

    Restores in REVERSE order of patching so subclass overrides land
    back before the ``Router`` parent — otherwise a subclass that
    overrode ``acompletion`` would briefly observe the unpatched
    parent before its own restore.
    """
    state = _current_state()
    if state is None:
        return
    _restore_originals(state)
    # Drop the core reference so the SpendGuardClient is no longer
    # held by this module (operator may want to close the client).
    state.core = None
    _set_state(None)
    log.info("spendguard_litellm_shim uninstalled")


def _restore_originals(state: _ShimState) -> None:
    """Walk ``state.originals`` in reverse + reset each attribute.

    Each entry is ``(owner, attr, original)``. ``setattr`` is the
    canonical restore (works for class attributes + module-level
    functions). Errors here are *not* swallowed — a failed restore
    leaves the litellm module in a bad state and operators need to
    see that loudly.
    """
    for owner, attr, original in reversed(state.originals):
        setattr(owner, attr, original)
    state.originals.clear()
    state.patched_subclasses.clear()


__all__ = [
    "SpendGuardShimAlreadyInstalled",
    "SpendGuardShimOptions",
    "SpendGuardShimSyncInAsyncContext",
    "install_shim",
    "is_installed",
    "uninstall_shim",
]
