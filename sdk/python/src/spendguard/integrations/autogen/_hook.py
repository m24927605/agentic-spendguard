# ruff: noqa: ANN401
"""``SpendGuardChatCompletionClient`` — AutoGen 0.4+ / AG2 wrap adapter.

Implements the D24 design: subclass ``autogen_core.models.ChatCompletionClient``
and wrap an ``inner`` client with PRE / POST sidecar hooks. ``create()``
inserts ``RequestDecision(LLM_CALL_PRE)`` BEFORE the inner HTTP fires
and ``emit_llm_call_post`` AFTER, propagating ``SUCCESS`` /
``FAILURE`` / ``CANCELLED`` based on the inner-call outcome.

Both AutoGen 0.4+ (Microsoft, maintenance mode as of 2026-02) and AG2
(community fork led by ex-AutoGen maintainers, ~48k stars Apache-2.0)
share ``autogen_core.models.ChatCompletionClient`` unchanged — AG2
vendored the namespace as a re-export through at least 0.7.x. One
wrapper covers both lineages.

Lifecycle (per design.md §4)::

    AssistantAgent.on_messages(...)
      ↓ model_client.create(messages, tools, ...)
      ↓ SpendGuardChatCompletionClient.create
        ├─ ctx = current_run_context()       (reused from openai_agents)
        ├─ signature = blake2b(messages | tools | extra_create_args)
        ├─ llm_call_id / decision_id derived from signature
        ├─ sidecar.RequestDecision(LLM_CALL_PRE)
        │     ALLOW    → continue
        │     DENY     → DecisionDenied propagates (no inner HTTP)
        │     DEGRADE  → SidecarUnavailable propagates (fail-closed)
        ├─ inner.create(messages, tools, ...)   provider HTTP
        └─ sidecar.emit_llm_call_post(SUCCESS|FAILURE|CANCELLED,
                                      estimated=usage.prompt + completion)

Per review-standards §1 / §2:
  - ``LINEAGE`` probe is telemetry-only. Business logic in ``create()``
    / ``create_stream()`` NEVER branches on it. A conditional on
    ``LINEAGE`` other than logging / metric labels would be a Blocker.
  - Composition over inheritance for the inner client: the wrapper
    NEVER instantiates ``OpenAIChatCompletionClient`` /
    ``AnthropicChatCompletionClient`` / any vendor SDK directly.
  - No ``super().__init__()`` call — ``ChatCompletionClient`` is an
    ABC with no shared state in either lineage. A super call would
    silently change inner-client behavior under upstream refactors.
  - ``current_run_context`` is IMPORTED from
    ``spendguard.integrations.openai_agents``, NOT redefined.
    Polyglot agent stacks share one trace.

POC scope (per design.md §3 / review-standards §3):
  - ``create_stream()`` is documented as pass-through with PRE/POST
    firing at the next ``create()`` boundary. Per-chunk gating tracked
    as follow-on. Test ``test_create_stream_does_not_call_request_decision``
    asserts the intentional behavior.
  - Pass-through introspection (``count_tokens`` / ``total_usage`` /
    ``actual_usage`` / ``remaining_tokens`` / ``capabilities`` /
    ``model_info``) carries NO side effects (no sidecar calls, no
    caching, no metric emission) — required by ``AssistantAgent``'s
    token-budget caps.
"""

from __future__ import annotations

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
# autogen_core import — required at module load. The package barrel
# ``__init__.py`` carries the install-hint ImportError guard; here we
# import resiliently so the unit suite can load ``_hook`` directly via
# package-path bypass (mirrors ``dspy/_wrapper.py`` pattern).
# ─────────────────────────────────────────────────────────────────────

try:  # pragma: no cover — branch chosen at import time
    from autogen_core.models import (  # type: ignore[import-not-found]
        ChatCompletionClient as _ChatCompletionClient,
    )

    _AUTOGEN_CORE_AVAILABLE = True
except ImportError:  # pragma: no cover — branch chosen at import time
    _ChatCompletionClient = None  # type: ignore[assignment, misc]
    _AUTOGEN_CORE_AVAILABLE = False


