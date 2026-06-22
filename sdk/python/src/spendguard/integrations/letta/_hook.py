# ruff: noqa: ANN401
"""``SpendGuardLettaClient`` — Letta ``LLMClientBase`` wrap adapter.

Implements the D26 design: subclass
``letta.llm_api.llm_client_base.LLMClientBase`` and wrap an ``inner``
client with PRE / POST sidecar hooks. ``send_llm_request()`` inserts
``RequestDecision(LLM_CALL_PRE)`` BEFORE the inner HTTP fires and
``emit_llm_call_post`` AFTER, propagating
``SUCCESS`` / ``FAILURE`` / ``CANCELLED`` based on the inner-call
outcome. ``send_llm_request_sync()`` brackets the same lifecycle for
older Letta code paths.

Letta (formerly MemGPT, ~22k stars, Apache-2.0) ships a per-provider
matrix of concrete subclasses of ``LLMClientBase`` —
``OpenAIClient`` / ``AnthropicClient`` / ``GoogleAIClient`` /
``DeepSeekClient``. SpendGuard wraps any instance polymorphically:
one wrapper covers all providers because gating sits at the ABC layer.

Lifecycle (per design.md §4)::

    Agent.step(...)
      → (internal reasoning loop, fans out to 3-4 LLM calls per turn)
        ↓ inner.send_llm_request(request_data, llm_config, tools, ...)
        ↓ SpendGuardLettaClient.send_llm_request
          ├─ ctx = current_run_context()       (reused from openai_agents)
          ├─ signature = blake2b(request_data | llm_config | tools |
          │                       force_tool_use)
          ├─ llm_call_id / decision_id derived from signature
          ├─ sidecar.RequestDecision(LLM_CALL_PRE)
          │     ALLOW    → continue
          │     DENY     → DecisionDenied propagates (no inner HTTP)
          │     DEGRADE  → SidecarUnavailable propagates (fail-closed)
          ├─ inner.send_llm_request(...)         provider HTTP
          └─ sidecar.emit_llm_call_post(SUCCESS|FAILURE|CANCELLED,
                                        estimated=usage.total_tokens)

Per review-standards §1 / §2:
  - Composition over inheritance for the inner client: the wrapper
    NEVER instantiates ``OpenAIClient`` / ``AnthropicClient`` /
    ``GoogleAIClient`` / any vendor SDK directly.
  - No ``super().__init__()`` call — ``LLMClientBase`` is an ABC
    whose init takes provider config the wrapper doesn't own. A super
    call would silently change inner-client behavior under upstream
    refactors.
  - ``__getattr__`` delegates ``llm_config`` / ``provider`` /
    ``build_request_data`` / ``convert_response_to_chat_completion`` /
    any future ``LLMClientBase`` additions to the inner client. No
    side effects in the pass-through path.
  - ``current_run_context`` is IMPORTED from
    ``spendguard.integrations.openai_agents``, NOT redefined.
    Polyglot agent stacks share one trace.
  - Sync path detects an active asyncio loop via
    ``asyncio.get_running_loop()`` and raises ``RuntimeError`` with a
    pointer at the async variant. Silent ``asyncio.run()`` re-entry
    is a release-blocking defect.

POC scope (per design.md §3 / review-standards §6):
  - Fail-closed is the only mode. No ``SPENDGUARD_LETTA_FAIL_OPEN``
    env knob.
  - ``request_data`` is treated as opaque — wrapper MUST NOT log or
    persist it outside the signature hash. Letta passes full message
    history into requests; payloads can contain user PII.
  - ``step_callback`` is documented as an inadequate alternative
    (turn-level gating over-grants reservations for multi-call turns)
    and is NOT shipped.
"""

from __future__ import annotations

import asyncio
import hashlib
import logging
from collections.abc import Callable
from typing import Any

from ...client import DecisionOutcome, SpendGuardClient
from ...ids import (
    derive_idempotency_key,
    derive_uuid_from_signature,
)
from ._errors import SpendGuardConfigError

# ─────────────────────────────────────────────────────────────────────
# letta import — required at module load. The package barrel
# ``__init__.py`` carries the install-hint ImportError guard; here we
# import resiliently so the unit suite can load ``_hook`` directly via
# package-path bypass (mirrors ``autogen/_hook.py`` pattern).
# ─────────────────────────────────────────────────────────────────────

