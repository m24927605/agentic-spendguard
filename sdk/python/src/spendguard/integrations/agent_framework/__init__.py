"""Microsoft Agent Framework (MAF) integration — SpendGuard middleware.

Drop-in MAF middleware that gates every `ChatClient.get_response` call
through the SpendGuard sidecar. Provides PRE-call reservation via
``RequestDecision(LLM_CALL_PRE)`` and POST-call usage commit via
``EmitTraceEvents(LLM_CALL_POST)`` — mirroring the .NET
`Spendguard.AgentFramework` NuGet shape from D07 SLICE_01–04.

Integration shape::

    from agent_framework import ChatAgent
    from agent_framework.openai import OpenAIChatClient

    from spendguard import SpendGuardClient
    from spendguard.integrations.agent_framework import (
        SpendGuardAgentFrameworkOptions,
        SpendGuardMiddleware,
        SpendGuardToolMiddleware,
        run_context,
        RunContext,
    )
    from spendguard._proto.spendguard.common.v1 import common_pb2

    client = SpendGuardClient(socket_path=..., tenant_id="...")
    await client.connect()
    await client.handshake()

    chat_middleware = SpendGuardMiddleware(
        client=client,
        options=SpendGuardAgentFrameworkOptions(
            tenant_id="...",
            budget_id="b1",
            window_instance_id="w1",
            sidecar_socket_path="/var/run/spendguard/sidecar.sock",
        ),
        unit=common_pb2.UnitRef(...),
        pricing=common_pb2.PricingFreeze(...),
    )

    agent = ChatAgent(
        chat_client=OpenAIChatClient(...),
        middleware=[chat_middleware],
    )
    async with run_context(RunContext(run_id="run-123")):
        result = await agent.run("Hello!")

Module layout (COV_d07_05):
  - ``_options.py``    — ``SpendGuardAgentFrameworkOptions`` dataclass.
  - ``_errors.py``     — re-exports of SDK error types (parity import path).
  - ``_run_context.py``— per-async-task ``ContextVar`` stash + manager.
  - ``_middleware.py`` — ``SpendGuardMiddleware`` (LLM-scope ChatMiddleware).
  - ``_tool_middleware.py`` — ``SpendGuardToolMiddleware`` (opt-in tool scope).

DEVIATION from design.md §3.4 / implementation.md §2.5:
  The spec asserts the MAF middleware base class lives at the import
  path ``agent_framework.middleware.ChatMiddleware``. The shipped MAF
  1.x line re-exports the symbol at the **top level** —
  ``agent_framework.ChatMiddleware`` — and the internal definition lives
  at the private ``agent_framework._middleware`` module. The middleware
  contract (abstract ``process(context, call_next)`` method on
  ``ChatMiddleware`` + ``ChatContext.messages`` / ``ChatContext.result``
  observable surface) is wire-stable; only the import path moved.
  This integration imports from the top-level public namespace
  (``from agent_framework import ChatMiddleware, ChatContext, …``) per
  MAF's documented public API. Documented here per review-standards
  §5 (cross-language drift trigger).

POC scope:
  - Streaming gating is bracketed at the chat-client boundary; per-chunk
    gating is a follow-on (parity with langchain / openai_agents).
  - DEGRADE mutation patches surfaced as APPLY_FAILED rather than
    applied (parity with other Python adapters).
  - Tool-scope gating is opt-in via ``SpendGuardToolMiddleware``; the
    chat middleware does NOT auto-emit ``TOOL_CALL_PRE`` for each
    function call (ADR-002).
"""

from __future__ import annotations

from ._errors import (
    DecisionDenied,
    SidecarUnavailable,
    SpendGuardConfigError,
    SpendGuardError,
)
from ._middleware import SpendGuardMiddleware
from ._options import SpendGuardAgentFrameworkOptions
from ._run_context import RunContext, current_run_context, run_context
from ._tool_middleware import SpendGuardToolMiddleware

__all__ = [
    # public middleware classes
    "SpendGuardMiddleware",
    "SpendGuardToolMiddleware",
    # options
    "SpendGuardAgentFrameworkOptions",
    # run-context machinery
    "RunContext",
    "run_context",
    "current_run_context",
    # error re-exports for ergonomic catch-from-one-place
    "DecisionDenied",
    "SidecarUnavailable",
    "SpendGuardConfigError",
    "SpendGuardError",
]
