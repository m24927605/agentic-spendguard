"""SpendGuard SDK — runtime safety layer client for AI agent frameworks.

Core surface (always available):

    from spendguard import SpendGuardClient, DecisionStopped, derive_idempotency_key

Framework integrations are optional (install via extras):

    # pydantic-ai auto-install is temporarily fail-closed due to
    # CVE-2026-25580; install a vetted non-vulnerable upstream release
    # when available.
    pip install spendguard-sdk[langchain]
    pip install spendguard-sdk[langgraph]
    pip install spendguard-sdk[openai-agents]

After installing the relevant extras::

    from spendguard.integrations.pydantic_ai import SpendGuardModel
    from spendguard.integrations.langchain   import SpendGuardChatModel
    # ...
"""

from .client import (
    DEFAULT_DECISION_TIMEOUT_S,
    DEFAULT_HANDSHAKE_TIMEOUT_S,
    DecisionOutcome,
    HandshakeOutcome,
    ReleaseOutcome,
    SpendGuardClient,
)
from .errors import (
    ApprovalBundleHotReloadedError,
    ApprovalDeniedError,
    ApprovalLapsedError,
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
from .ids import (
    default_call_signature,
    derive_idempotency_key,
    derive_uuid_from_signature,
    new_uuid7,
    workload_instance_id,
)
from .run_plan import RunPlan, current_run_plan, with_run_plan
from .session import (
    DEFAULT_MAX_PENDING_SESSION_DELTAS,
    CommitSessionDeltaOutcome,
    CommitSessionDeltaRequest,
    PendingSessionDelta,
    ReleaseSessionOutcome,
    ReleaseSessionRequest,
    ReserveSessionAccepted,
    ReserveSessionDenied,
    ReserveSessionOutcome,
    ReserveSessionRequest,
    SessionDeltaCommitInput,
    SessionPendingDeltaLimitError,
    SessionReleaseInput,
    SessionReservationHandle,
    SessionReservationHandleError,
    SessionReservationHandleSnapshot,
    SessionReservationReleasedError,
    SessionReservationReplayMismatchError,
    build_commit_session_delta_request,
    build_release_session_request,
    build_reserve_session_request,
)

__all__ = [
    # client
    "DEFAULT_DECISION_TIMEOUT_S",
    "DEFAULT_HANDSHAKE_TIMEOUT_S",
    "DecisionOutcome",
    "HandshakeOutcome",
    "ReleaseOutcome",
    "SpendGuardClient",
    # errors
    "ApprovalBundleHotReloadedError",
    "ApprovalDeniedError",
    "ApprovalLapsedError",
    "ApprovalRequired",
    "DecisionDenied",
    "DecisionSkipped",
    "DecisionStopped",
    "HandshakeError",
    "MutationApplyFailed",
    "SidecarUnavailable",
    "SpendGuardConfigError",
    "SpendGuardError",
    # ids
    "default_call_signature",
    "derive_idempotency_key",
    "derive_uuid_from_signature",
    "new_uuid7",
    "workload_instance_id",
    # run plan (Signal 3, SLICE_12)
    "RunPlan",
    "current_run_plan",
    "with_run_plan",
    # session reservation substrate (D41S_03)
    "DEFAULT_MAX_PENDING_SESSION_DELTAS",
    "CommitSessionDeltaOutcome",
    "CommitSessionDeltaRequest",
    "PendingSessionDelta",
    "ReleaseSessionOutcome",
    "ReleaseSessionRequest",
    "ReserveSessionAccepted",
    "ReserveSessionDenied",
    "ReserveSessionOutcome",
    "ReserveSessionRequest",
    "SessionDeltaCommitInput",
    "SessionPendingDeltaLimitError",
    "SessionReleaseInput",
    "SessionReservationHandle",
    "SessionReservationHandleError",
    "SessionReservationHandleSnapshot",
    "SessionReservationReleasedError",
    "SessionReservationReplayMismatchError",
    "build_commit_session_delta_request",
    "build_release_session_request",
    "build_reserve_session_request",
]

__version__ = "0.6.1"
