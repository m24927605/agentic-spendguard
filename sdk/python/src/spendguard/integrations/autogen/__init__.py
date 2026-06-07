"""AutoGen 0.4+ / AG2 ``ChatCompletionClient`` wrap adapter.

Both AutoGen 0.4+ (Microsoft, maintenance mode as of 2026-02) and AG2
(community fork led by ex-AutoGen maintainers, ~48k stars, Apache-2.0)
share ``autogen_core.models.ChatCompletionClient`` as the LLM
abstraction — AG2 vendored the namespace unchanged through at least
0.7.x. SpendGuard subclasses this single ABC and wraps an inner client
with ``create()`` / ``create_stream()`` PRE-before-HTTP /
POST-after-HTTP gating. One module covers both lineages.

Install with::

    pip install 'spendguard-sdk[autogen]'                  # base
    pip install autogen-agentchat>=0.4 autogen-ext[openai] # AutoGen lineage
    # OR
    pip install ag2>=0.7                                   # AG2 lineage

Integration shape::

    from autogen_ext.models.openai import OpenAIChatCompletionClient
    from autogen_agentchat.agents import AssistantAgent  # OR ag2.agents

    from spendguard import SpendGuardClient
    from spendguard.integrations.autogen import (
        SpendGuardChatCompletionClient, RunContext, run_context,
    )
    from spendguard._proto.spendguard.common.v1 import common_pb2

    client = SpendGuardClient(socket_path=..., tenant_id=...)
    await client.connect()
    await client.handshake()

    unit = common_pb2.UnitRef(unit_id="usd_micros",
                              token_kind="output_token",
                              model_family="gpt-4")
    pricing = common_pb2.PricingFreeze(pricing_version="2026-q2")

    guarded = SpendGuardChatCompletionClient(
        inner=OpenAIChatCompletionClient(model="gpt-4o-mini"),
        client=client,
        budget_id="...",
        window_instance_id="...",
        unit=unit,
        pricing=pricing,
        claim_estimator=lambda messages: [common_pb2.BudgetClaim(...)],
    )

    agent = AssistantAgent(name="x", model_client=guarded)

    async with run_context(RunContext(run_id="my-run-1")):
        result = await agent.on_messages([...], cancellation_token)

The ``LINEAGE`` constant tells you which lineage(s) are installed
alongside ``autogen-core``: ``"autogen"`` / ``"ag2"`` / ``"both"`` /
``"core-only"``. Per review-standards §1.1 this is telemetry only —
the wrapper's ``create()`` / ``create_stream()`` business logic NEVER
branches on it. The same wrapper instance works against either lineage.

POC scope:
  - Stream gating brackets the WHOLE stream at the model boundary;
    intra-stream tool calls inherit the parent reservation.
    Per-chunk gating tracked as follow-on.
  - ``CancelledError`` propagates as ``outcome=CANCELLED`` in POST
    (matches D12 LiteLLM shim pattern via ``type(exc).__name__``
    detection — avoids cross-loop ``isinstance`` mismatches across
    asyncio / trio / anyio).
  - DENY raises ``DecisionDenied`` directly. ``ChatCompletionClient``
    has no framework-side catch on the create() path in either
    lineage (verified against autogen-core 0.4.0 and ag2 0.7.0), so
    no DEVIATION-style wrap is needed — the raise reaches the
    ``AssistantAgent`` caller cleanly.
  - Fail-closed is the only mode (review-standards §6). No
    ``SPENDGUARD_AUTOGEN_FAIL_OPEN`` env knob exists.

D12 (LiteLLM SDK shim) covers the LiteLLM-routed case transitively
when the inner client is ``autogen_ext.models.litellm.LiteLLMChatCompletionClient``;
the two integrations coexist safely (D24 reserves at the
ChatCompletionClient layer, D12 short-circuits at the LiteLLM layer
when its in-flight contextvar is set — but the contextvar is only set
by D12's own monkey-patch, not by D24).
"""

from __future__ import annotations

# Import-time guard: surface a helpful error pointing at the
# ``pip install 'spendguard-sdk[autogen]'`` install command when the user
# imports this module without ``autogen-core`` installed. The guard
# fires once at module load; the wrapper class itself is import-resilient
# (the ``_hook`` module falls back to a plain base class in unit-test
# environments) so the unit suite still runs.
try:
    from autogen_core.models import (  # type: ignore[import-not-found] # noqa: F401
        ChatCompletionClient,
    )
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.autogen requires the [autogen] extra. "
        "Install with: pip install 'spendguard-sdk[autogen]' AND one of "
        "`autogen-agentchat>=0.4` (Microsoft) or `ag2>=0.7` (community)."
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
    LINEAGE,
    ClaimEstimator,
    RunContext,
    SpendGuardChatCompletionClient,
    current_run_context,
    run_context,
)
from ._options import SpendGuardAutoGenOptions

__all__ = [
    # Public surface — LOCKED per design.md §7 / review-standards §1.
    "ClaimEstimator",
    "LINEAGE",
    "RunContext",
    "SpendGuardAutoGenOptions",
    "SpendGuardChatCompletionClient",
    "current_run_context",
    "run_context",
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