# ─────────────────────────────────────────────────────────────────────
# Shared run-context — REUSED from openai_agents per review-standards
# §1.3. Polyglot agent stacks (OpenAI Agents → AutoGen → Pydantic-AI in
# one run) share a single trace because all four adapters read the
# same module-level ``spendguard_run_context`` contextvar.
#
# Resilient import: ``..openai_agents`` ImportErrors when the
# ``[openai-agents]`` extra isn't installed (the barrel raises a
# helpful install-hint error). The shared contextvar mechanics
# (RunContext frozen dataclass + run_context asynccontextmanager +
# current_run_context lookup) don't actually depend on the openai-agents
# package — only the integration's wrapper class does. We import the
# three symbols defensively and fall back to a local mirror (same
# contextvar NAME so cross-framework sharing still works) when the
# barrel fails.
# ─────────────────────────────────────────────────────────────────────

try:  # pragma: no cover — branch chosen at import time
    from ..openai_agents import (  # noqa: F401
        RunContext,
        current_run_context,
        run_context,
    )
except ImportError:  # pragma: no cover — branch chosen at import time
    # Fallback mirror — re-declare the same contextvar NAME so a
    # parent run_context() in another framework still shares the run_id
    # with this adapter. The fallback is byte-for-byte identical to the
    # openai_agents module's binding so the contextvar identity matches
    # at runtime.
    import contextvars
    from contextlib import asynccontextmanager
    from dataclasses import dataclass

    _RUN_CONTEXT: contextvars.ContextVar["RunContext | None"] = (
        contextvars.ContextVar("spendguard_run_context", default=None)
    )

    @dataclass(frozen=True, slots=True)
    class RunContext:  # type: ignore[no-redef]
        """Per ``AssistantAgent.on_messages()`` identifiers.

        Mirrors ``spendguard.integrations.openai_agents.RunContext``
        when the ``[openai-agents]`` extra is not installed. Same
        contextvar NAME means a parent LangChain / Pydantic-AI /
        Strands run shares the run_id with this adapter regardless of
        which fallback branch fired at import time.
        """

        run_id: str

    @asynccontextmanager
    async def run_context(ctx: "RunContext") -> AsyncIterator["RunContext"]:  # type: ignore[no-redef]
        """Bind a ``RunContext`` for the duration of the wrapped block."""
        token = _RUN_CONTEXT.set(ctx)
        try:
            yield ctx
        finally:
            _RUN_CONTEXT.reset(token)

    def current_run_context() -> "RunContext":  # type: ignore[no-redef]
        """Return the bound ``RunContext`` or raise a helpful ``RuntimeError``."""
        ctx = _RUN_CONTEXT.get()
        if ctx is None:
            raise RuntimeError(
                "spendguard.integrations.autogen called outside an active "
                "run_context(). Wrap your AssistantAgent.on_messages "
                "invocation:\n\n"
                "    async with run_context(RunContext(run_id=...)):\n"
                "        await agent.on_messages([...], cancellation_token)\n"
            )
        return ctx


log = logging.getLogger("spendguard.integrations.autogen")


# ─────────────────────────────────────────────────────────────────────
# Lineage probe — telemetry only, NEVER branches business logic.
# ─────────────────────────────────────────────────────────────────────


def _probe_lineage() -> str:
    """Detect which AutoGen / AG2 lineage is loaded alongside autogen-core.

    Returns:
        ``"both"`` when both lineages are installed.
        ``"ag2"`` when only AG2 is installed.
        ``"autogen"`` when only ``autogen-agentchat`` is installed.
        ``"core-only"`` when neither is installed — autogen-core only,
            degenerate but the wrapper still works against a custom
            ``ChatCompletionClient`` subclass.
    """
    has_autogen_agentchat = False
    has_ag2 = False
    try:  # pragma: no cover — branch chosen at import time
        import autogen_agentchat  # type: ignore[import-not-found] # noqa: F401

        has_autogen_agentchat = True
    except ImportError:  # pragma: no cover
        pass
    try:  # pragma: no cover — branch chosen at import time
        import ag2  # type: ignore[import-not-found] # noqa: F401

        has_ag2 = True
    except ImportError:  # pragma: no cover
        pass
    if has_autogen_agentchat and has_ag2:
        return "both"
    if has_ag2:
        return "ag2"
    if has_autogen_agentchat:
        return "autogen"
    return "core-only"


