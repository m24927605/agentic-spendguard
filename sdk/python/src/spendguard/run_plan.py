"""``with_run_plan`` decorator — SDK Signal 3 (explicit hint) wire.

Spec ref ``run-cost-projector-spec-v1alpha1.md`` §5.

A power-user opts in by decorating their agent function with the
expected number of LLM calls and tool calls. The SDK stuffs the sum
into the ``DecisionRequest.planned_steps_hint`` wire field; the
sidecar forwards to the run_cost_projector which uses Signal 3 to
override the Signal 1 induced estimate.

Usage::

    from spendguard import with_run_plan

    @with_run_plan(planned_calls=8, planned_tools=2)
    async def my_agent(query: str) -> str:
        # agent runs N LLM calls + M tool calls
        return await runner.run(...)

The decorator works on both sync AND async callables. Nested usage
keeps the outer plan (per spec semantics — outer scope owns the
budget envelope; inner functions don't replan).

Wire path::

    @with_run_plan(planned_calls=8, planned_tools=2)
    └─ context-var ``_RUN_PLAN`` set to RunPlan(8, 2)
       └─ on each request_decision call inside the decorated frame,
          the integration reads ``current_run_plan()`` and passes
          ``planned_steps_hint = 8 + 2 = 10`` to the sidecar
       └─ sidecar forwards to projector as ``ProjectRequest.planned_steps_hint``

The hint is **opt-in** — without ``with_run_plan`` the SDK does NOT
attach a hint and the projector falls back to Signal 1 (history-induced).
"""

from __future__ import annotations

import asyncio
import contextvars
import functools
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Any, ParamSpec, TypeVar, overload


P = ParamSpec("P")
R = TypeVar("R")


@dataclass(frozen=True, slots=True)
class RunPlan:
    """Caller-declared plan for one logical run.

    ``planned_calls`` — expected number of LLM calls in the run.
    ``planned_tools`` — expected number of tool calls in the run.

    ``planned_steps_hint`` = ``planned_calls + planned_tools`` (per
    spec §5.1; sidecar/projector treats steps as the disjoint union of
    LLM + tool calls).

    The projector validates ``planned_steps_hint`` is in
    ``[0, MAX_PLANNED_STEPS]`` (see
    ``services/run_cost_projector/src/server.rs`` MAX_PLANNED_STEPS);
    we don't repeat the bound here so a future Rust-side bump doesn't
    require an SDK release.
    """

    planned_calls: int
    planned_tools: int

    @property
    def planned_steps_hint(self) -> int:
        return self.planned_calls + self.planned_tools


# Context-var so integrations can read the active plan without
# threading it through every public API. Mirrors the
# ``_RUN_CONTEXT`` pattern in each integration.
_RUN_PLAN: contextvars.ContextVar[RunPlan | None] = contextvars.ContextVar(
    "spendguard_run_plan", default=None
)


def current_run_plan() -> RunPlan | None:
    """Return the active ``RunPlan`` if a frame above set one.

    Returns ``None`` when called outside a ``with_run_plan`` decorated
    callable — i.e. the request will NOT attach a Signal 3 hint and
    the projector falls back to Signal 1 (history-induced).
    """
    return _RUN_PLAN.get()


@overload
def with_run_plan(
    planned_calls: int,
    planned_tools: int | None = ...,
) -> Callable[[Callable[P, Awaitable[R]]], Callable[P, Awaitable[R]]]: ...


@overload
def with_run_plan(
    planned_calls: int,
    planned_tools: int | None = ...,
) -> Callable[[Callable[P, R]], Callable[P, R]]: ...


def with_run_plan(
    planned_calls: int,
    planned_tools: int | None = None,
) -> Callable[[Callable[..., Any]], Callable[..., Any]]:
    """Decorator that attaches Signal 3 (``planned_steps_hint``) to all
    ``request_decision`` calls inside the decorated frame.

    Parameters
    ----------
    planned_calls
        Expected LLM calls in the run. Must be ``>= 0``.
    planned_tools
        Optional expected tool calls. Defaults to ``0`` if not provided.

    Returns
    -------
    A decorator that preserves the original function's sync/async
    semantics (via ``inspect.iscoroutinefunction`` detection at wrap
    time, with a fallback async-aware ``functools.wraps``).

    Raises
    ------
    TypeError
        If ``planned_calls`` or ``planned_tools`` aren't non-negative
        integers (clear error per SLICE_12 §7 failure-mode column).
    TypeError
        If the decorated target isn't callable (e.g. someone decorates
        a string by mistake).

    Notes
    -----
    Nested decoration: the **outermost** ``with_run_plan`` wins (per
    spec — inner scopes don't replan once the budget envelope is set).
    A nested ``with_run_plan`` is a no-op for context purposes; the
    outer frame's plan remains active.

    The decorator does NOT itself wire the hint into ``request_decision``
    — that wiring lives in each integration (Phase C of SLICE_12
    updates the five integrations to read ``current_run_plan()`` and
    pass the hint).
    """
    # Argument validation at decorator construction time (catches
    # bugs at module import rather than first call).
    if not isinstance(planned_calls, int) or planned_calls < 0:
        raise TypeError(
            f"with_run_plan: planned_calls must be a non-negative int, "
            f"got {planned_calls!r}"
        )
    if planned_tools is None:
        planned_tools = 0
    if not isinstance(planned_tools, int) or planned_tools < 0:
        raise TypeError(
            f"with_run_plan: planned_tools must be a non-negative int "
            f"or None, got {planned_tools!r}"
        )

    plan = RunPlan(planned_calls=planned_calls, planned_tools=planned_tools)

    def decorator(fn: Callable[..., Any]) -> Callable[..., Any]:
        if not callable(fn):
            raise TypeError(
                f"with_run_plan: target must be callable, got {fn!r}. "
                f"Decorate a sync `def` or async `async def` function."
            )

        is_coroutine = asyncio.iscoroutinefunction(fn)

        if is_coroutine:

            @functools.wraps(fn)
            async def async_wrapper(*args: Any, **kwargs: Any) -> Any:
                # Nested usage: if a plan is already active, defer to it
                # (outer wins). The context-var reset on the outer frame
                # exit cleans up.
                existing = _RUN_PLAN.get()
                if existing is not None:
                    return await fn(*args, **kwargs)
                token = _RUN_PLAN.set(plan)
                try:
                    return await fn(*args, **kwargs)
                finally:
                    _RUN_PLAN.reset(token)

            return async_wrapper

        @functools.wraps(fn)
        def sync_wrapper(*args: Any, **kwargs: Any) -> Any:
            existing = _RUN_PLAN.get()
            if existing is not None:
                return fn(*args, **kwargs)
            token = _RUN_PLAN.set(plan)
            try:
                return fn(*args, **kwargs)
            finally:
                _RUN_PLAN.reset(token)

        return sync_wrapper

    return decorator


__all__ = [
    "RunPlan",
    "current_run_plan",
    "with_run_plan",
]
