"""Patch installers for LiteLLM SDK entry points.

Each ``_patch_*`` function captures the existing ``litellm.*`` attribute,
appends it to ``state.originals`` for ``uninstall_shim()`` to restore,
then assigns a wrapper that funnels into ``_DirectCore``.

The wrappers all share the same shape:

  1. Check ``_IN_FLIGHT.get()`` first — re-entry returns the original
     directly (no double-reserve).
  2. Set ``_IN_FLIGHT = True`` via token-based ContextVar.
  3. Delegate to ``state.core(_original_acompletion=original, **kwargs)``.
  4. Reset the ContextVar token in ``finally``.

Sync wrappers (``completion`` / ``text_completion``) add an
``asyncio.get_running_loop()`` guard up front to refuse calls from
inside a running loop (would deadlock the bridging ``asyncio.run``).
"""

from __future__ import annotations
