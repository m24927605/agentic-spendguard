"""Re-exports of SpendGuard SDK error types under the SmolAgents namespace.

Lets users write ``from spendguard.integrations.smolagents import
DecisionDenied`` without remembering the cross-module path. Parity with
the AutoGen / DSPy / Agno / Strands / ADK / BeeAI adapters' ``_errors.py``.

Per design.md §5 the DENY path raises ``DecisionDenied`` directly.
``smolagents.Model.generate`` is invoked synchronously inside
``MultiStepAgent.step()``; the framework lets exceptions bubble back out
to the agent's main loop without rewrapping (verified against
smolagents 1.26 — ``agents.py`` propagates non-``AgentError`` exceptions
verbatim), so the raised ``DecisionDenied`` reaches the
``CodeAgent.run`` / ``ToolCallingAgent.run`` caller cleanly. No
DEVIATION-1-style wrap (Agno-style) is needed.
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