try:  # pragma: no cover — branch chosen at import time
    from letta.llm_api.llm_client_base import (  # type: ignore[import-not-found]
        LLMClientBase as _LLMClientBase,
    )

    _LETTA_AVAILABLE = True
except ImportError:  # pragma: no cover — branch chosen at import time
    _LLMClientBase = None  # type: ignore[assignment, misc]
    _LETTA_AVAILABLE = False


# ─────────────────────────────────────────────────────────────────────
# Shared run-context — REUSED from openai_agents per review-standards
# §1.4. Polyglot agent stacks (OpenAI Agents → Letta → Pydantic-AI in
# one run) share a single trace because all adapters read the same
# module-level ``spendguard_run_context`` contextvar.
#
# Resilient import: ``..openai_agents`` ImportErrors when the
# ``[openai-agents]`` extra isn't installed (the barrel raises a
# helpful install-hint error). The shared contextvar mechanics
# (RunContext frozen dataclass + run_context asynccontextmanager +
# current_run_context lookup) don't actually depend on the openai-agents
# package — only the integration's wrapper class does. We import the
# three symbols defensively and fall back to a local mirror (same
# contextvar NAME so cross-framework sharing still works) when the
# barrel fails. Mirrors autogen/_hook.py exactly.
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
    from collections.abc import AsyncIterator
    from contextlib import asynccontextmanager
    from dataclasses import dataclass

    _RUN_CONTEXT: contextvars.ContextVar[RunContext | None] = (
        contextvars.ContextVar("spendguard_run_context", default=None)
    )

    @dataclass(frozen=True, slots=True)
    class RunContext:  # type: ignore[no-redef]
        """Per ``Agent.step()`` identifiers.

        Mirrors ``spendguard.integrations.openai_agents.RunContext``
        when the ``[openai-agents]`` extra is not installed. Same
        contextvar NAME means a parent LangChain / Pydantic-AI /
        AutoGen / Strands run shares the run_id with this adapter
        regardless of which fallback branch fired at import time.
        """

        run_id: str

    @asynccontextmanager
    async def run_context(ctx: RunContext) -> AsyncIterator[RunContext]:  # type: ignore[no-redef]
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
                "spendguard.integrations.letta called outside an active "
                "run_context(). Wrap your Agent.step invocation:\n\n"
                "    async with run_context(RunContext(run_id=...)):\n"
                "        response = await agent.step(message)\n"
            )
        return ctx


log = logging.getLogger("spendguard.integrations.letta")


# ─────────────────────────────────────────────────────────────────────
# Type aliases (public surface)
# ─────────────────────────────────────────────────────────────────────

ClaimEstimator = Callable[[Any], list[Any]]
"""Project ``BudgetClaim`` list from the Letta ``request_data`` payload.

Letta's ``send_llm_request(request_data, llm_config, tools, ...)``
receives an arbitrary provider-shaped request body (OpenAI / Anthropic
/ Gemini / DeepSeek — Letta builds it via
``inner.build_request_data(messages, llm_config, tools)``). The
estimator gets the body verbatim — it can inspect message content /
token counts / model name (via the caller's closure capture or via
``llm_config``) to project the reservation amount.

v1 contract: returns >= 1 claim. Empty list is a config error.
"""


# ─────────────────────────────────────────────────────────────────────
# Helpers (module-level so they're testable + reusable)
# ─────────────────────────────────────────────────────────────────────


def _signature(
    request_data: Any,
    llm_config: Any,
    tools: Any,
    force_tool_use: Any,
) -> str:
    """Stable 16-byte BLAKE2b hex hash over send_llm_request() inputs.

    Per review-standards §6 the signature includes
    ``llm_config`` — leaving it out lets a tenant flip model under the
    same reservation. ``force_tool_use`` and ``tools`` are signature
    inputs per design.md §5.

    blake2b-16 matches the OpenAI Agents / Agno / DSPy / AutoGen
    integrations' signature width so cross-framework ID derivation is
    symmetric.
    """
    text = (
        repr(request_data)
        + "|"
        + repr(llm_config)
        + "|"
        + repr(tools)
        + "|"
        + repr(force_tool_use)
    )
    return hashlib.blake2b(text.encode("utf-8"), digest_size=16).hexdigest()


