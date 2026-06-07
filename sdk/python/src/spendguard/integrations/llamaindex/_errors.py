"""Re-exports of SpendGuard SDK error types under the LlamaIndex namespace.

Lets users write ``from spendguard.integrations.llamaindex import
SpendGuardLlamaIndexDenied`` without remembering the cross-module path.
Parity with the AutoGen / DSPy / BeeAI / Agno / Strands / ADK adapters'
``_errors.py``.

Per design.md §2 the DENY path raises ``SpendGuardLlamaIndexDenied`` from
inside ``on_event_start`` — LlamaIndex has no documented "skip event"
return channel for ``CBEventType.LLM`` events, so raising IS the stop
signal. LlamaIndex's ``CallbackManager.event(...)`` context manager
propagates exceptions out through the enclosing ``LLM.chat`` /
``LLM.predict`` call, so a raised ``SpendGuardLlamaIndexDenied`` reaches
the caller's ``query_engine.query(...)`` invocation cleanly.
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


class SpendGuardLlamaIndexDenied(SpendGuardError):
    """Raised from ``on_event_start`` to short-circuit the LLM call.

    LlamaIndex's ``BaseCallbackHandler.on_event_start`` returns the
    ``event_id`` string but has no documented "veto" return value —
    raising an exception is the canonical stop signal. The exception
    propagates as the LLM call's terminal exception and is observable
    by the caller's own ``try``/``except`` block plus the framework's
    own tracer/observer hooks.

    Distinct from ``DecisionDenied`` so callers can catch LlamaIndex
    refusals separately from other adapter denials (e.g. when both D27
    and D12 are wired and the operator wants to log the surface that
    fired). The original ``DecisionDenied`` from the SpendGuard client
    is chained via ``__cause__`` (the ``raise ... from exc`` idiom).

    Attributes:
        reason_codes: List of policy reason codes from the sidecar
            decision (e.g. ``["BUDGET_EXHAUSTED"]``). Always a list,
            never ``None``.
        decision_id: Sidecar-minted decision id for cross-system
            correlation. Empty string when unavailable.
    """

    def __init__(
        self,
        reason_codes: list[str],
        decision_id: str = "",
    ) -> None:
        """Construct with reason codes + optional decision id.

        Args:
            reason_codes: Policy reason codes. ``None`` is coerced to
                ``["BUDGET_EXHAUSTED"]`` for backward-friendly default.
            decision_id: Sidecar decision id. Empty string when the
                client raised before the decision was minted.
        """
        self.reason_codes: list[str] = list(reason_codes) if reason_codes else []
        self.decision_id: str = decision_id
        codes = ",".join(self.reason_codes) if self.reason_codes else "BUDGET_EXHAUSTED"
        super().__init__(f"SpendGuard denied LLM call: {codes}")


__all__ = [
    "ApprovalRequired",
    "DecisionDenied",
    "DecisionSkipped",
    "DecisionStopped",
    "HandshakeError",
    "SidecarUnavailable",
    "SpendGuardConfigError",
    "SpendGuardError",
    "SpendGuardLlamaIndexDenied",
]
