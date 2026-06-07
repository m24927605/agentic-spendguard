"""Run-context auto-binding for Langflow-driven invocations.

Langflow nodes call ``ainvoke()`` / ``invoke()`` without wrapping the
call in ``spendguard.integrations.langchain.run_context()``. Without a
bound context the SDK raises ``RuntimeError``. We monkey-patch the
returned wrapper's ``_agenerate`` to enter a context using the Langflow
``flow_id`` (or a stable ``uuid4()`` fallback) when none is bound.

INV-3: caller-bound contexts ALWAYS win -- we only enter if
``_RUN_CONTEXT.get() is None``.

INV-4: the monkey-patch lives on the returned instance (not on the
class), so per-build invocations stay isolated and module-level state
never leaks across builds.
"""

from __future__ import annotations

import functools
import uuid
from typing import Any


def install_autobind(
    wrapped: Any,
    *,
    flow_id: str | None,
) -> Any:
    """Wrap ``wrapped._agenerate`` so each call auto-binds a run-context.

    Args:
        wrapped: a ``SpendGuardChatModel`` instance returned by the
            wrapper's ``build_model``.
        flow_id: Langflow's per-flow identifier (``self.graph.flow_id``).
            ``None`` -> generate a ``uuid4()`` fallback so canvas
            re-renders still produce stable per-build run-ids.

    Returns:
        The same ``wrapped`` instance with ``_agenerate`` patched.
    """
    from spendguard.integrations.langchain import (
        RunContext,
        _RUN_CONTEXT,
        run_context,
    )

    original_agen = wrapped._agenerate
    base_run_id = flow_id or f"langflow-{uuid.uuid4()}"
    call_counter = {"n": 0}

    @functools.wraps(original_agen)
    async def _agenerate_autobind(
        messages: Any,
        stop: Any = None,
        run_manager: Any = None,
        **kwargs: Any,
    ) -> Any:
        # INV-3: caller-bound contexts win.
        if _RUN_CONTEXT.get() is not None:
            return await original_agen(messages, stop, run_manager, **kwargs)
        call_counter["n"] += 1
        ctx = RunContext(run_id=f"{base_run_id}:{call_counter['n']}")
        async with run_context(ctx):
            return await original_agen(messages, stop, run_manager, **kwargs)

    # Pydantic v2 BaseModel blocks attribute set on validated fields,
    # but ``_agenerate`` is a method. ``object.__setattr__`` bypasses
    # the descriptor + validator chain so the patch lands on this
    # instance only (INV-4 — no class-global mutation).
    object.__setattr__(wrapped, "_agenerate", _agenerate_autobind)
    return wrapped


__all__ = ["install_autobind"]
