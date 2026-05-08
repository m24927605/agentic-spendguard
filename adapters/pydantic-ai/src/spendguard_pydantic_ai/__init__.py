"""SpendGuard Pydantic-AI adapter (L3 — UsageLimits hooks via sidecar UDS)."""

from .client import (
    DEFAULT_DECISION_TIMEOUT_S,
    DEFAULT_HANDSHAKE_TIMEOUT_S,
    DecisionOutcome,
    HandshakeOutcome,
    SpendGuardClient,
)
from .errors import (
    ApprovalRequired,
    DecisionDenied,
    DecisionSkipped,
    DecisionStopped,
    HandshakeError,
    MutationApplyFailed,
    SidecarUnavailable,
    SpendGuardError,
)
from .ids import (
    default_call_signature,
    derive_idempotency_key,
    derive_uuid_from_signature,
    new_uuid7,
    workload_instance_id,
)
from .model import (
    CallSignatureFn,
    ClaimEstimator,
    RunContext,
    SpendGuardModel,
    current_run_context,
    is_spendguard_skip,
    is_spendguard_terminal,
    run_context,
)

__all__ = [
    # client
    "DEFAULT_DECISION_TIMEOUT_S",
    "DEFAULT_HANDSHAKE_TIMEOUT_S",
    "DecisionOutcome",
    "HandshakeOutcome",
    "SpendGuardClient",
    # errors
    "ApprovalRequired",
    "DecisionDenied",
    "DecisionSkipped",
    "DecisionStopped",
    "HandshakeError",
    "MutationApplyFailed",
    "SidecarUnavailable",
    "SpendGuardError",
    # ids
    "default_call_signature",
    "derive_idempotency_key",
    "derive_uuid_from_signature",
    "new_uuid7",
    "workload_instance_id",
    # model
    "CallSignatureFn",
    "ClaimEstimator",
    "RunContext",
    "SpendGuardModel",
    "current_run_context",
    "is_spendguard_skip",
    "is_spendguard_terminal",
    "run_context",
]

__version__ = "0.1.0a1"
