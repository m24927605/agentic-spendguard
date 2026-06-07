# ruff: noqa: ANN401
"""Agno ``pre_hooks`` / ``post_hooks`` callable factories.

Implements the D22 design: two callable factories that emit
``inspect``-compatible async hook callables for
``Agent(pre_hooks=[pre()], post_hooks=[post()])``. Coverage is enforced
at the agent-runtime boundary so the SAME factory pair gates every Agno
``Model`` provider (OpenAIChat / Claude / Gemini / Groq / xAI /
DeepSeek / ...) with one registration — no per-vendor wrappers.

Lifecycle (per design.md §5)::

    Agent.arun(prompt)
      ↓ aexecute_pre_hooks(agent, run_input, run_context, session, ...)
      ↓ _pre_hook
        ├─ derive (run_id, signature, llm_call_id, decision_id, idempotency)
        ├─ client.request_decision(LLM_CALL_PRE)
        │    CONTINUE → stash by (run_id, signature)
        │    STOP / DENY → wrap into InputCheckError so Agno halts
        ├─ FIFO-evict oldest entry if map > 10k
        └─ return
      ↓ Agno → model.aresponse(...)
      ↓ aexecute_post_hooks(agent, run_output, run_context, session, ...)
      ↓ _post_hook
        ├─ pop (run_id, signature) slot
        ├─ extract usage from run_output.metrics
        │    SUCCESS → client.emit_llm_call_post(SUCCESS, total_tokens)
        │    RunError / missing metrics → emit_llm_call_post(PROVIDER_ERROR)
        └─ if slot missing → log warning + no-op (never commit w/o reserve)

DEVIATION-1 vs spec §6.5 (locked):
    Spec asserts "STOP / DENY raises DecisionDenied — Agno propagates the
    exception out of arun()". Agno's actual 2.x hook loop catches
    ``Exception`` and only re-raises ``InputCheckError`` /
    ``OutputCheckError`` (see ``agno.agent._hooks.aexecute_pre_hooks``
    line ~195 — ``except (InputCheckError, OutputCheckError) as e:
    raise e ... except Exception: log_exception(...)``). Without the
    wrap a DENY would be silently logged and the model would still be
    called, violating review-standards §3 "PRE before vendor SDK". The
    wrap raises ``InputCheckError(message, additional_data={...})``
    with the original ``DecisionDenied`` chained as ``__cause__`` so
    downstream callers still catch by ``DecisionDenied`` via the
    ``__cause__`` chain.

DEVIATION-2 vs spec §6.9 (locked):
    Spec asserts the post-hook async function declares ``(agent,
    run_response)`` literally. Agno 2.x ``aexecute_post_hooks`` builds
    its ``all_args`` dict with the key ``"run_output"`` (line ~281 of
    ``agno/agent/_hooks.py``). ``filter_hook_args`` then drops any
    parameter the closure declares that isn't in ``all_args``, so a
    closure declaring ``run_response`` would receive an empty kwargs
    set and the post never runs. Reality crystallised on
    ``run_output``; the closure follows reality. Tests in
    ``test_agno_pre_post.py`` assert the literal parameter name
    matches the Agno 2.x contract.
"""

from __future__ import annotations

import contextvars
import hashlib
import inspect
import logging
from collections import OrderedDict
from collections.abc import AsyncIterator, Callable
from contextlib import asynccontextmanager
from typing import Any

from ...client import DecisionOutcome, SpendGuardClient
from ...ids import (
    derive_idempotency_key,
    derive_uuid_from_signature,
)
from ._errors import (
    DecisionDenied,
    SpendGuardConfigError,
    SpendGuardError,
)
from ._options import RunContext, _InflightReservation

# Sidecar gRPC is awaited inside the async hook closures; the
# request_decision RPC raises DecisionDenied / DecisionStopped /
# ApprovalRequired derived classes on a STOP path. We re-export from
# `..._errors` so users only need one import path.


_LOGGER = logging.getLogger("spendguard.integrations.agno")

# Module-shared run-context. Same variable NAME as langchain.py:86 /
# pydantic_ai.py / openai_agents.py — cross-framework agents reuse one
# run_id. Per review-standards §1 a new context-var name would break
# multi-framework run_id sharing and is a Blocker.
_RUN_CONTEXT: contextvars.ContextVar[RunContext | None] = contextvars.ContextVar(
    "spendguard_run_context", default=None
)

