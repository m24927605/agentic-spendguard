"""Re-exports of SpendGuard SDK error types under the Letta namespace.

Lets users write ``from spendguard.integrations.letta import
DecisionDenied`` without remembering the cross-module path. Parity
with the AutoGen / DSPy / Agno / Strands / ADK adapters' ``_errors.py``.

Per design.md §5 the DENY path raises ``DecisionDenied`` directly.
``LLMClientBase`` has no callback / hook surface (Letta uses
``step_callback`` for coarse turn-level gating, which is documented as
inadequate per design.md §3 / non-goal). We subclass the ABC and wrap
the inner client, so a raised ``DecisionDenied`` (or any other
``SpendGuardError`` subclass) propagates straight back out through the
awaiting ``Agent.step`` without any framework re-wrapping (verified
against letta 0.8.0 — ``Agent.step`` does not blanket-catch on the
LLM-call path, so no DEVIATION-1-style wrap is needed).
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
