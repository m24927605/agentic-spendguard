"""Re-exports of SpendGuard SDK error types under the Agno namespace.

Lets users write ``from spendguard.integrations.agno import
DecisionDenied`` without remembering the cross-module path. Parity
with the ADK / Strands / MAF adapters' ``_errors.py``.

Per design.md §5 the DENY path raises ``DecisionDenied`` directly. The
pre-hook wraps it into an ``InputCheckError`` so Agno HALTS the run
before the vendor SDK fires; the original ``DecisionDenied`` is
preserved on ``__cause__``.

DEVIATION-1 vs spec §6 (locked) — REVISED against agno 2.6.18: the spec
asserted "STOP / DENY raises ``DecisionDenied`` — Agno propagates the
exception out of arun()". That is false. Agno's hook loop swallows
everything that is not an ``Input/OutputCheckError`` (so without the
wrap a DENY would be *logged* and the model would still be called — the
wrap is load-bearing), AND the ``InputCheckError`` halt does NOT
propagate out of ``Agent.arun()`` either: ``arun()`` RETURNS a
``RunOutput(status=RunStatus.error)`` (verified against the 2.6.18
wheel). Callers detect the deny via ``RunOutput.status``, not by
catching ``DecisionDenied``. The model is still blocked (PRE before
vendor SDK, review standards §3).
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
