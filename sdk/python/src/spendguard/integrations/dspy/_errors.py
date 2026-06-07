"""Re-exports of SpendGuard SDK error types under the DSPy namespace.

Lets users write ``from spendguard.integrations.dspy import
DecisionDenied`` without remembering the cross-module path. Parity with
the ADK / AWS Strands adapters' ``_errors.py``.

Per design.md §5 the DENY path raises ``DecisionDenied`` directly so
DSPy's runtime surfaces it to the caller before the LM dispatches.
DEGRADE raises ``SpendGuardDegradeBlocked`` (fail-closed default).
"""

from __future__ import annotations

from ...errors import (
    ApprovalRequired,
    DecisionDenied,
    DecisionSkipped,
    DecisionStopped,
    HandshakeError,
    SidecarUnavailable,
    SpendGuardConfigError,
    SpendGuardError,
)


class SpendGuardDegradeBlocked(SidecarUnavailable):
    """Sidecar returned DEGRADE and the callback is fail-closed.

    Distinct subclass of ``SidecarUnavailable`` so operators can catch
    DEGRADE specifically (versus connection/timeout failures). Set
    ``SPENDGUARD_DSPY_FAIL_OPEN=1`` for dev-only fail-open behaviour
    (allows the LM call; commit will not fire). The default is
    fail-closed per ADR-005.
    """


__all__ = [
    "ApprovalRequired",
    "DecisionDenied",
    "DecisionSkipped",
    "DecisionStopped",
    "HandshakeError",
    "SidecarUnavailable",
    "SpendGuardConfigError",
    "SpendGuardDegradeBlocked",
    "SpendGuardError",
]
