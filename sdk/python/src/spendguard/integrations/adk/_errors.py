"""Re-exports of SpendGuard SDK error types under the ADK integration namespace.

Lets users write ``from spendguard.integrations.adk import DecisionDenied``
without having to remember the cross-module path. Parity with the
Microsoft Agent Framework adapter's ``_errors.py``.

The deny path of the adapter does NOT raise ``DecisionDenied`` to the
ADK runtime (per design.md §5: deny returns a synthetic ``LlmResponse``
with ``error_code="SPENDGUARD_DENY"``). The re-export is provided for
users who want to type-check or catch the underlying exception when
they extend the callback (e.g. wrap it with their own observer).
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
