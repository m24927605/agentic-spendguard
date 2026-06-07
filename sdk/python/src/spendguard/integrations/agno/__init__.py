"""Agno pre_hooks / post_hooks integration — gates ``Agent.arun()`` via the sidecar.

Wraps an Agno ``Agent`` with ``SpendGuardAgnoPreHook`` +
``SpendGuardAgnoPostHook`` factories so every model call routes through
the SpendGuard sidecar's ``RequestDecision(LLM_CALL_PRE)`` →
``EmitLlmCallPost(SUCCESS)`` lifecycle. The same hook pair gates every
Agno ``Model`` provider (OpenAIChat / Claude / Gemini / Groq / xAI /
DeepSeek / ...) because Agno's first-party extension surface is the
callable-based ``pre_hooks`` / ``post_hooks`` lists on ``Agent`` —
there is no per-vendor model wrapping involved.

Integration shape::

    from agno.agent import Agent
    from agno.models.openai import OpenAIChat

    from spendguard import SpendGuardClient
    from spendguard.integrations.agno import (
        RunContext, SpendGuardAgnoPreHook, SpendGuardAgnoPostHook,
        run_context,
    )
    from spendguard._proto.spendguard.common.v1 import common_pb2

    client = SpendGuardClient(socket_path=..., tenant_id=...)
    await client.connect()
    await client.handshake()

    unit = common_pb2.UnitRef(unit_id="usd_micros",
                              token_kind="output_token",
                              model_family="gpt-4")
    pricing = common_pb2.PricingFreeze(pricing_version="2026-q2")

    pre = SpendGuardAgnoPreHook(
        client=client, budget_id="...", window_instance_id="...",
        unit=unit, pricing=pricing,
    )
    post = SpendGuardAgnoPostHook(client=client, unit=unit, pricing=pricing)

    agent = Agent(
        model=OpenAIChat(id="gpt-4o-mini"),
        pre_hooks=[pre()], post_hooks=[post()],
    )

    async with run_context(RunContext(run_id="my-run-1")):
        response = await agent.arun("Say hello in three words.")

POC scope:
  - Streaming (``Agent.arun(stream=True)``) is gated at PRE only; POST
    emits after the final chunk via Agno's standard post-hook dispatch.
  - DEGRADE mutation patches are surfaced as
    ``MutationApplyFailed`` rather than applied (parity with
    pydantic_ai / langchain integrations).
  - Tool-call hooks (``tool_hooks``) are NOT covered by D22; future
    deliverable (D22.1).

DEVIATIONS vs ``docs/specs/coverage/D22_agno/design.md`` (locked):
  - DEVIATION-1: spec §6.5 said "STOP / DENY raises DecisionDenied,
    Agno propagates the exception". Reality: Agno 2.x's hook loop
    catches ``Exception`` and only re-propagates
    ``InputCheckError`` / ``OutputCheckError``. Without the wrap a DENY
    would be logged + the model would still be called, violating
    review-standards §3 "PRE before vendor SDK". The pre-hook wraps
    ``DecisionDenied`` into ``InputCheckError`` with the original
    chained via ``__cause__``. Documented in ``_hook.py`` docstring.
  - DEVIATION-2: spec §6.9 said the post-hook async function declares
    ``(agent, run_response)``. Reality: Agno 2.x ``aexecute_post_hooks``
    builds its ``all_args`` dict with the key ``"run_output"`` (see
    ``agno/agent/_hooks.py:281``). ``filter_hook_args`` drops any
    parameter the closure declares that isn't in ``all_args``, so a
    closure declaring ``run_response`` would receive an empty kwargs
    set and the post would never run. The closure follows reality and
    declares ``run_output``. Tests assert the literal parameter name.

Both deviations were forced by Agno 2.x crystallising the
public-surface contract after the spec was authored; the spec's
``>=1.0,<2.0`` cap is widened to ``>=2.0,<3.0`` in ``pyproject.toml``
because ``pre_hooks`` / ``post_hooks`` only ship in the 2.x line.
"""

from __future__ import annotations

# Import-time guard: surface a helpful error pointing at the
# ``pip install 'spendguard-sdk[agno]'`` install command when the user
# imports this module without the ``agno`` package installed. The
# guard fires once at module load; the hook classes themselves accept
# duck-typed payloads in tests so the unit suite still runs.
try:
    from agno.agent import Agent  # noqa: F401
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.agno requires the [agno] extra. "
        "Install with: pip install 'spendguard-sdk[agno]'"
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
    CallSignatureFn,
    ClaimEstimator,
    SpendGuardAgnoPostHook,
    SpendGuardAgnoPreHook,
    current_run_context,
    run_context,
)
from ._options import RunContext, SpendGuardAgnoOptions

__all__ = [
    # Public surface — LOCKED per design.md §4 / review-standards §6.
    "CallSignatureFn",
    "ClaimEstimator",
    "RunContext",
    "SpendGuardAgnoOptions",
    "SpendGuardAgnoPostHook",
    "SpendGuardAgnoPreHook",
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
