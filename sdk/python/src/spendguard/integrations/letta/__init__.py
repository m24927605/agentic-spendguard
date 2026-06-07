"""Letta (ex-MemGPT) ``LLMClientBase`` wrap adapter.

Letta (formerly MemGPT, ~22k stars, Apache-2.0) is a stateful agent
platform with persistent memory. Every LLM call inside ``Agent.step()``
flows through a per-provider concrete subclass of
``letta.llm_api.llm_client_base.LLMClientBase`` —
``OpenAIClient`` / ``AnthropicClient`` / ``GoogleAIClient`` /
``DeepSeekClient``. There is **no formal pre-LLM middleware bus**:
``step_callback`` fires once per agent turn even though a turn frequently
fans out into 3-4 internal LLM calls (reasoning → tool select →
reflection), so step-level gating over-grants reservations.

The only safe surface that observes every LLM call is **subclassing
``LLMClientBase`` and overriding ``send_llm_request`` /
``send_llm_request_sync``**. SpendGuard wraps any ``LLMClientBase``
instance via composition: PRE / POST gating around both the async and
sync hot paths, identical semantics to
``SpendGuardChatCompletionClient`` (D24) and
``SpendGuardAgentsModel`` (D08).

When to use what:

- **Embedded Letta library** (in-process ``Agent.step()``) → use D26
  wrap (this module).
- **Self-hosted ``letta server``** REST surface → use D02 closed-CLI
  install + D03 base-URL drop-in. D26 is unnecessary and ignored on
  the server-side path.
- **LiteLLM-routed** (any Letta deployment whose inner provider is
  ``LiteLLMClient``) → D12 LiteLLM SDK shim covers transitively. No
  D26 work needed; the two adapters coexist safely.

Install with::

    pip install 'spendguard-sdk[letta]'
    pip install 'letta>=0.8,<1.0'

Integration shape::

    from letta.llm_api.openai_client import OpenAIClient

    from spendguard import SpendGuardClient
    from spendguard.integrations.letta import (
        SpendGuardLettaClient, wrap_llm_client,
        RunContext, run_context,
    )
    from spendguard._proto.spendguard.common.v1 import common_pb2

    client = SpendGuardClient(socket_path=..., tenant_id=...)
    await client.connect()
    await client.handshake()

    unit = common_pb2.UnitRef(unit_id="usd_micros",
                              token_kind="output_token",
                              model_family="gpt-4")
    pricing = common_pb2.PricingFreeze(pricing_version="2026-q2")

    inner = OpenAIClient(...)
    guarded = wrap_llm_client(
        inner=inner,
        client=client,
        budget_id="...",
        window_instance_id="...",
        unit=unit,
        pricing=pricing,
        claim_estimator=lambda req: [common_pb2.BudgetClaim(...)],
    )

    agent = letta_agent_factory(llm_client=guarded, ...)
    async with run_context(RunContext(run_id="my-run-1")):
        response = await agent.step(message)

POC scope:
  - ``CancelledError`` propagates as ``outcome=CANCELLED`` in POST
    (matches D12 / D24 pattern via ``type(exc).__name__`` detection —
    avoids cross-loop ``isinstance`` mismatches across asyncio /
    trio / anyio).
  - DENY raises ``DecisionDenied`` directly. ``LLMClientBase`` has no
    framework-side catch on the ``send_llm_request`` path (verified
    against letta 0.8.0), so the raise reaches the ``Agent.step``
    caller cleanly. No DEVIATION-style wrap is needed.
  - Fail-closed is the only mode (review-standards §6). No
    ``SPENDGUARD_LETTA_FAIL_OPEN`` env knob exists.
  - ``send_llm_request_sync`` detects an active asyncio loop via
    ``asyncio.get_running_loop()`` and raises ``RuntimeError`` with a
    message pointing at the async variant. No silent ``asyncio.run()``
    re-entry (review-standards §3.1).
  - ``__getattr__`` delegates ``llm_config`` / ``provider`` /
    ``build_request_data`` / ``convert_response_to_chat_completion``
    / any future ``LLMClientBase`` additions to the inner client. No
    side effects in the pass-through path (review-standards §1.3).
"""

from __future__ import annotations

# Import-time guard: surface a helpful error pointing at the
# ``pip install 'spendguard-sdk[letta]'`` install command when the user
# imports this module without ``letta`` installed. The wrapper class
# itself is import-resilient (the ``_hook`` module falls back to a
# plain base class in unit-test environments) so the unit suite still
# runs even when the extra is missing.
try:
    from letta.llm_api.llm_client_base import (  # type: ignore[import-not-found] # noqa: F401
        LLMClientBase,
    )
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.letta requires the [letta] extra. "
        "Install with: pip install 'spendguard-sdk[letta]' "
        "'letta>=0.8,<1.0'."
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
    SpendGuardLettaClient,
    current_run_context,
    run_context,
    wrap_llm_client,
)
from ._options import SpendGuardLettaOptions

__all__ = [
    # Public surface — LOCKED per design.md §7 / review-standards §1.
    "ClaimEstimator",
    "RunContext",
    "SpendGuardLettaClient",
    "SpendGuardLettaOptions",
    "current_run_context",
    "run_context",
    "wrap_llm_client",
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
