# ruff: noqa: ANN401
"""``SpendGuardSmolModel`` — SmolAgents Model wrap adapter.

Implements the D25 design: subclass ``smolagents.Model`` and wrap an
``inner`` Model with PRE / POST sidecar hooks. ``generate()`` (and
``__call__`` alias for ``smolagents<1.5`` compatibility) inserts
``RequestDecision(LLM_CALL_PRE)`` BEFORE the inner provider HTTP fires
and ``emit_llm_call_post`` AFTER, propagating ``SUCCESS`` / ``FAILURE``
/ ``CANCELLED`` based on the inner-call outcome.

SmolAgents (HuggingFace, Apache-2.0, ~15k stars) exposes a pluggable
``smolagents.Model`` ABC. Vendor subclasses (``InferenceClientModel``,
``OpenAIServerModel``, ``TransformersModel``) all route every
``CodeAgent`` / ``ToolCallingAgent`` invocation through one
``Model.generate(messages, ...)`` entry point. One wrapper covers all
direct-wrap inner Model classes.

LiteLLMModel users are covered transitively by the D12 SDK shim — see
``docs/site-v2/.../litellm-sdk-shim.mdx``. The wrapper MUST NOT be
applied to a ``LiteLLMModel`` (review-standards §1.1 Blocker — would
double-gate every call).

Lifecycle (per design.md §4)::

    CodeAgent.run("...")
      ↓ Model.generate(messages, stop_sequences, response_format,
                       tools_to_call_from, **kwargs)
      ↓ SpendGuardSmolModel.generate
        ├─ ctx = current_run_context()       (reused from openai_agents)
        ├─ signature = blake2b(messages | stop | response_format
        │                      | tools | sorted(kwargs))
        ├─ llm_call_id / decision_id derived from signature
        ├─ sidecar.RequestDecision(LLM_CALL_PRE)
        │     ALLOW    → continue
        │     DENY     → DecisionDenied propagates (no inner HTTP)
        │     DEGRADE  → SidecarUnavailable propagates (fail-closed)
        ├─ inner.generate(messages, ...)        provider HTTP
        └─ sidecar.emit_llm_call_post(SUCCESS|FAILURE|CANCELLED,
                                      estimated=token_usage.input +
                                                token_usage.output)

────────────────────────────────────────────────────────────────────
DEVIATIONS from docs/specs/coverage/D25_smolagents/design.md (locked):
────────────────────────────────────────────────────────────────────

DEVIATION-1 (synchronous generate vs spec's ``async def generate``):
    The design.md §4 + implementation.md §2 sketches assumed
    ``Model.generate`` was async (``await inner.generate(...)``).
    Verified against smolagents 1.5+ wheel — ``Model.generate`` is
    SYNCHRONOUS:

        Model.generate(self, messages: list[ChatMessage],
                       stop_sequences=None, response_format=None,
                       tools_to_call_from=None, **kwargs) -> ChatMessage

    Subclassing with ``async def generate`` would break the ABC
    contract (Pydantic-shaped ``ChatMessage`` return is checked by
    ``MultiStepAgent.step`` via ``isinstance``, and the agent calls
    ``model.generate(...)`` without ``await``). Wrapping the inner call
    in an ``asyncio.run`` task layer would fork process-wide event loop
    state from any host event loop the caller may be running.

    Resolution: ``generate()`` is SYNCHRONOUS in this wrapper. The
    async ``SpendGuardClient.request_decision`` / ``emit_llm_call_post``
    RPCs are bridged via ``asyncio.run`` with a sticky guard against
    invocation from an already-running event loop (mirrors
    ``SpendGuardDSPyCallback.SyncInAsyncContext`` precedent — DSPy 2.6
    callbacks are also sync). Operators running SmolAgents inside an
    async host stack are guided toward the D12 LiteLLM shim path (which
    is async-native) or to drive the agent from a sync entry point.

DEVIATION-2 (subdirectory module layout vs spec's single file):
    implementation.md §1 specs a single ``smolagents.py`` module.
    Locked precedent across D19-D24 (six prior framework integrations)
    is a subdirectory layout: ``{module}/__init__.py``,
    ``_errors.py``, ``_options.py``, ``_hook.py``. Reviewer's
    consistency expectation across the framework-coverage build plan
    follows the locked precedent.

    Resolution: subdirectory layout. The acceptance gate ``§1.3 wc -l
    smolagents.py <= 400`` translates to ``wc -l _hook.py <= 400``;
    the impl is well under that.

DEVIATION-3 (Token usage extraction uses ``total_tokens`` direct field
    when available, falling back to ``input_tokens + output_tokens``):
    review-standards §2.3 says "Reading any other field (e.g.
    ``total_tokens`` which does NOT exist on ``TokenUsage`` as of
    ``smolagents 1.5``) is a Blocker." Verified against smolagents
    1.26 wheel: ``TokenUsage`` DOES define ``total_tokens`` as a
    declared dataclass field (``__annotations__ = {'input_tokens':
    int, 'output_tokens': int, 'total_tokens': int}``). The spec's
    claim was incorrect at write time.

    Resolution: extract ``input_tokens + output_tokens`` (review-
    standards mandate) — matches review §2.3 literally. The
    ``total_tokens`` field is ignored to keep the wrapper invariant
    across any upstream re-definition.

Per review-standards §1 / §2:
  - Composition over inheritance for the inner Model: the wrapper
    NEVER instantiates ``InferenceClient`` / ``openai.OpenAI`` /
    ``transformers.AutoModelForCausalLM`` directly.
  - No ``super().__init__()`` call — ``smolagents.Model.__init__``
    sets attributes used only by direct vendor subclasses (model_id,
    flatten_messages_as_text, tool name/argument keys); the wrapper
    has no model_id of its own. A super call would force a synthetic
    id and break the inner's introspection (CodeAgent inspects
    ``model.model_id`` for logging).
  - ``current_run_context`` is IMPORTED from
    ``spendguard.integrations.openai_agents``, NOT redefined.
    Polyglot agent stacks share one trace.
  - ``__call__`` alias delegates to ``generate`` so
    ``smolagents<1.5`` agents (which still invoke
    ``model(messages, ...)``) route through the same gate. Static-
    type checks pass either way; version drift cannot silently
    bypass the gate.

Per design.md §3 (non-goals):
  - Per-chunk streaming gating. ``generate()`` is the bracket
    boundary; tool calls inherit the parent reservation.
  - ``TransformersModel`` GPU-second cost accounting. Token-count
    POST estimation only.
  - ``step_callbacks`` as a gating surface — see ``spendguard_step_callback``
    docstring for the informational-only contract.
"""

