"""SmolAgents ``Model.generate`` wrap + ``step_callbacks`` informational adapter.

SmolAgents (HuggingFace, Apache-2.0, ~15k stars) exposes a pluggable
``smolagents.Model`` ABC. Vendor subclasses — ``InferenceClientModel``,
``OpenAIServerModel`` (vLLM / Ollama / Together / Groq / OpenAI-
compatible), ``TransformersModel`` — all route every ``CodeAgent`` /
``ToolCallingAgent`` invocation through one
``Model.generate(messages, ...) -> ChatMessage`` entry point.
SpendGuard subclasses this single ABC and wraps an inner instance with
``generate()`` PRE-before-HTTP / POST-after-HTTP gating. ``__call__``
is aliased to ``generate`` for ``smolagents<1.5`` compatibility.

LiteLLMModel users are covered transitively by the D12 SDK shim — see
``docs/integrations/litellm-sdk-shim``. The wrapper REFUSES to wrap a
``LiteLLMModel`` directly (would double-gate; review-standards §1.1).

Install with::

    pip install 'spendguard-sdk[smolagents]'

Integration shape::

    from smolagents import CodeAgent, OpenAIServerModel

    from spendguard import SpendGuardClient
    from spendguard.integrations.smolagents import (
        SpendGuardSmolModel, spendguard_step_callback,
        RunContext, run_context,
    )
    from spendguard._proto.spendguard.common.v1 import common_pb2

    client = SpendGuardClient(socket_path=..., tenant_id=...)
    await client.connect(); await client.handshake()

    unit = common_pb2.UnitRef(unit_id="usd_micros",
                              token_kind="output_token",
                              model_family="gpt-4")
    pricing = common_pb2.PricingFreeze(pricing_version="2026-q2")

    guarded = SpendGuardSmolModel(
        inner=OpenAIServerModel(model_id="gpt-4o-mini",
                                api_base=..., api_key=...),
        client=client, budget_id="...", window_instance_id="...",
        unit=unit, pricing=pricing,
        claim_estimator=lambda messages: [common_pb2.BudgetClaim(...)],
    )

    agent = CodeAgent(
        model=guarded, tools=[],
        step_callbacks=[spendguard_step_callback(client, run_id="my-run-1")],
    )

    async with run_context(RunContext(run_id="my-run-1")):
        # CodeAgent.run is sync — driven inside the async run_context
        # via thread executor or by pre-binding the contextvar via an
        # outer async scope:
        result = agent.run("solve 2+2")

POC scope:
  - Gating brackets each ``generate()`` call at the model boundary;
    intra-step tool calls inherit the parent reservation. Per-chunk
    streaming gating tracked as follow-on.
  - ``TransformersModel`` GPU-second cost accounting is out of scope —
    token-count POST estimation only (``ChatMessage.token_usage``).
  - ``step_callbacks`` are NOT a gating surface. They fire AFTER each
    step completes; ``spendguard_step_callback`` is informational
    telemetry only. The wrapper is the gating surface.
  - DENY raises ``DecisionDenied`` directly. ``MultiStepAgent.step``
    has no framework-side catch on the ``model.generate()`` path
    (verified against smolagents 1.26), so the raise reaches the
    ``CodeAgent.run`` caller cleanly.
  - Fail-closed is the only mode (review-standards §7). No
    ``SPENDGUARD_SMOLAGENTS_FAIL_OPEN`` env knob exists.

D12 (LiteLLM SDK shim) covers the LiteLLM-routed case transitively
when the inner Model is ``smolagents.LiteLLMModel``; the two
integrations would coexist unsafely (double-gate), so the wrapper
explicitly REFUSES that combination at construction time.
"""

from __future__ import annotations

# Import-time guard: surface a helpful error pointing at the
# ``pip install 'spendguard-sdk[smolagents]'`` install command when the
# user imports this module without ``smolagents`` installed. The guard
# fires once at module load; the wrapper class itself is import-resilient
# (the ``_hook`` module falls back to a plain base class in unit-test
# environments) so the unit suite still runs.
try:
    from smolagents import Model  # type: ignore[attr-defined] # noqa: F401
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.smolagents requires the [smolagents] extra. "
        "Install with: pip install 'spendguard-sdk[smolagents]'"
    ) from exc

from ._errors import (
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
from ._hook import (
    ClaimEstimator,
    RunContext,
    SpendGuardSmolModel,
    SyncInAsyncContext,
    current_run_context,
    run_context,
    spendguard_step_callback,
)
from ._options import SpendGuardSmolAgentsOptions

__all__ = [
    # Public surface — LOCKED per design.md §7 / review-standards §1.
    "ClaimEstimator",
    "RunContext",
    "SpendGuardSmolAgentsOptions",
    "SpendGuardSmolModel",
    "SyncInAsyncContext",
    "current_run_context",
    "run_context",
    "spendguard_step_callback",
    # Error re-exports (catch-from-one-place).
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