LINEAGE: str = _probe_lineage()


# ─────────────────────────────────────────────────────────────────────
# Type aliases (public surface)
# ─────────────────────────────────────────────────────────────────────

ClaimEstimator = Callable[[list[Any]], list[Any]]
"""Project ``BudgetClaim`` list from the ``messages`` payload.

AutoGen / AG2 ``ChatCompletionClient.create`` receives
``list[LLMMessage]`` (subclasses: ``SystemMessage`` /
``UserMessage`` / ``AssistantMessage`` / ``FunctionExecutionResultMessage``).
The estimator gets that list verbatim — it can inspect message
content / token counts / model name (via the caller's closure capture)
to project the reservation amount.

v1 contract: returns >= 1 claim. Empty list is a config error.
"""


# ─────────────────────────────────────────────────────────────────────
# Helpers (module-level so they're testable + reusable)
# ─────────────────────────────────────────────────────────────────────


def _signature(
    messages: list[Any],
    tools: Any,
    extra_create_args: dict[str, Any],
) -> str:
    """Stable 16-byte BLAKE2b hex hash over create() inputs.

    Per review-standards §6 the signature is derived from
    ``repr(messages) + repr(tools) + repr(sorted(extra.items()))``.
    Sorting is required for determinism: ``dict.items()`` order is
    insertion-order in Python 3.7+, but different call sites passing
    the same logical map in different insertion orders would otherwise
    collide on different keys.

    blake2b-16 matches the OpenAI Agents / Agno / DSPy integrations'
    signature width so cross-framework ID derivation is symmetric.
    """
    text = (
        repr(messages)
        + "|"
        + repr(tools)
        + "|"
        + repr(sorted(extra_create_args.items()))
    )
    return hashlib.blake2b(text.encode("utf-8"), digest_size=16).hexdigest()


def _extract_total_tokens(result: Any) -> int:
    """Extract ``prompt_tokens + completion_tokens`` from a ``CreateResult``.

    Per review-standards §2.3 we read EXACTLY ``usage.prompt_tokens +
    usage.completion_tokens`` from ``autogen_core.models.RequestUsage``.
    Reading any other field (e.g. ``total_tokens``, which does NOT
    exist on ``RequestUsage`` in either lineage) is a Blocker.

    When ``usage is None`` return ``0`` — the projector still commits
    the reserve with a zero estimated amount, which is the correct
    fail-soft signal for a provider that didn't return usage. Raising
    here would block the audit chain.
    """
    if result is None:
        return 0
    usage = getattr(result, "usage", None)
    if usage is None:
        return 0
    prompt = getattr(usage, "prompt_tokens", 0) or 0
    completion = getattr(usage, "completion_tokens", 0) or 0
    try:
        return int(prompt) + int(completion)
    except (TypeError, ValueError):
        # Defensive: usage fields might be non-numeric on a custom
        # client. Better to report 0 than crash the audit chain.
        return 0


def _classify_exception(exc: BaseException) -> str:
    """Classify an inner-call exception into a POST outcome label.

    Per review-standards §2.2 we use ``type(exc).__name__ ==
    "CancelledError"`` (matches the D12 LiteLLM shim pattern) to avoid
    cross-loop ``isinstance`` mismatches across ``asyncio`` / ``trio``
    / ``anyio``. AutoGen 0.4 uses asyncio, AG2 ships an anyio-flavored
    runner; both raise a ``CancelledError`` from their respective
    runtimes when ``cancellation_token`` fires.

    Returns:
      * ``"CANCELLED"`` when the exception type name matches.
      * ``"FAILURE"`` for every other exception.
    """
    if type(exc).__name__ == "CancelledError":
        return "CANCELLED"
    return "FAILURE"


# ─────────────────────────────────────────────────────────────────────
# Wrapper class — concrete subclass of ChatCompletionClient
# ─────────────────────────────────────────────────────────────────────

# Pick the real ABC when autogen-core is available so AssistantAgent's
# isinstance checks pass; fall back to plain base class in unit tests
# where autogen-core isn't installed (mirrors dspy/_wrapper.py).
if _ChatCompletionClient is not None:  # pragma: no cover — chosen at import
    _ClientBase: Any = _ChatCompletionClient
