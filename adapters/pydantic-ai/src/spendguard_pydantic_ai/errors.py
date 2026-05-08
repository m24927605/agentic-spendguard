"""Exception hierarchy for the SpendGuard Pydantic-AI adapter.

These map sidecar-side decision outcomes to Python exceptions that
Pydantic-AI's Agent run loop will surface to the caller. The mapping is
narrow on purpose: only STOP and APPROVAL_TIMED_OUT terminate the run;
DEGRADE applies a mutation and continues; SKIP raises a non-fatal signal
the caller can catch.
"""

from __future__ import annotations


class SpendGuardError(Exception):
    """Base class for all spendguard adapter errors."""


class HandshakeError(SpendGuardError):
    """Sidecar handshake failed (version mismatch, signature invalid, etc.)."""


class SidecarUnavailable(SpendGuardError):
    """Sidecar UDS is unreachable or not responding within the deadline.

    Adapters in fail-closed mode should propagate this; fail-open
    deployments may catch and continue. Default per Sidecar §11 is
    fail-closed.
    """


class DecisionDenied(SpendGuardError):
    """Sidecar returned a non-CONTINUE / non-DEGRADE decision.

    Carries the original `decision_id` and reason codes so callers can
    log + correlate with audit chain. Subclassed for the specific
    decision kind.
    """

    def __init__(
        self,
        message: str,
        *,
        decision_id: str,
        reason_codes: list[str] | None = None,
        audit_decision_event_id: str | None = None,
    ) -> None:
        super().__init__(message)
        self.decision_id = decision_id
        self.reason_codes = reason_codes or []
        self.audit_decision_event_id = audit_decision_event_id


class DecisionStopped(DecisionDenied):
    """Sidecar returned STOP; run must terminate."""


class DecisionSkipped(DecisionDenied):
    """Sidecar returned SKIP; this trigger boundary is skipped (non-fatal)."""


class ApprovalRequired(DecisionDenied):
    """Sidecar returned REQUIRE_APPROVAL; run is paused pending approver."""

    def __init__(
        self,
        message: str,
        *,
        decision_id: str,
        approval_request_id: str,
        approver_role: str | None = None,
        reason_codes: list[str] | None = None,
        audit_decision_event_id: str | None = None,
    ) -> None:
        super().__init__(
            message,
            decision_id=decision_id,
            reason_codes=reason_codes,
            audit_decision_event_id=audit_decision_event_id,
        )
        self.approval_request_id = approval_request_id
        self.approver_role = approver_role


class MutationApplyFailed(SpendGuardError):
    """A DEGRADE decision returned a mutation patch the adapter could not apply.

    The adapter MUST surface this to the sidecar via ConfirmPublishOutcome
    with `outcome=APPLY_FAILED` so the audit chain records the failure.
    """
