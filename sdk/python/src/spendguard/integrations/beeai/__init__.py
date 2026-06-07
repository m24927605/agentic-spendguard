"""BeeAI Framework (IBM Research + Linux Foundation) `Emitter` integration.

Subscribes a single async handler to a ``BaseAgent``'s ``Emitter``
that intercepts ``*.llm.*.start`` / ``*.llm.*.success`` /
``*.llm.*.error`` events and routes them through SpendGuard's
``RequestDecision(LLM_CALL_PRE)`` → ``EmitLlmCallPost(SUCCESS)``
lifecycle. The same subscriber gates every BeeAI ``ChatModel``
backend (``OpenAIChatModel`` / ``WatsonxChatModel`` /
``OllamaChatModel`` / ``GroqChatModel`` / custom) because BeeAI's
first-party extension surface is the agent-runtime ``Emitter`` —
there is no per-vendor model wrapping involved.

Integration shape::

    from beeai_framework.agents.react import ReActAgent
    from beeai_framework.backend.chat import ChatModel
    from spendguard import SpendGuardClient, new_uuid7
    from spendguard.integrations.beeai import (
        RunContext, run_context, subscribe_spendguard,
    )
    from spendguard._proto.spendguard.common.v1 import common_pb2

    client = SpendGuardClient(socket_path=..., tenant_id=...)
    await client.connect()
    await client.handshake()

    unit = common_pb2.UnitRef(unit_id="usd_micros",
                              token_kind="output_token",
                              model_family="gpt-4")
    pricing = common_pb2.PricingFreeze(pricing_version="2026-q2")

    llm = ChatModel.from_name("openai:gpt-4o-mini")
    agent = ReActAgent(llm=llm, tools=[])

    unsubscribe = subscribe_spendguard(
        agent, client,
        budget_id="...", window_instance_id="...",
        unit=unit, pricing=pricing,
    )
    try:
        async with run_context(RunContext(run_id=str(new_uuid7()))):
            result = await agent.run("Say hello in three words.")
    finally:
        unsubscribe()

POC scope:
  - Streaming (``newToken`` / ``partialUpdate``) mid-stream gating
    is OUT (spec §3 non-goal); commit only after the final
    ``success`` event.
  - Tool-call mid-loop gating is OUT — v1 subscribes to ``llm.*``
    only; tool gating is the ``integrations.agt`` territory.
  - DEGRADE mutation patches are surfaced as APPLY_FAILED rather
    than applied (parity with langchain / pydantic-ai integrations).
  - DENY raises ``DecisionDenied``. BeeAI's ``Emitter._invoke``
    wraps any listener exception as ``EmitterError`` preserving
    ``__cause__`` — callers may catch by either type.

DEVIATIONS vs ``docs/specs/coverage/D23_beeai/design.md`` (locked):
  - DEVIATION-A: spec §4 / implementation.md §4 pinned
    ``beeai-framework>=0.3,<1.0``. Reality (2026-06-08): the actual
    PyPI release line is ``0.1.x`` with ``0.1.81`` as the latest;
    there is no ``0.3.x``. We pin ``>=0.1.81,<0.2`` in
    ``pyproject.toml`` so the extra floors at the version where
    ``Emitter.match`` returns a ``CleanupFn`` (verified at
    ``beeai_framework/emitter/emitter.py:176``) and the
    ``BaseAgent.emitter`` ``cached_property`` is stable
    (``beeai_framework/agents/base.py``).
  - DEVIATION-B: spec §5 R1 / implementation.md §2 imports
    ``run_context`` / ``current_run_context`` directly from
    ``spendguard.integrations.langchain``. Reality: ``langchain.py``
    raises ``ImportError`` at import time if ``langchain_core`` is
    missing (see ``langchain.py:66-74``), so re-importing into
    ``beeai.py`` would force every BeeAI user to install
    ``[langchain]`` transitively — a Blocker per review-standards
    §4 R1. We define a fresh ``ContextVar`` with the SAME NAME
    (``spendguard_run_context``) in ``_hook.py``; Python ContextVars
    are looked up by name at the interpreter level so cross-adapter
    run_id sharing still works exactly like spec §5 R1 intended.
    Mirrors the same compromise the Agno integration made
    (``agno/_hook.py:93``).

Module layout:
  - ``__init__.py`` (this file) — public surface, import-time guard.
  - ``_errors.py`` — error re-exports.
  - ``_options.py`` — ``RunContext`` + ``SpendGuardBeeAIOptions``.
  - ``_hook.py`` — ``subscribe_spendguard`` + handlers + inflight map.
"""

from __future__ import annotations

# Import-time guard: surface a helpful error pointing at the
# ``pip install 'spendguard-sdk[beeai]'`` install command when the
# user imports this module without the ``beeai-framework`` package
# installed. The guard fires once at module load; the
# ``subscribe_spendguard`` helper itself accepts duck-typed agents
# in tests so the unit suite still runs.
try:
    from beeai_framework.agents.base import BaseAgent  # noqa: F401
    from beeai_framework.emitter.emitter import Emitter, EventMeta  # noqa: F401
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.beeai requires the [beeai] extra. "
        "Install with: pip install 'spendguard-sdk[beeai]'"
    ) from exc

from ._errors import (
    ApprovalRequired,
    DecisionDenied,
    DecisionSkipped,
    DecisionStopped,
    HandshakeError,
    MutationApplyFailed,
    SidecarUnavailable,
    SpendGuardConfigError,
    SpendGuardError,
)
from ._hook import (
    BeeAiStartEvent,
    CallSignatureFn,
    ClaimEstimator,
    current_run_context,
    run_context,
    subscribe_spendguard,
)
from ._options import RunContext, SpendGuardBeeAIOptions

__all__ = [
    # Public surface — LOCKED per design.md §7 / review-standards §5.
    "BeeAiStartEvent",
    "CallSignatureFn",
    "ClaimEstimator",
    "RunContext",
    "SpendGuardBeeAIOptions",
    "current_run_context",
    "run_context",
    "subscribe_spendguard",
    # Error re-exports (catch-from-one-place).
    "ApprovalRequired",
    "DecisionDenied",
    "DecisionSkipped",
    "DecisionStopped",
    "HandshakeError",
    "MutationApplyFailed",
    "SidecarUnavailable",
    "SpendGuardConfigError",
    "SpendGuardError",
]
