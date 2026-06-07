"""Re-exports of SpendGuard SDK error types under the Agno namespace.

Lets users write ``from spendguard.integrations.agno import
DecisionDenied`` without remembering the cross-module path. Parity
with the ADK / Strands / MAF adapters' ``_errors.py``.

Per design.md §5 the DENY path raises ``DecisionDenied`` directly. In
Agno's actual 2.x ``aexecute_pre_hooks`` runtime, the loop catches
``Exception`` but **propagates ``InputCheckError``** unchanged (see
``agno.agent._hooks.aexecute_pre_hooks`` line 195 and following). The
pre-hook therefore wraps the ``DecisionDenied`` into an
``InputCheckError`` so Agno actually halts the run — the original
``DecisionDenied`` is preserved on ``__cause__`` for downstream
catch-by-type.

DEVIATION-1 vs spec §6 (locked): the spec asserted "STOP / DENY raises
``DecisionDenied`` — Agno propagates the exception out of arun()".
Agno's 2.x hook loop silently swallows everything that is not an
``Input/OutputCheckError``, so without the wrap a DENY would be
*logged* and the model would still be called. The wrap is the only
correctness-preserving path; bypassing it would violate review
standards §3 "PRE before vendor SDK".
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