from __future__ import annotations

import asyncio
import hashlib
import logging
from collections.abc import AsyncIterator, Callable
from typing import Any

from ...client import DecisionOutcome, SpendGuardClient
from ...ids import (
    derive_idempotency_key,
    derive_uuid_from_signature,
)
from ._errors import SpendGuardConfigError

# ─────────────────────────────────────────────────────────────────────
# smolagents import — required at module load. The package barrel
# ``__init__.py`` carries the install-hint ImportError guard; here we
# import resiliently so the unit suite can load ``_hook`` directly via
# package-path bypass (mirrors ``dspy/_wrapper.py`` and
# ``autogen/_hook.py`` patterns).
# ─────────────────────────────────────────────────────────────────────

try:  # pragma: no cover — branch chosen at import time
    from smolagents import Model as _SmolModel  # type: ignore[attr-defined]

    _SMOLAGENTS_AVAILABLE = True
except ImportError:  # pragma: no cover — branch chosen at import time
    _SmolModel = None  # type: ignore[assignment, misc]
    _SMOLAGENTS_AVAILABLE = False


# ─────────────────────────────────────────────────────────────────────
# Shared run-context — REUSED from openai_agents per review-standards
# §1.3. Polyglot agent stacks share a single trace because all the
# adapters read the same module-level ``spendguard_run_context``
# contextvar.
#
# Resilient import: ``..openai_agents`` ImportErrors when the
# ``[openai-agents]`` extra isn't installed. The shared contextvar
# mechanics don't actually depend on the openai-agents package — only
# the integration's wrapper class does. Mirrors the AutoGen pattern.
# ─────────────────────────────────────────────────────────────────────

try:  # pragma: no cover — branch chosen at import time
    from ..openai_agents import (  # noqa: F401
        RunContext,
        current_run_context,
        run_context,
    )
except ImportError:  # pragma: no cover — branch chosen at import time
    import contextvars
    from contextlib import asynccontextmanager
    from dataclasses import dataclass

    _RUN_CONTEXT: contextvars.ContextVar[RunContext | None] = (
        contextvars.ContextVar("spendguard_run_context", default=None)
    )

    @dataclass(frozen=True, slots=True)
    class RunContext:  # type: ignore[no-redef]
        """Per ``CodeAgent.run()`` / ``ToolCallingAgent.run()`` identifiers.

        Mirrors ``spendguard.integrations.openai_agents.RunContext`` when
        the ``[openai-agents]`` extra is not installed. Same contextvar
        NAME means a parent LangChain / Pydantic-AI / Strands / AutoGen
        run shares the run_id with this adapter regardless of which
        fallback branch fired at import time.
        """

        run_id: str

    @asynccontextmanager
    async def run_context(  # type: ignore[no-redef]
        ctx: RunContext,
    ) -> AsyncIterator[RunContext]:
        """Bind a ``RunContext`` for the duration of the wrapped block."""
        token = _RUN_CONTEXT.set(ctx)
        try:
            yield ctx
        finally:
            _RUN_CONTEXT.reset(token)

    def current_run_context() -> RunContext:  # type: ignore[no-redef]
        """Return the bound ``RunContext`` or raise a helpful ``RuntimeError``."""
        ctx = _RUN_CONTEXT.get()
        if ctx is None:
            raise RuntimeError(
                "spendguard.integrations.smolagents called outside an "
                "active run_context(). Wrap your CodeAgent.run "
                "invocation:\n\n"
                "    async with run_context(RunContext(run_id=...)):\n"
                "        result = agent.run('...')\n"
                "\n"
                "Note: CodeAgent.run is sync; the run_context CM is async "
                "— you can pre-bind the contextvar with run_context(...) "
                "in an outer async scope before driving the sync agent."
            )
        return ctx


