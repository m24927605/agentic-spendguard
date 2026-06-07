"""Per-async-task RunContext stash for MAF middleware.

Mirrors the ``contextvars.ContextVar`` pattern used by
``spendguard.integrations.langchain`` /
``spendguard.integrations.openai_agents`` so multi-framework agents
share a single ``run_id`` across stacks.

Implementation note:
  ``contextvars.ContextVar`` is the right primitive for MAF because the
  MAF middleware pipeline is awaitable + can fan out into parallel
  ``asyncio.gather`` branches. Each spawned async task inherits the
  parent's context copy at spawn time, so siblings cannot stomp on each
  other's stashed run state. Tests in test_middleware.py exercise the
  concurrent-isolation path explicitly.
"""

from __future__ import annotations

import contextvars
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from dataclasses import dataclass

_RUN_CONTEXT: contextvars.ContextVar[RunContext | None] = contextvars.ContextVar(
    "spendguard_agent_framework_run_context", default=None
)


@dataclass(frozen=True, slots=True)
class RunContext:
    """Per-MAF-agent.run() identifiers.

    Attributes:
        run_id: Caller-minted run identifier. Used as the
            ``RequestDecision.ids.run_id`` and as part of the
            idempotency-key derivation, so MAF retry middleware that
            re-enters the pipeline with the same logical step does not
            double-reserve.
    """

    run_id: str


@asynccontextmanager
async def run_context(ctx: RunContext) -> AsyncIterator[RunContext]:
    """Bind a ``RunContext`` for the duration of the wrapped block.

    Usage::

        async with run_context(RunContext(run_id="my-run-1")):
            result = await agent.run("hello")
    """
    token = _RUN_CONTEXT.set(ctx)
    try:
        yield ctx
    finally:
        _RUN_CONTEXT.reset(token)


def current_run_context() -> RunContext:
    """Return the active ``RunContext`` or raise a clear error.

    Raised error matches the message style of the langchain /
    openai_agents adapters so callers get the same setup-fix hint
    regardless of which Python integration they're using first.
    """
    ctx = _RUN_CONTEXT.get()
    if ctx is None:
        raise RuntimeError(
            "spendguard.integrations.agent_framework middleware called "
            "outside an active run_context(). Wrap your agent.run "
            "invocation:\n\n"
            "    from spendguard.integrations.agent_framework "
            "import RunContext, run_context\n"
            "    async with run_context(RunContext(run_id='...')):\n"
            "        await agent.run('hello')\n"
        )
    return ctx


__all__ = [
    "RunContext",
    "current_run_context",
    "run_context",
]