# Bound the inflight map. 10k entries × ~200B ≈ ~2MB ceiling. FIFO
# eviction matches D04 §5 (LangChain) precedent. asyncio single-loop
# guarantee covers concurrent access.
_INFLIGHT_MAX = 10_000

# Module-shared inflight map. Same-process pre/post pairs see it
# without explicit wiring. Operators can still pass their own map via
# the ``inflight`` constructor kwarg for stricter isolation.
_SHARED_INFLIGHT: OrderedDict[tuple[str, str], _InflightReservation] = OrderedDict()


# ─────────────────────────────────────────────────────────────────────
# Optional `InputCheckError` import for the DENY wrap (DEVIATION-1).
# When `agno` is not installed (unit-test environment without the
# extra) we fall back to a duck-typed stand-in that subclasses
# `DecisionDenied` so downstream catches still work.
# ─────────────────────────────────────────────────────────────────────

try:  # pragma: no cover — branch chosen at import time
    from agno.exceptions import InputCheckError as _AgnoInputCheckError
    from agno.exceptions import OutputCheckError as _AgnoOutputCheckError

    _AGNO_AVAILABLE = True
except ImportError:  # pragma: no cover
    _AgnoInputCheckError = None  # type: ignore[assignment, misc]
    _AgnoOutputCheckError = None  # type: ignore[assignment, misc]
    _AGNO_AVAILABLE = False


@asynccontextmanager
async def run_context(ctx: RunContext) -> AsyncIterator[RunContext]:
    """Bind a ``RunContext`` for the duration of the wrapped block.

    Usage::

        async with run_context(RunContext(run_id="my-run-1")):
            response = await agent.arun("hello")
    """
    token = _RUN_CONTEXT.set(ctx)
    try:
        yield ctx
    finally:
        _RUN_CONTEXT.reset(token)


def current_run_context() -> RunContext:
    """Return the bound ``RunContext`` or raise a helpful ``RuntimeError``.

    The error message references the ``run_context(...)`` async
    context-manager so callers can self-correct without reading the
    module source.
    """
    ctx = _RUN_CONTEXT.get()
    if ctx is None:
        raise RuntimeError(
            "spendguard.integrations.agno hook fired outside an active "
            "run_context(). Wrap your Agent.arun call:\n\n"
            "    async with run_context(RunContext(run_id=...)):\n"
            "        await agent.arun(prompt)\n"
        )
    return ctx


# ─────────────────────────────────────────────────────────────────────
# Type aliases
# ─────────────────────────────────────────────────────────────────────

ClaimEstimator = Callable[[Any, Any], list[Any]]
"""``(agent, run_input) → list[BudgetClaim]``.

``run_input`` is whatever the caller passed to ``agent.arun(...)``:
``str | list[Message] | dict | RunInput``.
"""

CallSignatureFn = Callable[[Any, Any], str]
"""``(agent, run_input) → 32-hex content signature``."""


def _default_call_signature(agent: Any, run_input: Any) -> str:
    """Hash (``model.id`` || visible ``run_input``) into a 32-char hex digest.

    Per review-standards §2 the hash MUST include the model id —
    omitting it would let two different models in the same run collide
    on the same inflight slot.

    blake2b-16 matches the LangChain integration's signature width so
    downstream ID derivation is symmetric.
    """
    model_id = getattr(getattr(agent, "model", None), "id", "") or ""
    # Agno's RunInput model exposes `input_content` which is what the
    # caller actually passed to `arun(...)`. Fall back to repr for
    # arbitrary shapes.
    visible = getattr(run_input, "input_content", run_input)
    payload = f"{model_id}\n{visible!s}"
    return hashlib.blake2b(payload.encode("utf-8"), digest_size=16).hexdigest()


def _coerce_str(value: Any) -> str:
    """Return ``str(value)`` even when ``__str__`` is broken.

    Defensive: arbitrary user-passed run_input shapes (custom objects)
    might raise from ``__str__``; the signature derivation MUST NOT
    raise — the worst-case must be a stable empty-ish digest.
    """
    try:
        return str(value)
    except Exception:  # noqa: BLE001
        return repr(value) if value is not None else ""