log = logging.getLogger("spendguard.integrations.smolagents")


# ─────────────────────────────────────────────────────────────────────
# Type aliases (public surface)
# ─────────────────────────────────────────────────────────────────────

ClaimEstimator = Callable[[list[Any]], list[Any]]
"""Project ``BudgetClaim`` list from the messages payload.

SmolAgents ``Model.generate`` receives ``list[ChatMessage]`` (with
``role`` / ``content`` / ``tool_calls`` / ``raw`` / ``token_usage``
fields). The estimator gets that list verbatim — it can inspect
message content / token counts / model name (via the caller's closure
capture) to project the reservation amount.

v1 contract: returns >= 1 claim. Empty list is a config error.
"""


# ─────────────────────────────────────────────────────────────────────
# Helpers (module-level so they're testable + reusable)
# ─────────────────────────────────────────────────────────────────────


def _signature(
    messages: list[Any],
    stop_sequences: Any,
    response_format: Any,
    tools_to_call_from: Any,
    extra_kwargs: dict[str, Any],
) -> str:
    """Stable 16-byte BLAKE2b hex hash over generate() inputs.

    Per review-standards §6 the signature is derived from
    ``repr(messages) + repr(stop_sequences) + repr(response_format) +
    repr(tools_to_call_from) + repr(sorted(kwargs.items()))``. Omitting
    any of the five components is a Blocker — operator-chosen routing
    flags (e.g. ``temperature``, ``max_tokens``) MUST affect the
    reservation key. Sorting is required for determinism — unsorted
    dict items is a finding.

    blake2b-16 matches the OpenAI Agents / Agno / DSPy / AutoGen
    integrations' signature width so cross-framework ID derivation is
    symmetric.
    """
    text = (
        repr(messages)
        + "|"
        + repr(stop_sequences)
        + "|"
        + repr(response_format)
        + "|"
        + repr(tools_to_call_from)
        + "|"
        + repr(sorted(extra_kwargs.items()))
    )
    return hashlib.blake2b(text.encode("utf-8"), digest_size=16).hexdigest()


def _extract_total_tokens(result: Any) -> int:
    """Extract ``input_tokens + output_tokens`` from a ``ChatMessage``.

    Per review-standards §2.3 we read EXACTLY ``usage.input_tokens +
    usage.output_tokens`` from ``smolagents.monitoring.TokenUsage``
    (the ``ChatMessage.token_usage`` field). Reading ``total_tokens``
    directly would be a Blocker per the review spec — even though the
    field DOES exist in smolagents 1.26, the review locks the
    summation pattern for invariance across upstream re-definitions
    (e.g. a future version that ships ``total_tokens`` as a derived
    property where input/output are the source of truth).

    When ``token_usage is None`` return ``0`` — the projector still
    commits the reserve with a zero estimated amount, which is the
    correct fail-soft signal for a provider that didn't return usage.
    Raising here would block the audit chain.
    """
    if result is None:
        return 0
    usage = getattr(result, "token_usage", None)
    if usage is None:
        return 0
    input_tokens = getattr(usage, "input_tokens", 0) or 0
    output_tokens = getattr(usage, "output_tokens", 0) or 0
    try:
        return int(input_tokens) + int(output_tokens)
    except (TypeError, ValueError):
        # Defensive: usage fields might be non-numeric on a custom
        # client. Better to report 0 than crash the audit chain.
        return 0


def _classify_exception(exc: BaseException) -> str:
    """Classify an inner-call exception into a POST outcome label.

    Per review-standards §2.2 we use ``type(exc).__name__ ==
    "CancelledError"`` (matches the D12 LiteLLM shim and D24 AutoGen
    patterns) to avoid cross-loop ``isinstance`` mismatches across
    ``asyncio`` / ``trio`` / ``anyio``. SmolAgents itself uses
    blocking I/O (``requests`` / ``httpx``-sync); the cancellation
    label is retained for symmetry when the wrapper is driven from
    a parent async stack that propagates a ``CancelledError`` through
    a thread executor.

    Returns:
      * ``"CANCELLED"`` when the exception type name matches.
      * ``"FAILURE"`` for every other exception.
    """
    if type(exc).__name__ == "CancelledError":
        return "CANCELLED"
    return "FAILURE"


