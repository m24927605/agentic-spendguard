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
]

__version__ = "0.5.0"