def _extract_total_tokens(result: Any) -> int:
    """Extract ``total_tokens`` from a Letta ``ChatCompletionResponse``.

    Per review-standards §2.3 we prefer ``usage.total_tokens`` over
    ``prompt + completion``. Letta normalizes all provider responses
    through ``convert_response_to_chat_completion → ChatCompletionResponse``
    which carries OpenAI-style usage with ``total_tokens`` populated.

    Fallback to ``prompt_tokens + completion_tokens`` covers older
    Letta versions where ``total_tokens`` may be absent.

    When ``usage is None`` return ``0`` — the projector still commits
    the reserve with a zero estimated amount, which is the correct
    fail-soft signal for a provider that didn't return usage. Raising
    here would block the audit chain.
    """
    if result is None:
        return 0
    # Letta 0.16.x's low-level ``request_async`` returns the RAW provider
    # response as a ``dict`` (``result["usage"]["total_tokens"]``); the older
    # ``send_llm_request`` returns a ``ChatCompletionResponse`` object
    # (``result.usage.total_tokens``). Handle both.
    if isinstance(result, dict):
        usage_d = result.get("usage")
        if not isinstance(usage_d, dict):
            return 0
        total = usage_d.get("total_tokens")
        if isinstance(total, int) and total > 0:
            return total
        prompt = usage_d.get("prompt_tokens", 0) or 0
        completion = usage_d.get("completion_tokens", 0) or 0
        try:
            return int(prompt) + int(completion)
        except (TypeError, ValueError):
            return 0
    usage = getattr(result, "usage", None)
    if usage is None:
        return 0
    # Prefer total_tokens (Letta's normalized field per OpenAI shape).
    total = getattr(usage, "total_tokens", None)
    if isinstance(total, int) and total > 0:
        return total
    # Fallback for older Letta versions where total_tokens may be unset.
    prompt = getattr(usage, "prompt_tokens", 0) or 0
    completion = getattr(usage, "completion_tokens", 0) or 0
    try:
        return int(prompt) + int(completion)
    except (TypeError, ValueError):
        # Defensive: usage fields might be non-numeric on a custom
        # client. Better to report 0 than crash the audit chain.
        return 0


def _extract_provider_event_id(result: Any) -> str:
    """Extract the OpenAI-shaped ``id`` from a ``ChatCompletionResponse``.

    Letta normalizes provider responses to an OpenAI-shaped envelope
    via ``convert_response_to_chat_completion``, so the top-level
    ``id`` field is the provider's request/response identifier
    regardless of inner vendor. Letta 0.16.x's ``request_async`` returns
    the raw provider ``dict`` instead — handle both shapes.
    """
    if isinstance(result, dict):
        return str(result.get("id", "") or "")
    return str(getattr(result, "id", "") or "")


def _classify_exception(exc: BaseException) -> str:
    """Classify an inner-call exception into a POST outcome label.

    Per review-standards §2.2 we use ``type(exc).__name__ ==
    "CancelledError"`` (matches the D12 LiteLLM shim and D24 AutoGen
    patterns) to avoid cross-loop ``isinstance`` mismatches across
    ``asyncio`` / ``trio`` / ``anyio``. Letta uses asyncio by default
    but operators frequently wrap ``Agent.step`` in anyio-flavored
    runners; both raise a ``CancelledError`` from their respective
    runtimes.

    Returns:
      * ``"CANCELLED"`` when the exception type name matches.
      * ``"FAILURE"`` for every other exception.
    """
    if type(exc).__name__ == "CancelledError":
        return "CANCELLED"
    return "FAILURE"


# ─────────────────────────────────────────────────────────────────────
# Wrapper class — concrete subclass of LLMClientBase
# ─────────────────────────────────────────────────────────────────────

# Pick the real ABC when letta is available so Letta's framework-side
# isinstance checks pass; fall back to plain base class in unit tests
# where letta isn't installed (mirrors autogen/_hook.py).
if _LLMClientBase is not None:  # pragma: no cover — chosen at import
    _ClientBase: Any = _LLMClientBase
else:

    class _ClientBase:  # type: ignore[no-redef]
        """Unit-test stand-in for ``letta.llm_api.llm_client_base.LLMClientBase``.

        Mirrors the Letta 0.8 GA contract surface — ``send_llm_request`` /
        ``send_llm_request_sync`` plus the build/convert helpers. The
        rest is deliberately unstubbed so a future binding gets a clear
        AttributeError rather than silently no-oping.
        """


