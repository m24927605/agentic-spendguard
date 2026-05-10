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
        matched_rule_ids: list[str] | None = None,
    ) -> None:
        super().__init__(message)
        self.decision_id = decision_id
        self.reason_codes = reason_codes or []
        self.audit_decision_event_id = audit_decision_event_id
        # Phase 3 wedge: which contract rules fired. Useful for audit
        # correlation when callers want to know "why was this denied?"
        # without re-querying canonical_events.
        self.matched_rule_ids = matched_rule_ids or []


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
        matched_rule_ids: list[str] | None = None,
        tenant_id: str | None = None,
    ) -> None:
        super().__init__(
            message,
            decision_id=decision_id,
            reason_codes=reason_codes,
            audit_decision_event_id=audit_decision_event_id,
            matched_rule_ids=matched_rule_ids,
        )
        self.approval_request_id = approval_request_id
        self.approver_role = approver_role
        # Round-2 #9 part 2 PR 9d: tenant_id needed for the resume()
        # round-trip — sidecar's ResumeAfterApproval RPC requires it
        # to scope the GetApprovalForResume lookup against tenant.
        self.tenant_id = tenant_id

    async def resume(self, client: "SpendGuardClient"):  # type: ignore[name-defined]  # noqa: F821
        """Call sidecar `ResumeAfterApproval` after the operator has
        approved (or denied) this request.

        Pydantic-AI usage:

            try:
                await client.request_decision(...)
            except ApprovalRequired as e:
                # ... wait for approver via your control plane / Slack ...
                outcome = await e.resume(client)

        Returns a `DecisionOutcome` on `approved` (the run can continue)
        and raises one of:
          * `ApprovalDeniedError` — operator rejected the approval
          * `ApprovalLapsedError` — pending / expired / cancelled
          * `SpendGuardError`     — transport / proto error
        """
        return await client.resume_after_approval(
            approval_id=self.approval_request_id,
            tenant_id=self.tenant_id or "",
            decision_id=self.decision_id,
        )


class ApprovalDeniedError(DecisionDenied):
    """Sidecar `ResumeAfterApproval` returned `denied`.

    Round-2 #9 part 2 PR 9d: raised by `ApprovalRequired.resume()` when
    the approver explicitly rejected the request. Carries the approver
    identity + reason for caller-side audit logging.
    """

    def __init__(
        self,
        message: str,
        *,
        decision_id: str,
        approver_subject: str | None = None,
        approver_reason: str | None = None,
        audit_decision_event_id: str | None = None,
        matched_rule_ids: list[str] | None = None,
    ) -> None:
        super().__init__(
            message,
            decision_id=decision_id,
            reason_codes=["approval_denied"],
            audit_decision_event_id=audit_decision_event_id,
            matched_rule_ids=matched_rule_ids,
        )
        self.approver_subject = approver_subject
        self.approver_reason = approver_reason


class ApprovalLapsedError(DecisionDenied):
    """Sidecar `ResumeAfterApproval` returned a non-actionable state.

    Round-2 #9 part 2 PR 9d: raised when the approval is in a state
    that callers cannot resume from — `pending` (still waiting),
    `expired` (TTL elapsed), `cancelled` (operator-cancelled), or
    something the resume handler doesn't recognise.

    The original `decision_id` is preserved so callers can correlate
    with their audit chain. The `state` attribute carries the
    sidecar-reported state for adaptive UI / retry logic.
    """

    def __init__(
        self,
        message: str,
        *,
        decision_id: str,
        state: str,
        audit_decision_event_id: str | None = None,
    ) -> None:
        super().__init__(
            message,
            decision_id=decision_id,
            reason_codes=[f"approval_lapsed_{state}"],
            audit_decision_event_id=audit_decision_event_id,
        )
        self.state = state


class MutationApplyFailed(SpendGuardError):
    """A DEGRADE decision returned a mutation patch the adapter could not apply.

    The adapter MUST surface this to the sidecar via ConfirmPublishOutcome
    with `outcome=APPLY_FAILED` so the audit chain records the failure.
    """
