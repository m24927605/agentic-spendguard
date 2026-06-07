"""Google ADK integration — gates ``LlmAgent`` model calls via the sidecar.

Wraps Google's Agent Development Kit (``google-adk`` ≥ 1.0) with a
``before_model_callback`` / ``after_model_callback`` pair that routes
every LLM turn through the SpendGuard sidecar's RequestDecision →
EmitLlmCallPost lifecycle.

Integration shape::

    from google.adk.agents import LlmAgent
    from google.adk.runners import InMemoryRunner

    from spendguard import SpendGuardClient
    from spendguard.integrations.adk import SpendGuardAdkCallback
    from spendguard._proto.spendguard.common.v1 import common_pb2

    client = SpendGuardClient(socket_path=..., tenant_id=...)
    await client.connect()
    await client.handshake()

    cb = SpendGuardAdkCallback(
        client=client,
        budget_id="my-budget",
        window_instance_id="my-window",
        unit=common_pb2.UnitRef(
            unit_id="usd_micros",
            token_kind="output_token",
            model_family="gemini-2.0-flash",
        ),
        pricing=common_pb2.PricingFreeze(...),
    )
    agent = LlmAgent(
        name="my-agent",
        model="gemini-2.0-flash",
        instructions="You are a budget-aware assistant.",
        before_model_callback=cb,
        after_model_callback=cb,
    )
    runner = InMemoryRunner(agent=agent)
    await runner.run_async(user_message=...)

POC scope:
  - Vendor coverage: Gemini direct + Vertex-backed Gemini +
    ``LiteLlm("openai/...")`` wrappers (usage extraction by shape).
  - Streaming intra-turn gating: not supported. Gating is at the turn
    boundary (parity with LangChain / openai-agents priors).
  - Tool callbacks: out of scope. Spend gating sits at the model
    boundary.
  - Deny path is the documented ADK short-circuit channel: a synthetic
    ``LlmResponse(error_code="SPENDGUARD_DENY", ...)`` returned by
    ``before_model_callback``. Raising is intentionally avoided.
"""

from __future__ import annotations

# Import-time guard: surface a helpful error pointing at the
# ``pip install 'spendguard-sdk[adk]'`` install command when the
# user imports this module without the extras installed. The
# guard fires once at module load; the callback class itself is
# import-resilient (it accepts duck-typed payloads in tests).
try:
    from google.adk.agents.callback_context import CallbackContext  # noqa: F401
    from google.adk.models import LlmRequest, LlmResponse  # noqa: F401
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.adk requires the [adk] extra. "
        "Install with: pip install 'spendguard-sdk[adk]'"
    ) from exc

from ._callback import (
    ClaimEstimator,
    RunIdFn,
    SpendGuardAdkCallback,
)
from ._errors import (
    DecisionDenied,
    DecisionSkipped,
    DecisionStopped,
    HandshakeError,
    SidecarUnavailable,
    SpendGuardConfigError,
    SpendGuardError,
)
from ._options import SpendGuardAdkOptions

__all__ = [
    # Primary callback class
    "SpendGuardAdkCallback",
    # Type aliases for advanced configuration
    "ClaimEstimator",
    "RunIdFn",
    # Optional POCO config
    "SpendGuardAdkOptions",
    # Error re-exports (catch-from-one-place)
    "DecisionDenied",
    "DecisionSkipped",
    "DecisionStopped",
    "HandshakeError",
    "SidecarUnavailable",
    "SpendGuardConfigError",
    "SpendGuardError",
]