class SpendGuardLettaClient(_ClientBase):  # type: ignore[misc, valid-type]
    """Letta ``LLMClientBase`` wrapper that gates each call.

    Subclasses ``letta.llm_api.llm_client_base.LLMClientBase`` and
    overrides ``send_llm_request`` / ``send_llm_request_sync`` to
    insert PRE / POST sidecar hooks around the inner client call.
    Other ``LLMClientBase`` methods (``build_request_data``,
    ``convert_response_to_chat_completion``, ``llm_config``,
    ``provider``, etc.) pass through via ``__getattr__``.

    Per review-standards §1.2 the wrapper:
      - Takes ``inner: LLMClientBase`` (composition, not inheritance).
      - NEVER instantiates ``OpenAIClient`` / ``AnthropicClient`` /
        ``GoogleAIClient`` / ``DeepSeekClient`` directly.
      - Does NOT call ``super().__init__()`` — ABC init takes provider
        config the wrapper doesn't own.
      - Does NOT subclass any concrete vendor client — that breaks
        polymorphism across providers.

    Per review-standards §6 fail-closed is the only mode. No
    ``SPENDGUARD_LETTA_FAIL_OPEN`` env knob exists.

    Args:
        inner: A live ``LLMClientBase`` instance. Owned by the
            caller; not closed by the wrapper.
        client: A connected + handshook ``SpendGuardClient``.
        budget_id: Budget the reservation debits. REQUIRED.
        window_instance_id: Time-window scope on the budget. REQUIRED.
        unit: ``common_pb2.UnitRef`` describing the unit binding.
            REQUIRED.
        pricing: ``common_pb2.PricingFreeze`` for ledger lookup. REQUIRED.
        claim_estimator: ``(request_data) → list[BudgetClaim]``
            projector. REQUIRED — design.md §5 locks "No default
            ``claim_estimator``" because per-provider tokenizer
            mismatch makes a single default fragile.
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
        # NOTE: LLMClientBase init takes provider config the wrapper
        # doesn't own (verified against letta 0.8.0). Per
        # review-standards §1.2 we MUST NOT call super().__init__() —
        # silently changes inner-client behavior under upstream
        # refactors. ``__getattr__`` delegates llm_config / provider /
        # build_request_data / convert_response_to_chat_completion.
        if inner is None:
            raise SpendGuardConfigError(
                "SpendGuardLettaClient(inner=...) is required; "
                "got None. Pass a live letta.llm_api.llm_client_base."
                "LLMClientBase instance (e.g. OpenAIClient(...))."
            )
        if client is None:
            raise SpendGuardConfigError(
                "SpendGuardLettaClient(client=...) is required; got None."
            )
        if not budget_id:
            raise SpendGuardConfigError(
                "SpendGuardLettaClient(budget_id=...) required."
            )
        if not window_instance_id:
            raise SpendGuardConfigError(
                "SpendGuardLettaClient(window_instance_id=...) required."
            )
        unit_id = getattr(unit, "unit_id", "") if unit is not None else ""
        if not unit_id:
            raise SpendGuardConfigError(
                "SpendGuardLettaClient unit.unit_id required."
            )
        if claim_estimator is None:
            raise SpendGuardConfigError(
                "SpendGuardLettaClient(claim_estimator=...) is required; "
                "design.md §5 locks no default estimator because "
                "per-provider tokenizer mismatch makes a single default "
                "fragile (OpenAI cl100k_base vs Anthropic vs Gemini)."
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
    # __getattr__ delegation — review-standards §1.3
    # ─────────────────────────────────────────────────────────────────

    def __getattr__(self, name: str) -> Any:
        """Delegate any ``LLMClientBase`` attribute we don't override.

        Covers ``llm_config`` / ``provider`` / ``build_request_data`` /
        ``convert_response_to_chat_completion`` and any future
        ``LLMClientBase`` additions. ``__getattr__`` only fires after
        normal attribute lookup fails, so our explicit attrs
        (``_inner``, ``_client``, ``_budget_id``, ...) shadow correctly.

        Per review-standards §1.3 / §4 this MUST NOT add side effects
        (logging, metric emission, caching). The framework calls these
        methods often; side effects here are a Blocker.
        """
        # Avoid infinite recursion if _inner hasn't been set yet (e.g.
        # during pickling / copy.deepcopy where __getattr__ fires
        # before __init__). Returns AttributeError, which matches the
        # default protocol.
        if name == "_inner":
            raise AttributeError(name)
        try:
            inner = object.__getattribute__(self, "_inner")
        except AttributeError:
            raise AttributeError(name) from None
        return getattr(inner, name)

    # ─────────────────────────────────────────────────────────────────
    # send_llm_request — PRE before HTTP, POST after, fail-closed.
    # ─────────────────────────────────────────────────────────────────

    async def send_llm_request(
        self,
        request_data: Any,
        llm_config: Any,
        tools: Any = None,
        force_tool_use: bool = False,
        **kwargs: Any,
    ) -> Any:
        """Wrap ``inner.send_llm_request(...)`` with PRE/POST sidecar hooks.

        Order (verified by bytecode in tests):
          1. ``RequestDecision(LLM_CALL_PRE)`` — fail-closed.
          2. ``inner.send_llm_request(...)`` — provider HTTP, ONLY if
             PRE allowed.
          3. ``emit_llm_call_post(...)`` — fires for SUCCESS / FAILURE
             / CANCELLED when a reservation was issued.

        Per review-standards §2.1 ``RequestDecision`` is awaited
        BEFORE ``self._inner.send_llm_request(...)``. DENY raises
        ``DecisionDenied`` BEFORE any inner-client method is awaited,
        so ZERO HTTP requests reach the inner OpenAI/Anthropic
        transport on a DENY (asserted by
        ``test_real_letta_deny_path_zero_provider_http``).

        Forward-compat: ``**kwargs`` swallows any future parameters
        Letta adds without breaking older wrapper versions. Spec
        design.md §4 was authored against letta 0.8.0; the wrapper
        passes everything through to ``inner.send_llm_request``
        verbatim.
        """
        ctx = current_run_context()
        signature = _signature(request_data, llm_config, tools, force_tool_use)
        llm_call_id = str(
            derive_uuid_from_signature(signature, scope="llm_call_id")
        )
        decision_id = str(
            derive_uuid_from_signature(signature, scope="decision_id")
        )
        step_id = f"{ctx.run_id}:letta-call:{signature[:16]}"
        idempotency_key = derive_idempotency_key(
            tenant_id=self._client.tenant_id,
            session_id=self._client.session_id,
            run_id=ctx.run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )

        # Project budget claims via the operator-supplied estimator.
        projected_claims = self._claim_estimator(request_data)

        decision_context = {
            "integration": "letta",
        }
        # Capture the inner client class name for audit context — best
        # effort, never raises. Useful for dashboards grouping by
        # provider (OpenAIClient vs AnthropicClient vs GoogleAIClient).
        inner_type = type(self._inner).__name__
        if inner_type:
            decision_context["inner_client"] = inner_type

        # ── PRE — fail-closed reserve ────────────────────────────────
        # request_decision raises DecisionDenied / DecisionStopped /
        # ApprovalRequired derived classes on a STOP path; we let them
        # propagate unchanged. LLMClientBase has no framework-side
        # catch on the send_llm_request path (verified against letta
        # 0.8.0), so the raise reaches the Agent.step caller cleanly.
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
            result = await self._inner.send_llm_request(
                request_data,
                llm_config,
                tools=tools,
                force_tool_use=force_tool_use,
                **kwargs,
            )
        except BaseException as exc:
            # CancelledError + everything else.  Per review-standards
            # §2.2 POST fires for FAILURE / CANCELLED when a
            # reservation was issued; otherwise (DENY-then-fail
            # unreachable but defensively guarded) POST is skipped.
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
                    # exception that the Agent.step caller is about
                    # to see. Log + swallow; reservation TTL-sweeps.
                    log.warning(
                        "spendguard.integrations.letta: emit_llm_call_post "
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
        provider_event_id = _extract_provider_event_id(result)
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
                provider_event_id=provider_event_id,
                outcome="SUCCESS",
            )
        return result

    # ─────────────────────────────────────────────────────────────────
    # request_async — Letta 0.16.x low-level provider call (API drift).
    # ─────────────────────────────────────────────────────────────────

    async def request_async(
        self,
        request_data: Any,
        llm_config: Any,
    ) -> Any:
        """Gate Letta 0.16.x's ``LLMClientBase.request_async``.

        DRIFT: Letta 0.16 removed the old per-call
        ``send_llm_request(request_data, llm_config, tools, force_tool_use)``
        client surface this adapter was authored against (0.8.0). The
        canonical low-level provider call is now
        ``request_async(request_data: dict, llm_config) -> dict``. We gate it
        exactly like ``send_llm_request``: PRE ``request_decision``
        (fail-closed; DENY raises ``DecisionDenied`` BEFORE the inner call so
        ZERO provider HTTP fires) → ``inner.request_async`` → POST
        ``emit_llm_call_post``. The raw ``dict`` response is handled by the
        dict-aware ``_extract_total_tokens`` / ``_extract_provider_event_id``.
        """
        ctx = current_run_context()
        signature = _signature(request_data, llm_config, None, False)
        llm_call_id = str(
            derive_uuid_from_signature(signature, scope="llm_call_id")
        )
        decision_id = str(
            derive_uuid_from_signature(signature, scope="decision_id")
        )
        step_id = f"{ctx.run_id}:letta-call:{signature[:16]}"
        idempotency_key = derive_idempotency_key(
            tenant_id=self._client.tenant_id,
            session_id=self._client.session_id,
            run_id=ctx.run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )
        projected_claims = self._claim_estimator(request_data)
        decision_context = {"integration": "letta"}
        inner_type = type(self._inner).__name__
        if inner_type:
            decision_context["inner_client"] = inner_type

        # PRE — fail-closed reserve (raises DecisionDenied on a STOP).
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

        # Inner call — provider HTTP, ONLY if PRE allowed.
        try:
            result = await self._inner.request_async(request_data, llm_config)
        except BaseException as exc:
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
                    log.warning(
                        "spendguard.integrations.letta: emit_llm_call_post "
                        "failed on exception path (run_id=%s sig=%s err=%r) — "
                        "reservation will TTL-sweep",
                        ctx.run_id,
                        signature[:8],
                        post_exc,
                    )
            raise

        # POST — success commit.
        total_tokens = _extract_total_tokens(result)
        provider_event_id = _extract_provider_event_id(result)
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
                provider_event_id=provider_event_id,
                outcome="SUCCESS",
            )
        return result

    # ─────────────────────────────────────────────────────────────────
    # send_llm_request_sync — fail-closed against loop re-entry.
    # ─────────────────────────────────────────────────────────────────

    def send_llm_request_sync(
        self,
        request_data: Any,
        llm_config: Any,
        tools: Any = None,
        force_tool_use: bool = False,
        **kwargs: Any,
    ) -> Any:
        """Sync sibling — older Letta code still uses this.

        Per review-standards §3.1 detect an active asyncio loop via
        ``asyncio.get_running_loop()`` and raise ``RuntimeError`` with
        a message pointing at the async variant. Silent ``asyncio.run()``
        re-entry inside a loop is a release-blocking defect (nested
        event loops corrupt the reservation state of any in-flight
        ``send_llm_request`` on the parent loop).

        Outside an active loop (fresh thread, plain script), spin up
        an ``asyncio.run(...)`` to execute the async path. PRE/POST
        still fires identically.
        """
        # Detect a running loop — if inside one, refuse silent re-entry.
        running = False
        try:
            asyncio.get_running_loop()
        except RuntimeError:
            running = False
        else:
            running = True
        if running:
            raise RuntimeError(
                "spendguard.integrations.letta.SpendGuardLettaClient."
                "send_llm_request_sync called from inside an active asyncio "
                "loop. Use `await client.send_llm_request(...)` instead — "
                "the async variant is the canonical Letta 0.8+ path."
            )
        return asyncio.run(
            self.send_llm_request(
                request_data,
                llm_config,
                tools=tools,
                force_tool_use=force_tool_use,
                **kwargs,
            )
        )


def wrap_llm_client(
    *,
    inner: Any,
    client: SpendGuardClient,
    budget_id: str,
    window_instance_id: str,
    unit: Any,
    pricing: Any,
    claim_estimator: ClaimEstimator,
    route: str = "llm.call",
) -> SpendGuardLettaClient:
    """Factory — wrap any Letta ``LLMClientBase`` subclass instance.

    Convenience over calling ``SpendGuardLettaClient(...)`` directly so
    operator code reads:

        guarded = wrap_llm_client(inner=OpenAIClient(...), ...)

    instead of the longer class name. Identical semantics to the
    constructor; same arg names, same validation.
    """
    return SpendGuardLettaClient(
        inner=inner,
        client=client,
        budget_id=budget_id,
        window_instance_id=window_instance_id,
        unit=unit,
        pricing=pricing,
        claim_estimator=claim_estimator,
        route=route,
    )


__all__ = [
    "ClaimEstimator",
    "RunContext",
    "SpendGuardLettaClient",
    "_classify_exception",
    "_extract_provider_event_id",
    "_extract_total_tokens",
    "_signature",
    "current_run_context",
    "run_context",
    "wrap_llm_client",
]