else:

    class _ClientBase:  # type: ignore[no-redef]
        """Unit-test stand-in for ``autogen_core.models.ChatCompletionClient``.

        Mirrors the AutoGen 0.4 / AG2 GA contract surface — ``create`` /
        ``create_stream`` plus the introspection methods. The rest is
        deliberately unstubbed so a future binding gets a clear
        AttributeError rather than silently no-oping.
        """


class SpendGuardChatCompletionClient(_ClientBase):  # type: ignore[misc, valid-type]
    """AutoGen / AG2 ``ChatCompletionClient`` wrapper that gates each call.

    Subclasses ``autogen_core.models.ChatCompletionClient`` and
    overrides ``create`` to insert PRE / POST sidecar hooks around the
    inner client call. ``create_stream`` returns the inner stream
    directly (POC scope — parity with ``SpendGuardAgentsModel.stream_response``);
    PRE / POST fires at the next ``create()`` boundary when the
    framework eventually issues a non-streaming finalization call.

    Per review-standards §1.2 the wrapper:
      - Takes ``inner: ChatCompletionClient`` (composition, not inheritance).
      - NEVER instantiates ``OpenAIChatCompletionClient`` /
        ``AnthropicChatCompletionClient`` / any vendor SDK directly.
      - Does NOT call ``super().__init__()`` — ABC has no shared state.
      - Does NOT subclass any concrete vendor client — that breaks AG2
        polymorphism guarantees.

    Per review-standards §6 fail-closed is the only mode. No
    ``SPENDGUARD_AUTOGEN_FAIL_OPEN`` env knob exists.

    Args:
        inner: A live ``ChatCompletionClient`` instance. Owned by the
            caller; not closed by the wrapper.
        client: A connected + handshook ``SpendGuardClient``.
        budget_id: Budget the reservation debits. REQUIRED.
        window_instance_id: Time-window scope on the budget. REQUIRED.
        unit: ``common_pb2.UnitRef`` describing the unit binding.
            REQUIRED.
        pricing: ``common_pb2.PricingFreeze`` for ledger lookup. REQUIRED.
        claim_estimator: ``(messages) → list[BudgetClaim]`` projector.
            REQUIRED — design.md §5 locks "No default ``claim_estimator``"
            because ``ChatCompletionClient.model`` is not standardized
            across vendor implementations.
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
        # NOTE: ChatCompletionClient is an ABC with NO shared state in
        # ``__init__`` (verified against autogen-core 0.4.0 and ag2 0.7.0).
        # Per review-standards §1.2 we MUST NOT call super().__init__()
        # — silently changes inner-client behavior under upstream
        # refactors.
        if inner is None:
            raise SpendGuardConfigError(
                "SpendGuardChatCompletionClient(inner=...) is required; "
                "got None. Pass a live autogen_core.models.ChatCompletionClient "
                "instance (e.g. OpenAIChatCompletionClient(model='gpt-4o-mini'))."
            )
        if client is None:
            raise SpendGuardConfigError(
                "SpendGuardChatCompletionClient(client=...) is required; "
                "got None."
            )
        if not budget_id:
            raise SpendGuardConfigError(
                "SpendGuardChatCompletionClient(budget_id=...) required."
            )
        if not window_instance_id:
            raise SpendGuardConfigError(
                "SpendGuardChatCompletionClient(window_instance_id=...) required."
            )
        unit_id = getattr(unit, "unit_id", "") if unit is not None else ""
        if not unit_id:
            raise SpendGuardConfigError(
                "SpendGuardChatCompletionClient unit.unit_id required."
            )
        if claim_estimator is None:
            raise SpendGuardConfigError(
                "SpendGuardChatCompletionClient(claim_estimator=...) is "
                "required; design.md §5 locks no default estimator "
                "because ChatCompletionClient.model is not standardized "
                "across vendor implementations."
            )
        self._inner = inner
        self._client = client
        self._budget_id = budget_id
        self._window_instance_id = window_instance_id
        self._unit = unit
        self._pricing = pricing
        self._claim_estimator = claim_estimator
        self._route = route

    # ─────────────────────────────────────────────────────────────────
    # create — PRE before HTTP, POST after, fail-closed.
    # ─────────────────────────────────────────────────────────────────

    async def create(
        self,
        messages: list[Any],
        *,
        tools: Any = (),
        tool_choice: Any = "auto",
        json_output: Any = None,
        extra_create_args: dict[str, Any] | None = None,
        cancellation_token: Any = None,
        **kwargs: Any,
    ) -> Any:
        """Wrap ``inner.create(...)`` with PRE/POST sidecar hooks.

        Order (verified by bytecode in tests):
          1. ``RequestDecision(LLM_CALL_PRE)`` — fail-closed.
          2. ``inner.create(...)`` — provider HTTP, ONLY if PRE allowed.
          3. ``emit_llm_call_post(...)`` — fires for SUCCESS / FAILURE
             / CANCELLED when a reservation was issued.

        Per review-standards §2.1 ``RequestDecision`` is awaited
        BEFORE ``self._inner.create(...)``. DENY raises
        ``DecisionDenied`` BEFORE any inner-client method is awaited.

        Per review-standards §6 ``extra_create_args`` is shallow-copied
        before being included in the signature — without the copy a
        caller mutating the dict between PRE and inner call would
        create a TOCTOU.

        Forward-compat: ``tool_choice`` is recent (autogen-core 0.7+);
        ``**kwargs`` swallows any future parameters AutoGen / AG2 add
        without breaking older wrapper versions. Spec design.md §4 was
        authored against 0.4.0; the wrapper passes everything through
        to ``inner.create`` verbatim.
        """
        ctx = current_run_context()
        # Shallow-copy the operator's dict so a mutation between PRE
        # and inner call doesn't break the signature contract.
        extra = dict(extra_create_args or {})
        signature = _signature(messages, tools, extra)
        llm_call_id = str(
            derive_uuid_from_signature(signature, scope="llm_call_id")
        )
        decision_id = str(
            derive_uuid_from_signature(signature, scope="decision_id")
        )
        step_id = f"{ctx.run_id}:autogen-call:{signature[:16]}"
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
            "integration": "autogen",
            "lineage": LINEAGE,
        }
        # Capture the inner client class name for audit context — best
        # effort, never raises. Useful for dashboards grouping by
        # backend (OpenAIChatCompletionClient vs Anthropic vs Azure).
        inner_type = type(self._inner).__name__
        if inner_type:
            decision_context["inner_client"] = inner_type

        # ── PRE — fail-closed reserve ────────────────────────────────
        # request_decision raises DecisionDenied / DecisionStopped /
        # ApprovalRequired derived classes on a STOP path; we let them
        # propagate unchanged. ChatCompletionClient has no
        # framework-side catch on the create() path in either lineage,
        # so the raise reaches the AssistantAgent caller cleanly.
        outcome: DecisionOutcome = await self._client.request_decision(
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
        )

        # ── Inner call ───────────────────────────────────────────────
        try:
            # Build inner-call kwargs defensively: only forward
            # ``tool_choice`` when the operator passed it (older inner
            # clients don't accept it) and pass through any unknown
            # kwargs the caller supplied.
            inner_kwargs: dict[str, Any] = {
                "tools": tools,
                "json_output": json_output,
                "extra_create_args": extra,
                "cancellation_token": cancellation_token,
            }
            # tool_choice is autogen-core 0.7+; forward only when set
            # to a non-default value so older inner clients still work.
            if tool_choice != "auto":
                inner_kwargs["tool_choice"] = tool_choice
            inner_kwargs.update(kwargs)
            result = await self._inner.create(messages, **inner_kwargs)
        except BaseException as exc:
            # CancelledError + everything else.  Per review-standards
            # §2.2 POST fires for FAILURE / CANCELLED when a reservation
            # was issued; otherwise (DENY-then-fail unreachable but
            # defensively guarded) POST is skipped.
            if outcome.reservation_ids:
                outcome_kind = _classify_exception(exc)
                try:
                    await self._client.emit_llm_call_post(
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
                    )
                except Exception as post_exc:  # noqa: BLE001
                    # Best-effort: never mask the original inner
                    # exception that the AssistantAgent caller is about
                    # to see. Log + swallow; reservation TTL-sweeps.
                    log.warning(
                        "spendguard.integrations.autogen: emit_llm_call_post "
                        "failed on exception path (run_id=%s sig=%s err=%r) — "
                        "reservation will TTL-sweep",
                        ctx.run_id,
                        signature[:8],
                        post_exc,
                    )
            raise

        # ── POST — success commit ────────────────────────────────────
        # Per review-standards §2.2 POST MUST NOT fire when no
        # reservation exists (DENY path is unreachable here because
        # request_decision raises; this guards an ALLOW-with-empty-
        # reservation-ids corner case the projector might still emit
        # in degenerate test setups).
        total_tokens = _extract_total_tokens(result)
        if outcome.reservation_ids:
            await self._client.emit_llm_call_post(
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
            )
        return result

    # ─────────────────────────────────────────────────────────────────
    # create_stream — POC pass-through.
    # ─────────────────────────────────────────────────────────────────

    def create_stream(
        self,
        messages: list[Any],
        *,
        tools: Any = (),
        tool_choice: Any = "auto",
        json_output: Any = None,
        extra_create_args: dict[str, Any] | None = None,
        cancellation_token: Any = None,
        **kwargs: Any,
    ) -> AsyncIterator[Any]:
        """Pass-through to ``inner.create_stream`` (POC scope).

        Per design.md §5 / review-standards §3 stream gating brackets
        the WHOLE stream at the model boundary; intra-stream tool
        calls inherit the parent reservation. Per-chunk gating is
        explicitly out of scope for D24 — tracked as follow-on parity
        with ``openai_agents.stream_response``.

        The test ``test_create_stream_does_not_call_request_decision``
        enforces this is intentional behavior, not a regression.

        Forward-compat: same ``tool_choice`` + ``**kwargs`` shape as
        ``create()`` for newer autogen-core releases.
        """
        inner_kwargs: dict[str, Any] = {
            "tools": tools,
            "json_output": json_output,
            "extra_create_args": extra_create_args,
            "cancellation_token": cancellation_token,
        }
        if tool_choice != "auto":
            inner_kwargs["tool_choice"] = tool_choice
        inner_kwargs.update(kwargs)
        return self._inner.create_stream(messages, **inner_kwargs)

    async def close(self) -> None:
        """Pass-through to ``inner.close()`` — abstract in autogen-core 0.7+.

        Per review-standards §4 every ``ChatCompletionClient`` abstract
        or concrete method except ``create`` is pass-through; ``close``
        was promoted to an abstract method in autogen-core 0.7.x.
        Side-effect-free at the wrapper layer.

        Best-effort: tolerate inner clients that don't define ``close``
        (e.g. duck-typed test fakes) so the wrapper degrades gracefully.
        """
        inner_close = getattr(self._inner, "close", None)
        if inner_close is None:
            return
        result = inner_close()
        # ``close`` may be sync or async depending on the inner client.
        if hasattr(result, "__await__"):
            await result

    # ─────────────────────────────────────────────────────────────────
    # Pass-through introspection — required by AssistantAgent / token-
    # budget caps. Per review-standards §4 these methods carry NO side
    # effects (no sidecar calls, no caching, no metric emission).
    # ─────────────────────────────────────────────────────────────────

    def actual_usage(self) -> Any:
        """Return the inner client's ``actual_usage()``."""
        return self._inner.actual_usage()

    def total_usage(self) -> Any:
        """Return the inner client's ``total_usage()``."""
        return self._inner.total_usage()

    def count_tokens(self, messages: list[Any], *, tools: Any = ()) -> int:
        """Return the inner client's ``count_tokens(messages, tools=...)``."""
        return self._inner.count_tokens(messages, tools=tools)

    def remaining_tokens(
        self, messages: list[Any], *, tools: Any = ()
    ) -> int:
        """Return the inner client's ``remaining_tokens(messages, tools=...)``."""
        return self._inner.remaining_tokens(messages, tools=tools)

    @property
    def capabilities(self) -> Any:
        """Return the inner client's ``capabilities``."""
        return self._inner.capabilities

    @property
    def model_info(self) -> Any:
        """Return the inner client's ``model_info``."""
        return self._inner.model_info


__all__ = [
    "ClaimEstimator",
    "LINEAGE",
    "RunContext",
    "SpendGuardChatCompletionClient",
    "_classify_exception",
    "_extract_total_tokens",
    "_signature",
    "current_run_context",
    "run_context",
]