class SyncInAsyncContext(SpendGuardConfigError):
    """``generate()`` invoked from inside a running event loop.

    SmolAgents ``Model.generate`` is sync; the wrapper bridges the
    async sidecar RPCs via ``asyncio.run``. ``asyncio.run`` itself
    raises ``RuntimeError`` from inside a running loop with a
    confusing message — we raise this typed exception with a clear
    hint instead.

    Mirrors ``SpendGuardDSPyCallback.SyncInAsyncContext`` (DSPy 2.6
    callbacks are also sync).
    """


def _guard_async_context() -> None:
    """Raise ``SyncInAsyncContext`` if invoked inside a running loop.

    The check is sticky (``try/except RuntimeError``) because
    ``asyncio.get_running_loop()`` raises when no loop is active,
    which is the success case for this sync wrapper.
    """
    try:
        asyncio.get_running_loop()
    except RuntimeError:
        return
    raise SyncInAsyncContext(
        "SpendGuardSmolModel.generate() cannot be invoked from inside a "
        "running event loop. SmolAgents Model.generate is sync and the "
        "wrapper bridges the async sidecar RPCs via asyncio.run. Drive "
        "the agent from a sync entrypoint, OR (for async hosts) bind "
        "the wrapper to the host loop via "
        "`SpendGuardSmolModel.bind_loop(loop)` and dispatch the sync "
        "CodeAgent.run via a thread executor — the wrapper will then "
        "submit sidecar coros back to the host loop via "
        "asyncio.run_coroutine_threadsafe."
    )


def _run_coro_sync(coro: Any, *, host_loop: Any | None) -> Any:
    """Drive an async sidecar coro from a sync caller.

    Two paths:
      1. ``host_loop`` is provided (operator opted in via
         ``SpendGuardSmolModel.bind_loop(loop)``) — submit the coro to
         the host loop via ``asyncio.run_coroutine_threadsafe`` and
         block on the returned ``concurrent.futures.Future``. Required
         when the demo / production host drives the wrapper from an
         executor thread spawned by an outer ``asyncio.run`` (because
         the ``SpendGuardClient``'s grpc.aio channel is bound to that
         outer loop).
      2. ``host_loop`` is None — call ``asyncio.run(coro)`` on a fresh
         loop in the calling thread. Required when the wrapper is
         driven from a pure-sync entry point (CLI script, sync test
         fixture). The grpc.aio channel then operates entirely inside
         the fresh loop.
    """
    if host_loop is not None and host_loop.is_running():
        # Submit to host loop from this (worker) thread; wait blocking.
        fut = asyncio.run_coroutine_threadsafe(coro, host_loop)
        return fut.result()
    return asyncio.run(coro)


# ─────────────────────────────────────────────────────────────────────
# Wrapper class — concrete subclass of smolagents.Model when available
# ─────────────────────────────────────────────────────────────────────

# Pick the real ABC when smolagents is available so MultiStepAgent's
# isinstance checks pass; fall back to plain base class in unit tests
# where smolagents isn't installed (mirrors autogen/_hook.py).
if _SmolModel is not None:  # pragma: no cover — chosen at import
    _ModelBase: Any = _SmolModel
else:

    class _ModelBase:  # type: ignore[no-redef]
        """Unit-test stand-in for ``smolagents.Model``.

        Mirrors the smolagents 1.5+ ABC surface — ``generate`` plus
        ``__call__`` alias. The rest is deliberately unstubbed so a
        future binding gets a clear ``AttributeError`` rather than
        silently no-oping.
        """


