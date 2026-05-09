"""SpendGuard SDK — runtime safety layer client for AI agent frameworks.

Core surface (always available):

    from spendguard import SpendGuardClient, DecisionStopped, derive_idempotency_key

Framework integrations are optional (install via extras):

    pip install spendguard-sdk[pydantic-ai]
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
]

__version__ = "0.1.0a1"
