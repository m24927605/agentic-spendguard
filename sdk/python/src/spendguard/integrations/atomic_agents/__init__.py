"""Atomic Agents (BrainBlend AI) + Instructor (Jason Liu) wrap adapter.

Atomic Agents is a Pydantic-first agent framework built on Instructor.
``BaseAgent`` is constructed via ``BaseAgentConfig(client=<instructor_client>,
model=..., input_schema=..., output_schema=...)``. At run time
``agent.run(...)`` calls
``self.client.chat.completions.create_with_completion(response_model=output_schema, ...)``
on the wrapped ``Instructor`` / ``AsyncInstructor`` object.

Atomic Agents has **no first-class LLM-call middleware**. The only
surface that observes every call — including Instructor's
**validation-retry loop** (a Pydantic ValidationError on the parsed
response triggers a fresh provider HTTP call with the validation error
injected into ``messages``) — is the Instructor object itself.

We therefore wrap the Instructor object via composition, NOT the raw
provider SDK (``openai.OpenAI`` / ``anthropic.Anthropic``). Wrapping
the raw SDK would silently undercount Instructor's retries because
Instructor calls its patched ``.chat.completions.create*`` method
internally, not the raw transport.

  - sync   ``instructor.Instructor``      → ``SpendGuardInstructorProxy``
  - async  ``instructor.AsyncInstructor`` → ``SpendGuardAsyncInstructorProxy``

Both proxies override ``.chat.completions.create`` and
``.chat.completions.create_with_completion``. Every other attribute
passes through via ``__getattr__`` so ``proxy.mode`` /
``proxy.create_kwargs`` / any future Instructor attribute remains
reachable without code change.

Install with::

    pip install 'spendguard-sdk[atomic-agents]'
    # transitively pulls atomic-agents>=2.0,<3 + instructor>=1.5,<2.0

Integration shape::

    import instructor
    from openai import OpenAI
    from atomic_agents.agents.base_agent import BaseAgent, BaseAgentConfig
    from pydantic import BaseModel

    from spendguard import SpendGuardClient
    from spendguard.integrations.atomic_agents import wrap_instructor_client
    from spendguard.integrations.openai_agents import RunContext, run_context
    from spendguard._proto.spendguard.common.v1 import common_pb2

    client = SpendGuardClient(socket_path=..., tenant_id=...)
    await client.connect()
    await client.handshake()

    unit = common_pb2.UnitRef(unit_id="usd_micros",
                              token_kind="output_token",
                              model_family="gpt-4")
    pricing = common_pb2.PricingFreeze(pricing_version="2026-q2")

    class Answer(BaseModel):
        final: str

    raw_instructor = instructor.from_openai(OpenAI())
    guarded = wrap_instructor_client(
        raw_instructor,
        spendguard_client=client,
        budget_id="...",
        window_instance_id="...",
        unit=unit,
        pricing=pricing,
        claim_estimator=lambda kwargs: [common_pb2.BudgetClaim(...)],
    )

    agent = BaseAgent(BaseAgentConfig(
        client=guarded, model="gpt-4o-mini",
        system_prompt_generator=..., input_schema=..., output_schema=Answer,
    ))

    async with run_context(RunContext(run_id="my-run-1")):
        result = agent.run({"query": "What's 2+2?"})

DEVIATIONS vs ``docs/specs/coverage/D28_atomic_agents/design.md`` (locked):
  - DEVIATION-A: design.md §8.2 pinned ``atomic-agents>=1.0,<2.0``.
    Reality (2026-06-08): the actual PyPI release line is 2.x with
    ``2.8.0`` as the latest; there is no published 1.x line under the
    current ``atomic-agents`` package name. We pin ``>=2.0,<3`` in
    ``pyproject.toml`` so the extra:
      1. Fail-closes against a future breaking-change major (3.x line),
      2. Floors at the version where ``BaseAgent`` /
         ``BaseAgentConfig(client=<instructor>)`` are GA — verified
         against ``atomic-agents==2.8.0`` from PyPI.
  - DEVIATION-B: design.md §4 / implementation.md §1 specified a
    single ``atomic_agents.py`` flat module. We split into a
    ``atomic_agents/`` subpackage (``__init__``, ``_errors``,
    ``_options``, ``_hook``) mirroring the autogen / beeai / dspy
    layout so the import-time guard fires cleanly on a missing extra
    while ``_hook`` stays directly importable for tests that bypass
    the barrel.

POC scope:
  - Streaming (``instructor.Partial[...]`` / ``Iterable[...]``) is
    OUT — design.md §3 non-goal; commit only after the final parsed
    response via Instructor's standard ``create_with_completion`` path.
  - Wrapping ``client.messages.create`` (Anthropic-native Instructor
    surface) is OUT — Atomic Agents documents ``chat.completions``.
  - DENY raises ``DecisionDenied`` directly out of the proxy's
    overridden method. Atomic Agents' ``BaseAgent.run`` has no
    framework-side catch on the create-call path (verified against
    ``atomic-agents==2.8.0``), so the raise reaches the caller cleanly.
  - Fail-closed is the only mode (review-standards §6). No
    ``SPENDGUARD_ATOMIC_AGENTS_FAIL_OPEN`` env knob exists.
  - Each Instructor validation retry mutates ``messages`` (validation
    error injected) → ``_signature(kwargs)`` naturally diverges →
    fresh ``llm_call_id`` → fresh reservation. Tested behavior, not
    a configuration knob (per review-standards §2.2).

Module layout:
  - ``__init__.py`` (this file) — public surface, import-time guard.
  - ``_errors.py`` — error re-exports.
  - ``_options.py`` — ``SpendGuardAtomicAgentsOptions`` POCO.
  - ``_hook.py`` — ``wrap_instructor_client`` factory + proxies.
"""

from __future__ import annotations

# Import-time guard: surface a helpful error pointing at the
# ``pip install 'spendguard-sdk[atomic-agents]'`` install command when
# the user imports this module without ``instructor`` installed.
# The wrapper class itself is import-resilient (the ``_hook`` module
# falls back to a duck-typed branch so unit tests still run).
try:
    import instructor  # noqa: F401
    from instructor import AsyncInstructor, Instructor  # noqa: F401
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.atomic_agents requires the [atomic-agents] "
        "extra. Install with: pip install 'spendguard-sdk[atomic-agents]' "
        "(transitively pulls atomic-agents>=2.0,<3 + instructor>=1.5,<2.0)."
    ) from exc

try:
    # Atomic Agents itself is not imported by ``_hook`` (we wrap the
    # Instructor object, not a BaseAgent type), but we surface a
    # friendly install hint here since this adapter is named for it.
    import atomic_agents  # noqa: F401
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.atomic_agents requires atomic-agents "
        "installed. Install with: pip install 'spendguard-sdk[atomic-agents]' "
        "(pulls atomic-agents>=2.0,<3 + instructor>=1.5,<2.0)."
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
    SpendGuardAsyncInstructorProxy,
    SpendGuardInstructorProxy,
    current_run_context,
    run_context,
    wrap_instructor_client,
)
from ._options import SpendGuardAtomicAgentsOptions

__all__ = [
    # Public surface — LOCKED per design.md §7 / review-standards §1.
    "ClaimEstimator",
    "RunContext",
    "SpendGuardAsyncInstructorProxy",
    "SpendGuardAtomicAgentsOptions",
    "SpendGuardInstructorProxy",
    "current_run_context",
    "run_context",
    "wrap_instructor_client",
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