# ─────────────────────────────────────────────────────────────────────
# Pre-hook factory
# ─────────────────────────────────────────────────────────────────────


class SpendGuardAgnoPreHook:
    """Factory producing an ``async`` callable for ``Agent(pre_hooks=[...])``.

    Per review-standards §1 this is a callable factory, NOT a
    ``Model`` subclass. ``__call__()`` returns the closure that
    Agno's ``aexecute_pre_hooks`` invokes via signature injection.
    The closure declares the literal parameter name ``run_input``
    (and ``agent``) because Agno filters keyword arguments by name —
    see ``agno.utils.hooks.filter_hook_args``.

    Args:
        client: A connected + handshook ``SpendGuardClient``. Owned by
            the caller; not closed by the factory.
        budget_id: Budget the reservation debits. REQUIRED.
        window_instance_id: Time-window scope on the budget. REQUIRED.
        unit: ``common_pb2.UnitRef`` describing the unit binding. REQUIRED.
        pricing: ``common_pb2.PricingFreeze`` for ledger lookup. REQUIRED.
        claim_estimator: Optional ``(agent, run_input) → list[BudgetClaim]``
            projector. When ``None`` the constructor wires
            ``agno_default_claim_estimator`` from
            ``_default_estimator.py`` so the same instance handles
            multi-model ``Team`` agents (model resolved per call).
        call_signature_fn: Optional override of the signature hashing.
            Defaults to the blake2b-16 hash of
            ``agent.model.id || run_input``.
        route: ``request_decision.route``. Defaults to ``"llm.call"`` so
            dashboards group with the LangChain integration.
        inflight: Optional caller-owned inflight map for strict
            isolation between hook pairs. Defaults to the module-shared
            ``_SHARED_INFLIGHT``.
    """

    def __init__(
        self,
        *,
        client: SpendGuardClient,
        budget_id: str,
        window_instance_id: str,
        unit: Any,
        pricing: Any,
        claim_estimator: ClaimEstimator | None = None,
        call_signature_fn: CallSignatureFn | None = None,
        route: str = "llm.call",
        inflight: OrderedDict[tuple[str, str], _InflightReservation] | None = None,
    ) -> None:
        if client is None:
            raise SpendGuardConfigError(
                "SpendGuardAgnoPreHook(client=...) is required; got None."
            )
        if not budget_id:
            raise SpendGuardConfigError(
                "SpendGuardAgnoPreHook(budget_id=...) required."
            )
        if not window_instance_id:
            raise SpendGuardConfigError(
                "SpendGuardAgnoPreHook(window_instance_id=...) required."
            )
        unit_id = getattr(unit, "unit_id", "") if unit is not None else ""
        if not unit_id:
            raise SpendGuardConfigError(
                "SpendGuardAgnoPreHook unit.unit_id required "
                "(DESIGN.md §6)."
            )

        self._client = client
        self._budget_id = budget_id
        self._window_instance_id = window_instance_id
        self._unit = unit
        self._unit_id = unit_id
        self._pricing = pricing
        self._claim_estimator = claim_estimator
        self._call_signature_fn = call_signature_fn or _default_call_signature
        self._route = route
        self._inflight = (
            inflight if inflight is not None else _SHARED_INFLIGHT
        )

    def __call__(self) -> Callable[..., Any]:
        """Return the async hook closure for ``Agent(pre_hooks=[pre()])``."""
        if self._claim_estimator is None:
            # Lazy import: keeps the integration module importable
            # without the [agno] extra when only the type aliases are
            # used (e.g. for static checking).
            from .._default_estimator import agno_default_claim_estimator

            self._claim_estimator = agno_default_claim_estimator(
                budget_id=self._budget_id,
                window_instance_id=self._window_instance_id,
                unit=self._unit,
                model="",  # resolved per-call from agent.model.id
            )

        # Capture the bound-method references in the closure scope so
        # the hook signature stays a top-level ``(agent, run_input)``
        # rather than ``(self, agent, run_input)``. Agno's
        # ``inspect.signature`` introspection requires the visible
        # parameter NAMES to match the dict keys Agno builds.
        client = self._client
        unit = self._unit
        pricing = self._pricing
        claim_estimator = self._claim_estimator
        assert claim_estimator is not None  # appeases mypy after lazy-set
        call_signature_fn = self._call_signature_fn
        route = self._route
        inflight = self._inflight

        async def _pre_hook(agent: Any, run_input: Any) -> None:
            """Agno pre-hook: reserve before the model HTTP fires.

            Declares ``(agent, run_input)`` because Agno's
            ``aexecute_pre_hooks`` (see ``agno/agent/_hooks.py``)
            passes those names. ``filter_hook_args`` drops any
            parameter the closure declares that isn't in Agno's
            ``all_args`` — declaring ``run_input`` keeps the closure
            wired to whatever the caller passed to ``Agent.arun(...)``.
            """
            ctx = current_run_context()
            signature = call_signature_fn(agent, run_input)
            llm_call_id = str(
                derive_uuid_from_signature(signature, scope="llm_call_id")
            )
            decision_id = str(
                derive_uuid_from_signature(signature, scope="decision_id")
            )
            step_id = f"{ctx.run_id}:agno-call:{signature[:16]}"
            idempotency_key = derive_idempotency_key(
                tenant_id=client.tenant_id,
                session_id=client.session_id,
                run_id=ctx.run_id,
                step_id=step_id,
                llm_call_id=llm_call_id,
                trigger="LLM_CALL_PRE",
            )

            model = getattr(agent, "model", None)
            model_id = (
                str(getattr(model, "id", "") or "") if model is not None else ""
            )
            decision_context = {
                "integration": "agno",
                "model_backend": (
                    type(model).__name__ if model is not None else "unknown"
                ),
            }
            if model_id:
                decision_context["model_id"] = model_id

            try:
                outcome: DecisionOutcome = await client.request_decision(
                    trigger="LLM_CALL_PRE",
                    run_id=ctx.run_id,
                    step_id=step_id,
                    llm_call_id=llm_call_id,
                    tool_call_id="",
                    decision_id=decision_id,
                    route=route,
                    projected_claims=claim_estimator(agent, run_input),
                    idempotency_key=idempotency_key,
                    projected_unit=unit,
                    decision_context_json=decision_context,
                )
            except DecisionDenied as denied:
                # DEVIATION-1: wrap into InputCheckError so Agno
                # actually halts. The original DecisionDenied chains
                # via __cause__ so downstream catches still work.
                _raise_halt_for_deny(denied)

            # Stash for the matching post-hook.
            key = (ctx.run_id, signature)
            inflight[key] = _InflightReservation(
                signature=signature,
                reservation_ids=list(outcome.reservation_ids),
                decision_id=outcome.decision_id,
                llm_call_id=llm_call_id,
                step_id=step_id,
                unit=unit,
                pricing=pricing,
                model_id=model_id,
            )
            # Bounded FIFO eviction.
            while len(inflight) > _INFLIGHT_MAX:
                inflight.popitem(last=False)

        return _pre_hook


