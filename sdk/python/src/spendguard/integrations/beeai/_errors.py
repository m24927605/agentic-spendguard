"""Re-exports of SpendGuard SDK error types under the BeeAI namespace.

Lets users write ``from spendguard.integrations.beeai import
DecisionDenied`` without remembering the cross-module path. Parity
with the ADK / Strands / MAF / Agno / DSPy adapters' ``_errors.py``.

Per ``docs/specs/coverage/D23_beeai/design.md`` §5 the DENY path
raises ``DecisionDenied`` directly. BeeAI's ``Emitter._invoke`` wraps
any exception raised inside a listener into ``EmitterError``
preserving the original via ``__cause__`` (see
``beeai_framework/emitter/emitter.py`` line ~244 ``_invoke``). The
adapter does NOT swallow ``DecisionDenied`` so the wrapped
``EmitterError`` propagates out of the await chain rooted at
``ReActAgent.run(...)``; callers can catch by either
``DecisionDenied`` (via ``__cause__``) or by
``EmitterError`` (BeeAI's framework error). The DENY path is
documented in the public docs page and in the docstring of
``subscribe_spendguard`` so users self-correct without reading
the source.
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