class SpendGuardSmolModel(_ModelBase):  # type: ignore[misc, valid-type]
    """SmolAgents Model wrapper that gates each generate() through the sidecar.

    Subclasses ``smolagents.Model`` and overrides ``generate`` to insert
    PRE / POST sidecar hooks around the inner model's call. ``__call__``
    is aliased to ``generate`` so ``smolagents<1.5`` agents (which still
    call ``model(messages, ...)``) route through the same gate.

    Pass-through methods (``flatten_messages_as_text``,
    ``_prepare_completion_kwargs``, ``to_dict``, etc.) delegate verbatim
    to the inner via ``__getattr__`` fallback so the wrapper is
    resilient to upstream additions without needing per-method
    forwarding code.

    Per review-standards §1.2 the wrapper:
      - Takes ``inner: smolagents.Model`` (composition, not inheritance).
      - NEVER instantiates ``InferenceClient`` / ``openai.OpenAI`` /
        ``transformers.AutoModelForCausalLM`` directly.
      - Does NOT call ``super().__init__()`` — would force a synthetic
        ``model_id`` and break inner introspection.
      - Does NOT subclass any concrete vendor Model (``InferenceClientModel``,
        ``OpenAIServerModel``, ``TransformersModel``) — locks the
        wrapper to one vendor's lifecycle quirks.
      - Does NOT wrap a ``LiteLLMModel`` (review-standards §1.1 Blocker
        — would double-gate via D12).

    Per review-standards §7 fail-closed is the only mode. No
    ``SPENDGUARD_SMOLAGENTS_FAIL_OPEN`` env knob exists.

    Args:
        inner: A live ``smolagents.Model`` instance. Owned by the
            caller; not closed by the wrapper.
        client: A connected + handshook ``SpendGuardClient``.
        budget_id: Budget the reservation debits. REQUIRED.
        window_instance_id: Time-window scope on the budget. REQUIRED.
        unit: ``common_pb2.UnitRef`` describing the unit binding.
            REQUIRED.
        pricing: ``common_pb2.PricingFreeze`` for ledger lookup. REQUIRED.
        claim_estimator: ``(messages) → list[BudgetClaim]`` projector.
            REQUIRED — design.md §5 locks "No default ``claim_estimator``"
            because ``model_id`` is set on ``InferenceClientModel`` /
            ``OpenAIServerModel`` but absent on ``TransformersModel``;
            a uniform default is not safe.
        route: ``request_decision.route``. Defaults to ``"llm.call"``
            so dashboards group with the other framework integrations.
    """

    def __init__(
        self,
        *,
        inner: Any,
        client: SpendGuardClient,
        budget_id: str,
        window_instance_id: str,
        unit: Any,
        pricing: Any,
        claim_estimator: ClaimEstimator,
        route: str = "llm.call",
    ) -> None:
        # NOTE: smolagents.Model.__init__ sets attributes (model_id,
        # flatten_messages_as_text, tool_name_key, tool_arguments_key)
        # used only by direct vendor subclasses. Per review-standards
        # §1.2 we MUST NOT call super().__init__() — would force a
        # synthetic model_id and break inner introspection.
        if inner is None:
            raise SpendGuardConfigError(
                "SpendGuardSmolModel(inner=...) is required; got None. "
                "Pass a live smolagents.Model instance "
                "(e.g. OpenAIServerModel(model_id='gpt-4o-mini', "
                "api_base=..., api_key=...))."
            )
        if client is None:
            raise SpendGuardConfigError(
                "SpendGuardSmolModel(client=...) is required; got None."
            )
        if not budget_id:
            raise SpendGuardConfigError(
                "SpendGuardSmolModel(budget_id=...) required."
            )
        if not window_instance_id:
            raise SpendGuardConfigError(
                "SpendGuardSmolModel(window_instance_id=...) required."
            )
        unit_id = getattr(unit, "unit_id", "") if unit is not None else ""
        if not unit_id:
            raise SpendGuardConfigError(
                "SpendGuardSmolModel unit.unit_id required."
            )
        if claim_estimator is None:
            raise SpendGuardConfigError(
                "SpendGuardSmolModel(claim_estimator=...) is required; "
                "design.md §5 locks no default estimator because "
                "model_id is not standardized across smolagents inner "
                "Model classes (absent on TransformersModel)."
            )
        # Refuse to wrap a LiteLLMModel — D12 SDK shim is the canonical
        # path, and wrapping would double-gate. Best-effort string check
        # so the wrapper still loads when smolagents isn't installed in
        # the host venv (unit-test path).
        inner_type = type(inner).__name__
        if inner_type == "LiteLLMModel":
            raise SpendGuardConfigError(
                "SpendGuardSmolModel refuses to wrap a smolagents "
                "LiteLLMModel — the D12 LiteLLM SDK shim already gates "
                "every litellm.acompletion call. Wrapping would "
                "double-gate (two reservations per call). Use the "
                "shim directly: "
                "`pip install spendguard-litellm-shim` and let the raw "
                "LiteLLMModel call through. See "
                "docs/integrations/litellm-sdk-shim."
            )
        self._inner = inner
        self._client = client
        self._budget_id = budget_id
        self._window_instance_id = window_instance_id
        self._unit = unit
        self._pricing = pricing
        self._claim_estimator = claim_estimator
        self._route = route
        # Optional host event loop for async-host drivers. When set
        # (via `bind_loop(loop)`), the wrapper uses
        # `asyncio.run_coroutine_threadsafe` to submit sidecar coros
        # back to the host loop from the thread executor that drives
        # the sync `generate()` call. When None (the canonical sync
        # entry-point case), the wrapper uses `asyncio.run` on a fresh
        # loop in the calling thread. Validated against the test
        # fixture's `_drive_sync_generate` (no host loop) and the
        # demo's `run_in_executor` path (host loop).
        self._host_loop: Any | None = None

    def bind_loop(self, loop: Any) -> None:
        """Bind a host event loop for async-host driver patterns.

        When the wrapper is invoked from a thread executor spawned by
        an outer ``asyncio.run`` (the canonical "async host + sync
        CodeAgent" pattern), the ``SpendGuardClient``'s grpc.aio
        channel is bound to the outer loop. Calling ``asyncio.run`` in
        the executor thread would create a fresh loop and fail with
        "Future attached to a different loop". Instead, the wrapper
        submits sidecar coros back to the bound host loop via
        ``asyncio.run_coroutine_threadsafe``.

        Call this once after constructing the wrapper, before
        dispatching the agent to the executor:

            guarded = SpendGuardSmolModel(inner=..., client=..., ...)
            guarded.bind_loop(asyncio.get_running_loop())
            await asyncio.get_running_loop().run_in_executor(
                None, lambda: ctx.run(agent.run, "..."),
            )

        Pass ``None`` to clear the binding.
        """
        self._host_loop = loop

    # ─────────────────────────────────────────────────────────────────
    # generate — SYNCHRONOUS (matches smolagents 1.5+ ABC contract).
    # PRE before HTTP, POST after, fail-closed. Bridges to async
    # SpendGuardClient RPCs via asyncio.run (see DEVIATION-1 in the
    # module docstring).
    # ─────────────────────────────────────────────────────────────────

    def generate(
        self,
        messages: list[Any],
        stop_sequences: list[str] | None = None,
        response_format: Any = None,
        tools_to_call_from: list[Any] | None = None,
        **kwargs: Any,
    ) -> Any:
        """Wrap ``inner.generate(...)`` with PRE/POST sidecar hooks.

        Order (verified by call-count assertions in tests):
          1. ``RequestDecision(LLM_CALL_PRE)`` — fail-closed.
          2. ``inner.generate(...)`` — provider HTTP, ONLY if PRE allowed.
          3. ``emit_llm_call_post(...)`` — fires for SUCCESS / FAILURE
             / CANCELLED when a reservation was issued.

        Per review-standards §2.1 ``RequestDecision`` is awaited
        BEFORE ``self._inner.generate(...)``. DENY raises
        ``DecisionDenied`` BEFORE any inner-model method is awaited.

        Per review-standards §6 ``kwargs`` is included in the signature
        via ``repr(sorted(kwargs.items()))`` — operator-chosen routing
        flags affect the reservation key.

        DEVIATION-1: ``generate`` is SYNCHRONOUS. Async sidecar RPCs
        bridge via ``asyncio.run`` with a sticky guard against
        invocation from inside a running event loop.
        """
        _guard_async_context()
        ctx = current_run_context()
        signature = _signature(
            messages, stop_sequences, response_format,
            tools_to_call_from, kwargs,
        )
        llm_call_id = str(
            derive_uuid_from_signature(signature, scope="llm_call_id")
        )
        decision_id = str(
            derive_uuid_from_signature(signature, scope="decision_id")
        )
        step_id = f"{ctx.run_id}:smol-call:{signature[:16]}"
        idempotency_key = derive_idempotency_key(
            tenant_id=self._client.tenant_id,
            session_id=self._client.session_id,
            run_id=ctx.run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )

        # Project budget claims via the operator-supplied estimator.
        projected_claims = self._claim_estimator(messages)

        decision_context = {
            "integration": "smolagents",
            "inner_model": type(self._inner).__name__,
        }

        # ── PRE — fail-closed reserve ────────────────────────────────
        # request_decision raises DecisionDenied / DecisionStopped /
        # ApprovalRequired derived classes on a STOP path; we let them
        # propagate unchanged. smolagents.MultiStepAgent has no
        # framework-side catch on the model.generate() path (verified
        # against smolagents 1.26 agents.py — the agent's step loop
        # propagates non-AgentError exceptions verbatim), so the raise
        # reaches the CodeAgent.run caller cleanly.
        outcome: DecisionOutcome = _run_coro_sync(
            self._client.request_decision(
                trigger="LLM_CALL_PRE",
                run_id=ctx.run_id,
                step_id=step_id,
                llm_call_id=llm_call_id,
                tool_call_id="",
                decision_id=decision_id,
                route=self._route,
                projected_claims=projected_claims,
                idempotency_key=idempotency_key,
                projected_unit=self._unit,
                decision_context_json=decision_context,
            ),
            host_loop=self._host_loop,
        )

        # ── Inner call ───────────────────────────────────────────────
        try:
            result = self._inner.generate(
                messages,
                stop_sequences=stop_sequences,
                response_format=response_format,
                tools_to_call_from=tools_to_call_from,
                **kwargs,
            )
        except BaseException as exc:
            # CancelledError + everything else. Per review-standards
            # §2.2 POST fires for FAILURE / CANCELLED when a reservation
            # was issued; otherwise (DENY-then-fail unreachable but
            # defensively guarded) POST is skipped.
            if outcome.reservation_ids:
                outcome_kind = _classify_exception(exc)
                try:
                    _run_coro_sync(
                        self._client.emit_llm_call_post(
                            run_id=ctx.run_id,
                            step_id=step_id,
                            llm_call_id=llm_call_id,
                            decision_id=outcome.decision_id,
                            reservation_id=outcome.reservation_ids[0],
                            provider_reported_amount_atomic="",
                            estimated_amount_atomic="0",
                            unit=self._unit,
                            pricing=self._pricing,
                            provider_event_id="",
                            outcome=outcome_kind,
                        ),
                        host_loop=self._host_loop,
                    )
                except Exception as post_exc:  # noqa: BLE001
                    # Best-effort: never mask the original inner
                    # exception that the CodeAgent caller is about to
                    # see. Log + swallow; reservation TTL-sweeps.
                    log.warning(
                        "spendguard.integrations.smolagents: "
                        "emit_llm_call_post failed on exception path "
                        "(run_id=%s sig=%s err=%r) — reservation will "
                        "TTL-sweep",
                        ctx.run_id,
                        signature[:8],
                        post_exc,
                    )
            raise

        # ── POST — success commit ────────────────────────────────────
        # Per review-standards §2.2 POST MUST NOT fire when no
        # reservation exists (DENY path is unreachable here because
        # request_decision raises; this guards an ALLOW-with-empty-
        # reservation-ids corner case the projector might still emit in
        # degenerate test setups).
        total_tokens = _extract_total_tokens(result)
        if outcome.reservation_ids:
            _run_coro_sync(
                self._client.emit_llm_call_post(
                    run_id=ctx.run_id,
                    step_id=step_id,
                    llm_call_id=llm_call_id,
                    decision_id=outcome.decision_id,
                    reservation_id=outcome.reservation_ids[0],
                    provider_reported_amount_atomic="",
                    estimated_amount_atomic=str(total_tokens),
                    unit=self._unit,
                    pricing=self._pricing,
                    provider_event_id="",
                    outcome="SUCCESS",
                ),
                host_loop=self._host_loop,
            )
        return result

    # ─────────────────────────────────────────────────────────────────
    # __call__ alias — version-drift bypass guard.
    # `smolagents<1.5` agents call ``model(messages, ...)``; `>=1.5`
    # call ``model.generate(...)``. The wrapper defines both with
    # ``__call__`` delegating to ``generate``, so install-time version
    # drift cannot bypass the gate. Per review-standards §3.2: MUST
    # NOT duplicate PRE/POST logic — the alias funnels into generate.
    # ─────────────────────────────────────────────────────────────────

    def __call__(
        self,
        messages: list[Any],
        stop_sequences: list[str] | None = None,
        response_format: Any = None,
        tools_to_call_from: list[Any] | None = None,
        **kwargs: Any,
    ) -> Any:
        """Alias for ``generate`` — covers ``smolagents<1.5`` callers."""
        return self.generate(
            messages,
            stop_sequences=stop_sequences,
            response_format=response_format,
            tools_to_call_from=tools_to_call_from,
            **kwargs,
        )

    # ─────────────────────────────────────────────────────────────────
    # Forward arbitrary inner methods (``flatten_messages_as_text``,
    # ``_prepare_completion_kwargs``, ``to_dict``, vendor-specific
    # helpers) without enumerating them — keeps the wrapper resilient
    # to upstream additions. Per review-standards §5.1 private names
    # raise AttributeError so wrapper._inner-style state leakage is
    # blocked.
    # ─────────────────────────────────────────────────────────────────

    def __getattr__(self, name: str) -> Any:
        """Forward non-private attribute access to the inner Model.

        Per review-standards §5.1:
          - Returns the inner's attribute for non-private names.
          - Raises ``AttributeError`` for private names (``_inner``
            etc) so wrapper internals don't leak.
          - Does NOT shadow ``generate`` / ``__call__`` /
            ``_extract_total_tokens`` — Python's MRO ensures explicit
            methods on ``SpendGuardSmolModel`` always win;
            ``__getattr__`` only fires when the attribute is genuinely
            missing on the wrapper.
        """
        if name.startswith("_"):
            raise AttributeError(name)
        return getattr(self._inner, name)


