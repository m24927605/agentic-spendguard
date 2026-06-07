"""Re-exports of SpendGuard SDK error types under the MAF integration namespace.

Lets users write ``from spendguard.integrations.agent_framework import
DecisionDenied`` without having to remember the cross-module path.
Matches the parity established by the .NET ``Spendguard.AgentFramework``
exception namespace (``SpendGuardDecisionDeniedException`` etc.).

Per review-standards.md §2.3 P3 — exception type names map to the .NET
side as:

    Python                       .NET
    --------------------------   --------------------------------
    DecisionDenied               SpendGuardDecisionDeniedException
    SidecarUnavailable           SidecarUnavailableException
    SpendGuardConfigError        SpendGuardConfigurationException
"""

from __future__ import annotations

from ...errors import (
    DecisionDenied,
    DecisionSkipped,
    DecisionStopped,
    HandshakeError,
    SidecarUnavailable,
    SpendGuardConfigError,
    SpendGuardError,
)

__all__ = [
    "DecisionDenied",
    "DecisionSkipped",
    "DecisionStopped",
    "HandshakeError",
    "SidecarUnavailable",
    "SpendGuardConfigError",
    "SpendGuardError",
]
