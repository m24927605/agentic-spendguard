"""Re-exports of SpendGuard SDK error types under the Atomic Agents namespace.

Lets users write ``from spendguard.integrations.atomic_agents import
DecisionDenied`` without remembering the cross-module path. Parity
with the autogen / beeai / dspy / agno / strands / adk adapters'
``_errors.py``.

Per design.md §5 the DENY path raises ``DecisionDenied`` directly out
of the proxy's overridden ``.chat.completions.create*`` method.
``BaseAgent.run`` in atomic-agents 2.8.0 has no framework-side catch
on the create-call path (verified at
``atomic_agents/agents/base_agent.py``), so the raised
``DecisionDenied`` propagates straight to the caller without any
framework re-wrapping. No DEVIATION-1-style wrap is needed.
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
