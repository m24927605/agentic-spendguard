"""Re-exports of SpendGuard SDK error types under the AutoGen namespace.

Lets users write ``from spendguard.integrations.autogen import
DecisionDenied`` without remembering the cross-module path. Parity
with the DSPy / Agno / Strands / ADK adapters' ``_errors.py``.

Per design.md §5 the DENY path raises ``DecisionDenied`` directly.
``ChatCompletionClient`` has no callback / hook surface in either
AutoGen 0.4+ or AG2 — both lineages call ``model_client.create(...)``
inside ``AssistantAgent._call_llm`` (AutoGen) or the equivalent AG2
path. We subclass the ABC and wrap the inner client, so a raised
``DecisionDenied`` (or any other ``SpendGuardError`` subclass)
propagates straight back out through the awaiting ``AssistantAgent``
without any framework re-wrapping. AssistantAgent does NOT catch
``Exception`` on the create-call path the way Agno does (see
``autogen_agentchat/agents/_assistant_agent.py`` — the LLM call
sits inside the agent's main coroutine and exceptions bubble), so no
DEVIATION-1-style wrap is needed.
"""

from __future__ import annotations

from ...errors import (
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

__all__ = [
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