# ─────────────────────────────────────────────────────────────────────
# Post-hook factory
# ─────────────────────────────────────────────────────────────────────


class SpendGuardAgnoPostHook:
    """Factory producing an ``async`` callable for ``Agent(post_hooks=[...])``.

    Pair this with the matching ``SpendGuardAgnoPreHook``; both must
    share the same inflight map (the module-shared default suffices
    unless the caller is partitioning across multiple sidecars).

    Args:
        client: A connected + handshook ``SpendGuardClient``.
        unit: ``common_pb2.UnitRef`` for commit shape.
        pricing: ``common_pb2.PricingFreeze`` for commit shape.
        call_signature_fn: Same default + override semantics as the
            pre-hook. The post-hook re-derives the signature so it can
            find the inflight slot.
        inflight: Same default + override semantics as the pre-hook.
    """

    def __init__(
        self,
        *,
        client: SpendGuardClient,
        unit: Any,
        pricing: Any,
        call_signature_fn: CallSignatureFn | None = None,
        inflight: OrderedDict[tuple[str, str], _InflightReservation] | None = None,
    ) -> None:
        if client is None:
            raise SpendGuardConfigError(
                "SpendGuardAgnoPostHook(client=...) is required; got None."
            )
        unit_id = getattr(unit, "unit_id", "") if unit is not None else ""
        if not unit_id:
            raise SpendGuardConfigError(
                "SpendGuardAgnoPostHook unit.unit_id required."
            )
        self._client = client
        self._unit = unit
        self._pricing = pricing
        self._call_signature_fn = call_signature_fn or _default_call_signature
        self._inflight = (
            inflight if inflight is not None else _SHARED_INFLIGHT
        )

    def __call__(self) -> Callable[..., Any]:
        """Return the async hook closure for ``Agent(post_hooks=[post()])``."""
        client = self._client
        call_signature_fn = self._call_signature_fn
        inflight = self._inflight

        async def _post_hook(agent: Any, run_output: Any) -> None:
            """Agno post-hook: commit with real usage or PROVIDER_ERROR.

            DEVIATION-2: Agno 2.x ``aexecute_post_hooks`` passes the
            run result under the key ``"run_output"`` (see
            ``agno/agent/_hooks.py`` line ~281). The closure declares
            ``run_output`` literally so Agno's signature filter binds
            it.
            """
            ctx = current_run_context()
            # The post-hook recomputes the signature off the SAME input
            # the pre-hook saw. Agno copies the run input onto
            # ``run_output.input`` on the way back (see
            # ``agno/agent/_hooks.py:150`` and the equivalent line in
            # ``aexecute_pre_hooks``). We prefer that field, falling
            # back to whatever input the run_output ships with.
            run_input = getattr(run_output, "input", None)
            if run_input is None:
                run_input = ""
            signature = call_signature_fn(agent, run_input)
            slot = inflight.pop((ctx.run_id, signature), None)
            if slot is None:
                # Either the pre-hook never fired (user instrumentation
                # bug) or two post-hooks fired for one pre. Log once
                # + no-op rather than emit a commit-without-reserve
                # event. Review-standards §3 lifecycle: "Post finding
                # no slot logs and no-ops".
                _LOGGER.warning(
                    "spendguard.agno: post_hook fired without matching pre "
                    "reservation (run_id=%s sig=%s)",
                    ctx.run_id,
                    signature[:8],
                )
                return
            if not slot.reservation_ids:
                # SKIPPED outcome on the PRE — no reservation was
                # ever issued so there's nothing to commit.
                return

            total_tokens, provider_event_id, outcome = _extract_usage(run_output)

            try:
                await client.emit_llm_call_post(
                    run_id=ctx.run_id,
                    step_id=slot.step_id,
                    llm_call_id=slot.llm_call_id,
                    decision_id=slot.decision_id,
                    reservation_id=slot.reservation_ids[0],
                    provider_reported_amount_atomic="",
                    estimated_amount_atomic=str(total_tokens),
                    unit=slot.unit,
                    pricing=slot.pricing,
                    provider_event_id=provider_event_id,
                    outcome=outcome,
                )
            except SpendGuardError as exc:
                # Agno's outer post-hook loop swallows non-Check
                # exceptions; rather than rely on that we log + return
                # so the caller's run still surfaces the model output.
                # The reservation will TTL-sweep on the ledger side.
                _LOGGER.warning(
                    "spendguard.agno: emit_llm_call_post failed for "
                    "run_id=%s sig=%s err=%r — reservation will TTL-sweep",
                    ctx.run_id,
                    signature[:8],
                    exc,
                )

        return _post_hook