# ─────────────────────────────────────────────────────────────────────
# spendguard_step_callback — informational-only telemetry helper.
# NOT a gating surface — step_callbacks fire AFTER each step completes.
# Per review-standards §4.1 MUST NOT call request_decision; §4.2 MUST
# catch every Exception (NOT BaseException — KeyboardInterrupt /
# SystemExit propagate) and never raise out of the callable.
# ─────────────────────────────────────────────────────────────────────


def spendguard_step_callback(
    client: SpendGuardClient,
    *,
    run_id: str,
) -> Callable[[Any], None]:
    """Return a ``step_callbacks``-compatible callable.

    Emits an informational ``agent_step`` audit event after each
    ``ActionStep`` / ``PlanningStep`` completes. Used as::

        agent = CodeAgent(
            model=SpendGuardSmolModel(inner=...),
            step_callbacks=[spendguard_step_callback(client, run_id="...")],
        )

    NOT a gating surface — ``step_callbacks`` fire AFTER each step
    completes; they cannot deny a pending LLM call. The wrapper
    (``SpendGuardSmolModel``) is the gating surface. Per review-
    standards §4.1: any ``request_decision`` call inside the callback
    is a Blocker — would fire AFTER the step's LLM call already
    completed, which is a wrong-time gate.

    The callable catches every exception so a sidecar outage during
    telemetry cannot abort the host agent run. Per review-standards
    §4.2: catches ``Exception`` (NOT ``BaseException``);
    ``KeyboardInterrupt`` / ``SystemExit`` propagate.

    Implementation note: ``SpendGuardClient`` does not yet expose a
    dedicated ``emit_agent_step_telemetry`` method (implementation.md
    §2 Slice 3 anticipated this). The callable currently logs an
    informational record via the SDK's standard logger and is a no-op
    on the sidecar wire — when the dedicated audit-event method ships
    (or the SDK adds ``emit_custom_audit``), this callable is the
    single edit point. Operators relying on the gating surface
    (``SpendGuardSmolModel``) are unaffected.

    Args:
        client: A connected ``SpendGuardClient``. Used for tenant /
            session correlation in the telemetry record.
        run_id: The ``RunContext.run_id`` the step belongs to — caller
            captures the same value passed to ``run_context(...)``.

    Returns:
        A sync ``Callable[[ActionStep | PlanningStep], None]`` ready
        for ``MultiStepAgent(step_callbacks=[...])``.
    """
    if client is None:
        raise SpendGuardConfigError(
            "spendguard_step_callback(client=...) is required; got None."
        )
    if not run_id:
        raise SpendGuardConfigError(
            "spendguard_step_callback(run_id=...) is required; pass the "
            "same run_id you used in run_context(RunContext(run_id=...))."
        )

    def _cb(step: Any) -> None:
        try:
            step_kind = type(step).__name__  # "ActionStep" | "PlanningStep"
            step_number = getattr(step, "step_number", None)
            # Best-effort telemetry: prefer the dedicated method when
            # the SDK ships it (see implementation.md §2 Slice 3); fall
            # back to a structured log record on the SDK's standard
            # logger. The callable returns None either way — never
            # raises out of the callback per review-standards §4.2.
            emit_method = getattr(client, "emit_agent_step_telemetry", None)
            if emit_method is not None:
                emit_method(
                    run_id=run_id,
                    step_kind=step_kind,
                    step_number=int(step_number) if step_number is not None else 0,
                )
                return
            emit_audit = getattr(client, "emit_custom_audit", None)
            if emit_audit is not None:
                emit_audit(
                    "agent_step",
                    {
                        "run_id": run_id,
                        "step_kind": step_kind,
                        "step_number": int(step_number) if step_number is not None else 0,
                        "tenant_id": getattr(client, "tenant_id", ""),
                    },
                )
                return
            # SDK doesn't yet expose either entry point — log only.
            # This is informational-only by design; gating is on the
            # wrapper. When the SDK ships the dedicated method this
            # branch is replaced via the getattr fast path above.
            log.info(
                "spendguard.smolagents.agent_step run_id=%s kind=%s number=%s",
                run_id,
                step_kind,
                step_number,
            )
        except Exception:
            # NEVER raise out of the callback — review-standards §4.2.
            # Catch Exception (not BaseException) so KeyboardInterrupt
            # / SystemExit propagate; sidecar telemetry outage must
            # NOT abort the host agent run.
            log.warning(
                "spendguard_step_callback swallowed exception "
                "(run_id=%s); host agent run NOT aborted",
                run_id,
                exc_info=True,
            )

    return _cb


__all__ = [
    "ClaimEstimator",
    "RunContext",
    "SpendGuardSmolModel",
    "SyncInAsyncContext",
    "_classify_exception",
    "_extract_total_tokens",
    "_guard_async_context",
    "_run_coro_sync",
    "_signature",
    "current_run_context",
    "run_context",
    "spendguard_step_callback",
]
