"""AWS Strands Agents SDK integration — gates ``Agent`` invocations via the sidecar.

Wraps Amazon's first-party Agent SDK (``strands-agents`` >= 1.0) with a
``HookProvider`` that registers ``before_invocation`` / ``after_invocation``
callbacks on Strands' typed event bus. Each ``agent.invoke_async()``
turn routes through the SpendGuard sidecar's RequestDecision →
EmitLlmCallPost lifecycle.

Coverage is enforced at the agent-runtime boundary, not the model
boundary, so the SAME provider instance gates every Strands model
backend — Bedrock (default in AWS shops), OpenAI, Anthropic, Gemini,
Ollama, and LiteLLM — with one registration.

Integration shape::

    from strands import Agent
    from strands.models.bedrock import BedrockModel

    from spendguard import SpendGuardClient
    from spendguard.integrations.strands import (
        SpendGuardStrandsHookProvider,
    )
    from spendguard._proto.spendguard.common.v1 import common_pb2

    client = SpendGuardClient(socket_path=..., tenant_id=...)
    await client.connect()
    await client.handshake()

    def reconcile(invocation, result):
        usage = result.usage
        total = (usage.total_tokens
                 or (usage.input_tokens + usage.output_tokens))
        return [common_pb2.BudgetClaim(
            budget_id="...", unit=unit, amount_atomic=str(total),
            direction=common_pb2.BudgetClaim.DEBIT,
            window_instance_id="...")]

    guard = SpendGuardStrandsHookProvider(
        client=client,
        budget_id="...",
        window_instance_id="...",
        unit=common_pb2.UnitRef(
            unit_id="usd_micros",
            token_kind="output_token",
            model_family="anthropic.claude-3-5-sonnet"),
        pricing=common_pb2.PricingFreeze(pricing_version="2026-q2"),
        claim_estimator=my_estimator,
        claim_reconciler=reconcile,
    )

    agent = Agent(
        model=BedrockModel(
            model_id="anthropic.claude-3-5-sonnet-20241022-v2:0"),
        hooks=[guard],
    )
    result = await agent.invoke_async(prompt="Hello")

POC scope:
  - Vendor coverage: Bedrock + OpenAI + Anthropic + Gemini + Ollama +
    LiteLLM (every Strands ``Model`` backend) — usage extraction reads
    ``result.usage`` field shape, not a model-string match.
  - Streaming intra-invocation gating: not supported. Gating is at
    invocation boundary (parity with LangChain / openai-agents priors).
  - Per-tool budgets via ``before_tool`` / ``after_tool``: out of
    scope; deferred to D20.1. Tool cost bundled into parent invocation.
  - Deny path raises ``DecisionDenied`` directly; Strands wraps to
    ``HookExecutionError`` and caller catches via ``__cause__`` chain.
  - DEGRADE fail-closed by default; ``SPENDGUARD_STRANDS_FAIL_OPEN=1``
    allows the call (dev only — no commit row will be produced).
"""

from __future__ import annotations

# Import-time guard: surface a helpful error pointing at the
# ``pip install 'spendguard-sdk[strands]'`` install command when the
# user imports this module without the extras installed. The guard
# fires once at module load; the provider class itself is
# import-resilient (it accepts duck-typed payloads in tests).
try:
    from strands.hooks import (  # noqa: F401
        AfterInvocationEvent,
        BeforeInvocationEvent,
        HookProvider,
    )
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.strands requires the [strands] extra. "
        "Install with: pip install 'spendguard-sdk[strands]'"
    ) from exc

from ._errors import (
    ApprovalRequired,
    DecisionDenied,
    DecisionSkipped,
    DecisionStopped,
    HandshakeError,
    SidecarUnavailable,
    SpendGuardConfigError,
    SpendGuardDegradeBlocked,
    SpendGuardError,
)
from ._hook_provider import (
    ClaimEstimator,
    ClaimReconciler,
    SpendGuardStrandsHookProvider,
    current_run_context,
    run_context,
)
from ._options import (
    SpendGuardStrandsOptions,
    StrandsRunContext,
)

__all__ = [
    # Primary provider class
    "SpendGuardStrandsHookProvider",
    # Type aliases for advanced configuration
    "ClaimEstimator",
    "ClaimReconciler",
    # Optional POCO config + run context
    "SpendGuardStrandsOptions",
    "StrandsRunContext",
    "current_run_context",
    "run_context",
    # Error re-exports (catch-from-one-place)
    "ApprovalRequired",
    "DecisionDenied",
    "DecisionSkipped",
    "DecisionStopped",
    "HandshakeError",
    "SidecarUnavailable",
    "SpendGuardConfigError",
    "SpendGuardDegradeBlocked",
    "SpendGuardError",
]