# ─────────────────────────────────────────────────────────────────────
# Internal helpers
# ─────────────────────────────────────────────────────────────────────


def _raise_halt_for_deny(denied: DecisionDenied) -> None:
    """Wrap a ``DecisionDenied`` into an Agno-halting ``InputCheckError``.

    DEVIATION-1 implementation. The wrap targets ``InputCheckError``
    when ``agno.exceptions`` is importable; otherwise re-raises the
    original ``DecisionDenied`` so unit tests that don't install the
    extra still see the locked behaviour.

    The ``additional_data`` payload carries ``decision_id`` /
    ``reason_codes`` so callers can introspect the deny outcome via
    Agno's event stream without re-walking ``__cause__``.
    """
    if _AgnoInputCheckError is not None:
        additional: dict[str, Any] = {
            "spendguard": True,
            "decision_id": getattr(denied, "decision_id", "") or "",
            "reason_codes": list(
                getattr(denied, "reason_codes", None) or []
            ),
        }
        raise _AgnoInputCheckError(
            str(denied),
            additional_data=additional,
        ) from denied
    raise denied


def _extract_usage(run_output: Any) -> tuple[int, str, str]:
    """Return ``(total_tokens, provider_event_id, outcome)``.

    Agno's ``RunOutput`` exposes ``metrics`` (token counts) and
    ``status`` (run lifecycle state). When ``status`` is a failure
    state, OR when ``run_output`` itself is ``None`` / falsy, we
    report ``PROVIDER_ERROR`` so the projector releases the
    reservation; the success path reports ``SUCCESS`` with whatever
    ``total_tokens`` (or ``input_tokens + output_tokens``) the
    provider returned.

    Per design.md §6.5 (locked) the PROVIDER_ERROR commit is
    mandatory on RunError / missing metrics — silently no-op'ing
    would leak the reservation in the ledger.
    """
    if run_output is None:
        return 0, "", "PROVIDER_ERROR"

    # Status field carries the run lifecycle outcome. Agno 2.x uses
    # `RunStatus` enum with values like RUNNING / COMPLETED / PAUSED
    # / CANCELLED / ERROR. We treat anything that is NOT COMPLETED /
    # RUNNING as PROVIDER_ERROR (CANCELLED / ERROR / PAUSED-but-no
    # output).
    status = getattr(run_output, "status", None)
    status_name = (
        str(getattr(status, "value", status)).upper() if status is not None else ""
    )
    if getattr(run_output, "error", None):
        return 0, "", "PROVIDER_ERROR"
    # Failure states.
    if status_name in {"ERROR", "FAILED", "CANCELLED", "RUNERROR"}:
        return 0, "", "PROVIDER_ERROR"

    metrics = getattr(run_output, "metrics", None) or {}
    total = 0
    # Agno's metrics layout: a Metrics dataclass with attributes
    # `input_tokens` / `output_tokens` / `total_tokens` (and per-event
    # arrays). Dict shape (`{"total_tokens": ...}`) is also common
    # because some providers stream metrics through as raw dicts.
    total_attr = getattr(metrics, "total_tokens", None)
    if isinstance(total_attr, int) and total_attr > 0:
        total = total_attr
    elif isinstance(metrics, dict):
        v = metrics.get("total_tokens")
        if isinstance(v, list) and v:
            v = v[0]
        if isinstance(v, (int, float)) and v > 0:
            total = int(v)
    if total == 0:
        inp = (
            getattr(metrics, "input_tokens", None)
            if not isinstance(metrics, dict)
            else metrics.get("input_tokens")
        )
        out = (
            getattr(metrics, "output_tokens", None)
            if not isinstance(metrics, dict)
            else metrics.get("output_tokens")
        )
        for v in (inp, out):
            if isinstance(v, list) and v:
                v = v[0]
            if isinstance(v, (int, float)) and v > 0:
                total += int(v)

    provider_event_id = ""
    rid = (
        getattr(run_output, "run_id", None)
        or getattr(run_output, "response_id", None)
        or ""
    )
    if isinstance(rid, str):
        provider_event_id = rid

    # If we got here on a missing/zero metrics path BUT a healthy
    # status, still report SUCCESS with total=0 — the projector
    # commits the reserve as-is. The PROVIDER_ERROR path only fires
    # when the run itself failed.
    return total, provider_event_id, "SUCCESS"


def _hook_param_names(hook_callable: Any) -> list[str]:
    """Helper for tests: return the parameter names Agno will read.

    The ``inspect.signature`` reading is what
    ``agno.utils.hooks.filter_hook_args`` does at runtime, so this
    helper centralises the introspection used by test cases
    #16 / #17 (locked) for the named-parameter assertion.
    """
    try:
        sig = inspect.signature(hook_callable)
    except (TypeError, ValueError):
        return []
    return list(sig.parameters)


__all__ = [
    "CallSignatureFn",
    "ClaimEstimator",
    "RunContext",
    "SpendGuardAgnoPostHook",
    "SpendGuardAgnoPreHook",
    "current_run_context",
    "run_context",
]
