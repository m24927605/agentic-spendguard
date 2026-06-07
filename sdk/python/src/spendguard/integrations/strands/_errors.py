"""Re-exports of SpendGuard SDK error types under the Strands namespace.

Lets users write ``from spendguard.integrations.strands import
DecisionDenied`` without remembering the cross-module path. Parity with
the ADK / Microsoft Agent Framework adapters' ``_errors.py``.

Per design.md §5 the DENY path raises ``DecisionDenied`` directly so
Strands' typed event-bus runtime surfaces it as ``HookExecutionError``;
callers catch via the ``__cause__`` chain. The re-export is provided for
users who want to type-check or catch the underlying exception when
extending the provider (e.g. wrap it with their own observer hook).
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
    """Sidecar returned DEGRADE and the provider is fail-closed.

    Distinct subclass of ``SidecarUnavailable`` so operators can catch
    DEGRADE specifically (versus connection/timeout failures). Strands
    surfaces it as ``HookExecutionError`` with this in ``__cause__``.

    Set ``SPENDGUARD_STRANDS_FAIL_OPEN=1`` for dev-only fail-open
    behaviour (allows the invocation; commit will not fire). The
    default is fail-closed per ADR-005.
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
