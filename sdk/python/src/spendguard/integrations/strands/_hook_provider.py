"""``SpendGuardStrandsHookProvider`` — AWS Strands Agents SDK gating.

Implements the Strands ``HookProvider`` contract: a single instance
registered with ``hooks=[provider]`` on an ``Agent``, with
``register_hooks(registry)`` binding callbacks to
``BeforeInvocationEvent`` (reserve) and ``AfterInvocationEvent``
(commit/release).

Coverage is enforced at the agent-runtime boundary, so the SAME
provider gates every Strands model backend with one instance: Bedrock
(default in AWS shops), OpenAI, Anthropic, Gemini, Ollama, and
LiteLLM. Multi-vendor support is asserted in the test matrix.

Lifecycle (per design.md §4):

    Agent.invoke_async(prompt)
      ↓ BeforeInvocationEvent(invocation_id, model, messages, tools)
      ↓ before_invocation
        ├─ estimator(invocation) → BudgetClaim
        ├─ sidecar.RequestDecision  ←── BEFORE provider HTTP
        │    CONTINUE = stash by invocation_id
        │    DENY     = raise DecisionDenied (HookExecutionError)
        │    DEGRADE  = raise SpendGuardDegradeBlocked (fail-closed default)
        └─ return
      ↓ Strands → model.invoke(...) → provider HTTP
      ↓ AfterInvocationEvent(invocation_id, result, exception?)
      ↓ after_invocation
        ├─ pop stash by invocation_id
        ├─ exception != None → emit_llm_call_post(FAILURE | CANCELLED)
        │    (do NOT mask original)
        └─ else → reconciler(invocation, result) → emit_llm_call_post(SUCCESS)

Stash is per-provider ``dict[str, _PendingInvocation]`` keyed by
Strands' ``invocation_id``. Strands fans parallel tool calls and
``asyncio.gather`` over invocations; invocation_id is the only stable
key (run-scoped keys collide on fan-out).

POC scope:
  - End-of-invocation commit only; intra-invocation streaming
    (``on_message``) inherits the parent reservation.
  - Tool-call gating is bundled into the parent estimator; per-tool
    budgets deferred to D20.1.
  - ``before_tool`` / ``after_tool`` hooks NOT registered in v1.
  - DEGRADE → fail-closed by default; ``SPENDGUARD_STRANDS_FAIL_OPEN=1``
    allows the call (dev only — no commit row will be produced).
"""

from __future__ import annotations

import asyncio
import contextvars
import logging
import os
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
    SidecarUnavailable,
    SpendGuardConfigError,
    SpendGuardDegradeBlocked,
    SpendGuardError,
)
from ._options import StrandsRunContext, _PendingInvocation

# Strands' typed hook surface. We import the abstract base + the two
# event classes the provider listens to. Like the ADK / MAF priors, the
# barrel ``__init__.py`` carries an import-time guard with the
# ``pip install 'spendguard-sdk[strands]'`` hint; internally we accept
# duck-typed shapes so the unit suite runs without strands-agents
# installed.
try:  # pragma: no cover — branch chosen at import time
    from strands.hooks import (  # type: ignore[import-not-found]
        AfterInvocationEvent as _RealAfterInvocationEvent,
    )
    from strands.hooks import (
        BeforeInvocationEvent as _RealBeforeInvocationEvent,
    )
    from strands.hooks import (
        HookProvider as _RealHookProvider,
    )
    from strands.hooks import (
        HookRegistry as _RealHookRegistry,
    )

    _STRANDS_AVAILABLE = True
except ImportError:  # pragma: no cover — branch chosen at import time
    _RealHookProvider = None  # type: ignore[assignment, misc]
    _RealHookRegistry = None  # type: ignore[assignment, misc]
    _RealBeforeInvocationEvent = None  # type: ignore[assignment, misc]
    _RealAfterInvocationEvent = None  # type: ignore[assignment, misc]
    _STRANDS_AVAILABLE = False


_LOGGER = logging.getLogger("spendguard.integrations.strands")


# ─────────────────────────────────────────────────────────────────────
# Type aliases
# ─────────────────────────────────────────────────────────────────────

ClaimEstimator = Callable[[Any], list[Any]]
"""Project a list of ``BudgetClaim`` proto messages from a Strands
``Invocation``. v1 contract: returns exactly 1 claim."""

ClaimReconciler = Callable[[Any, Any], list[Any]]
"""Receives ``(Invocation, InvocationResult)``; returns reconciled
``BudgetClaim`` list. v1 contract: returns exactly 1 claim. The result
carries normalized ``usage`` across providers per Strands GA shape."""


# ─────────────────────────────────────────────────────────────────────
# Provider base class — pick real ABC when available so isinstance and
# Strands' bus dispatch both work; fall back to a plain base class in
# unit tests where strands-agents isn't installed.
# ─────────────────────────────────────────────────────────────────────

if _RealHookProvider is not None:  # pragma: no cover — chosen at import
    _ProviderBase = _RealHookProvider
else:
    class _ProviderBase:  # type: ignore[no-redef]
        """Unit-test stand-in for ``strands.hooks.HookProvider``."""


# ─────────────────────────────────────────────────────────────────────
# Helpers
# ─────────────────────────────────────────────────────────────────────


def _validate_claim(claim: Any, *, source: str, expected_unit_id: str) -> None:
    """Surface developer mistakes in claim production at construction time.

    Mirrors litellm.py's ``_validate_claim`` so cross-adapter behaviour
    is symmetric. Raises ``SpendGuardConfigError`` rather than silently
    propagating a broken claim to the sidecar.
    """
    amount = getattr(claim, "amount_atomic", None)
    if amount is None or str(amount).strip() == "":
        raise SpendGuardConfigError(
            f"strands {source} returned a claim with empty amount_atomic; "
            "non-empty integer string required (DESIGN.md §6)."
        )
    unit = getattr(claim, "unit", None)
    unit_id = getattr(unit, "unit_id", "") if unit is not None else ""
    if not unit_id:
        raise SpendGuardConfigError(
            f"strands {source} returned a claim with no unit.unit_id; "
            "must match provider unit binding (DESIGN.md §6)."
        )
    if expected_unit_id and unit_id != expected_unit_id:
        raise SpendGuardConfigError(
            f"strands {source} claim.unit.unit_id={unit_id!r} does not "
            f"match provider unit={expected_unit_id!r}."
        )


def _extract_usage_tokens(result: Any) -> int:
    """Pull total token count from a Strands ``InvocationResult.usage``.

    Strands GA normalises across providers — `result.usage` is the
    canonical accessor. Extraction order matches the multi-backend
    shapes the project covers:

      1. ``result.usage.total_tokens`` (Bedrock / Anthropic / Cohere).
      2. ``result.usage.input_tokens + output_tokens`` (Anthropic shape
         when ``total_tokens`` is absent).
      3. ``result.usage.prompt_tokens + completion_tokens`` (OpenAI /
         LiteLLM normalised shape).
      4. ``result.usage.total_token_count`` (Gemini legacy field — some
         LiteLLM-via-Gemini paths still emit this).
      5. Default: 0.
    """
    usage = getattr(result, "usage", None)
    if usage is None:
        return 0
    # 1) Universal total
    total = getattr(usage, "total_tokens", None)
    if isinstance(total, int) and total > 0:
        return total
    # 2) Anthropic-style split
    inp = getattr(usage, "input_tokens", None) or 0
    out = getattr(usage, "output_tokens", None) or 0
    if isinstance(inp, int) and isinstance(out, int) and (inp + out) > 0:
        return int(inp) + int(out)
    # 3) OpenAI / LiteLLM normalised split
    inp = getattr(usage, "prompt_tokens", None) or 0
    out = getattr(usage, "completion_tokens", None) or 0
    if isinstance(inp, int) and isinstance(out, int) and (inp + out) > 0:
        return int(inp) + int(out)
    # 4) Gemini legacy
    total = getattr(usage, "total_token_count", None)
    if isinstance(total, int) and total > 0:
        return total
    return 0


def _extract_provider_event_id(result: Any) -> str:
    """Pull a provider-side event id from a Strands ``InvocationResult``.

    Strands GA normalises ``result.id`` across providers. Fall back to
    ``model_response.id`` (raw Bedrock InvokeModel surface) and
    ``response_id`` (LiteLLM-via-anthropic) before defaulting to empty
    so the commit row always fires.
    """
    rid = (
        getattr(result, "id", None)
        or getattr(result, "response_id", None)
        or getattr(getattr(result, "model_response", None), "id", None)
    )
    return str(rid) if rid else ""


def _model_backend_name(model: Any) -> str:
    """Return the Strands ``Model`` subclass name for the decision context.

    Strands' ``Invocation.model`` is one of ``BedrockModel`` /
    ``OpenAIModel`` / ``AnthropicModel`` / ``GeminiModel`` /
    ``OllamaModel`` / ``LiteLLMModel``. Falls back to ``"unknown"``.
    """
    if model is None:
        return "unknown"
    name = type(model).__name__
    return name or "unknown"


def _classify_exception(exc: Any) -> str:
    """Classify a Strands invocation exception into a release outcome.

    Returns the ``outcome`` string expected by
    ``SpendGuardClient.emit_llm_call_post``:

      * ``asyncio.CancelledError`` → ``"CANCELLED"`` (run aborted).
      * Anything else              → ``"FAILURE"``.
    """
    if isinstance(exc, asyncio.CancelledError):
        return "CANCELLED"
    return "FAILURE"


def _looks_like_before_event(event: Any) -> bool:
    """Duck-type sniff for ``BeforeInvocationEvent``.

    Strands GA exposes ``.invocation`` on both Before/After but only
    Before lacks ``.result``. Used as the unit-test fallback when
    strands-agents isn't installed.
    """
    return hasattr(event, "invocation") and not hasattr(event, "result")


def _looks_like_after_event(event: Any) -> bool:
    """Duck-type sniff for ``AfterInvocationEvent``."""
    return hasattr(event, "invocation") and hasattr(event, "result")


# ─────────────────────────────────────────────────────────────────────
# Main provider class
# ─────────────────────────────────────────────────────────────────────


class SpendGuardStrandsHookProvider(_ProviderBase):  # type: ignore[misc, valid-type]
    """Strands HookProvider that gates each agent invocation through SpendGuard.

    Subclass of ``strands.hooks.HookProvider`` when strands-agents is
    installed; falls back to a plain base in unit tests. Strands'
    bus calls ``register_hooks(registry)`` once at agent construction;
    the provider binds two callbacks (``before_invocation`` /
    ``after_invocation``) keyed by event type.

    Per-invocation stash:
      ``self._stash[invocation_id]`` carries the reservation companion
      ids between the PRE reserve and the matching POST commit/release.
      Strands assigns a fresh ``invocation_id`` per attempt (verified
      against 1.0 source — open question #1 in design.md §8 locked) so
      retries get their own reserve+commit pair (matches LiteLLM
      ADR-002). ``asyncio.gather``-style parallelism over invocations
      is stash-safe because each gets a distinct ``invocation_id``.

    Args:
        client: A connected + handshook ``SpendGuardClient``. Owned by
            the caller; not closed by the provider.
        budget_id: Budget the reservation debits.
        window_instance_id: Time-window scope on the budget.
        unit: ``common_pb2.UnitRef`` describing the unit binding.
        pricing: ``common_pb2.PricingFreeze`` for ledger lookup.
        claim_estimator: Optional projector from ``Invocation`` to a
            single-element BudgetClaim list. When ``None``, raises
            ``SpendGuardConfigError`` at first call — Strands' message
            shape is model-backend-specific, so v1 requires the caller
            to supply a tokenizer (parity with MAF's same constraint).
        claim_reconciler: REQUIRED. Receives
            ``(Invocation, InvocationResult)``; returns a single-element
            reconciled BudgetClaim list. Reads ``result.usage`` for
            real provider-reported tokens.
        fail_closed: When ``True`` (default), DEGRADE and sidecar
            unavailability raise ``SpendGuardDegradeBlocked`` /
            ``SidecarUnavailable``. ``SPENDGUARD_STRANDS_FAIL_OPEN=1``
            also forces fail-open (dev only).
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
        claim_reconciler: ClaimReconciler,
        fail_closed: bool = True,
        route: str = "llm.call",
    ) -> None:
        if client is None:
            raise SpendGuardConfigError(
                "SpendGuardStrandsHookProvider(client=...) is required; got None."
            )
        if not budget_id:
            raise SpendGuardConfigError(
                "SpendGuardStrandsHookProvider(budget_id=...) required."
            )
        if not window_instance_id:
            raise SpendGuardConfigError(
                "SpendGuardStrandsHookProvider(window_instance_id=...) required."
            )
        unit_id = getattr(unit, "unit_id", "") if unit is not None else ""
        if not unit_id:
            raise SpendGuardConfigError(
                "SpendGuardStrandsHookProvider unit.unit_id required "
                "(DESIGN.md §6)."
            )
        if claim_reconciler is None:
            raise SpendGuardConfigError(
                "SpendGuardStrandsHookProvider(claim_reconciler=...) is "
                "required; Strands' usage shape varies per backend so the "
                "caller must own the reconciliation projection."
            )

        self._client = client
        self._budget_id = budget_id
        self._window_instance_id = window_instance_id
        self._unit = unit
        self._unit_id = unit_id
        self._pricing = pricing
        self._claim_estimator = claim_estimator
        self._claim_reconciler = claim_reconciler
        self._fail_closed = fail_closed
        self._route = route
        self._fail_open_dev: bool = (
            os.environ.get("SPENDGUARD_STRANDS_FAIL_OPEN") == "1"
        )
        if self._fail_open_dev:
            _LOGGER.warning(
                "spendguard: SPENDGUARD_STRANDS_FAIL_OPEN=1 — fail-open; "
                "sidecar errors will allow agent invocations. DEV ONLY."
            )
        self._stash: dict[str, _PendingInvocation] = {}

    # ─────────────────────────────────────────────────────────────────
    # Strands HookProvider contract
    # ─────────────────────────────────────────────────────────────────

    def register_hooks(self, registry: Any) -> None:
        """Strands bus contract: bind callbacks to event types.

        Strands' ``HookRegistry.add_callback(EventClass, callback)``
        registers the callback to fire whenever an instance of
        ``EventClass`` is published on the bus. We register the two
        invocation-lifecycle hooks; tool / message events are not
        gated in v1 (design.md §3).
        """
        if _RealBeforeInvocationEvent is not None and _RealAfterInvocationEvent is not None:
            # Real Strands: bind via the typed event classes.
            registry.add_callback(_RealBeforeInvocationEvent, self.before_invocation)
            registry.add_callback(_RealAfterInvocationEvent, self.after_invocation)
            return
        # Unit-test path: register may be a stub. Attempt the bind but
        # tolerate any failure so SimpleNamespace-style registries don't
        # break the test fixtures.
        try:
            registry.add_callback("BeforeInvocationEvent", self.before_invocation)
            registry.add_callback("AfterInvocationEvent", self.after_invocation)
        except Exception as exc:  # noqa: BLE001
            _LOGGER.debug(
                "spendguard: register_hooks stub registry rejected bind: %r",
                exc,
            )

    # ─────────────────────────────────────────────────────────────────
    # before_invocation — reserve + stash
    # ─────────────────────────────────────────────────────────────────

    async def before_invocation(self, event: Any) -> None:
        """Reserve before each agent invocation.

        CONTINUE → stash by invocation_id and return.
        DENY     → raise ``DecisionDenied`` (Strands wraps to
                    ``HookExecutionError``).
        DEGRADE  → raise ``SpendGuardDegradeBlocked`` (fail-closed)
                    or warn + return (fail-open via env flag).
        """
        if self._claim_estimator is None:
            raise SpendGuardConfigError(
                "SpendGuardStrandsHookProvider.before_invocation called but "
                "claim_estimator is None; supply one at construction."
            )

        inv = getattr(event, "invocation", None)
        if inv is None:
            raise SpendGuardConfigError(
                "BeforeInvocationEvent.invocation missing — Strands GA "
                "contract requires it. Verify strands-agents>=1.0."
            )
        invocation_id = getattr(inv, "invocation_id", None)
        if not invocation_id:
            raise SpendGuardConfigError(
                "Strands Invocation has no invocation_id — pinned contract "
                "as of strands-agents>=1.0. Verify SDK version."
            )
        invocation_id = str(invocation_id)

        estimator_claims = self._claim_estimator(inv)
        if len(estimator_claims) != 1:
            raise SpendGuardConfigError(
                f"strands claim_estimator returned {len(estimator_claims)} "
                "claims; v1 contract requires exactly 1 (DESIGN.md §6)."
            )
        _validate_claim(
            estimator_claims[0],
            source="claim_estimator",
            expected_unit_id=self._unit_id,
        )

        # Resolve run / step / llm_call IDs.
        run_ctx = getattr(event, "_spendguard_run_context", None)
        if run_ctx is None:
            run_ctx = _CURRENT_RUN_CONTEXT.get()
        run_id = (
            str(run_ctx.run_id)
            if isinstance(run_ctx, StrandsRunContext)
            else str(derive_uuid_from_signature(
                f"strands:{invocation_id}", scope="run_id"
            ))
        )
        step_id = (
            run_ctx.step_id
            if isinstance(run_ctx, StrandsRunContext) and run_ctx.step_id
            else f"strands:{invocation_id[:16]}"
        )
        llm_call_id = str(
            derive_uuid_from_signature(
                f"strands:{invocation_id}", scope="llm_call_id"
            )
        )
        decision_id = str(
            derive_uuid_from_signature(
                f"strands:{invocation_id}", scope="decision_id"
            )
        )
        idempotency_key = derive_idempotency_key(
            tenant_id=self._client.tenant_id,
            session_id=self._client.session_id,
            run_id=run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )

        model = getattr(inv, "model", None)
        model_backend = _model_backend_name(model)
        model_id = str(getattr(model, "model_id", "") or "") if model is not None else ""

        # Decision context — captures the integration name + the
        # discovered model backend + model id so the SQL verify step can
        # confirm the right backend exercised the path.
        decision_context = {
            "integration": "strands",
            "model_backend": model_backend,
        }
        if model_id:
            decision_context["model_id"] = model_id

        try:
            outcome: DecisionOutcome = await self._client.request_decision(
                trigger="LLM_CALL_PRE",
                run_id=run_id,
                step_id=step_id,
                llm_call_id=llm_call_id,
                tool_call_id="",
                decision_id=decision_id,
                route=self._route,
                projected_claims=estimator_claims,
                idempotency_key=idempotency_key,
                projected_unit=self._unit,
                decision_context_json=decision_context,
            )
        except DecisionDenied:
            # Strands wraps to HookExecutionError; caller catches via
            # __cause__. Do NOT stash — no reservation to release.
            raise
        except SpendGuardError as exc:
            if self._fail_open_dev or not self._fail_closed:
                _LOGGER.warning(
                    "spendguard: strands fail-open — allowing invocation "
                    "despite sidecar error %r (DEV ONLY).",
                    exc,
                )
                return
            raise SidecarUnavailable(
                f"sidecar pre-invocation failed: {exc}"
            ) from exc

        decision_name = getattr(outcome, "decision", "")
        if decision_name == "DEGRADE":
            if self._fail_open_dev or not self._fail_closed:
                _LOGGER.warning(
                    "spendguard: strands DEGRADE under fail-open — allowing "
                    "invocation; commit will NOT fire (DEV ONLY)."
                )
                return
            raise SpendGuardDegradeBlocked(
                "sidecar returned DEGRADE; Strands provider fails closed."
            )

        if len(outcome.reservation_ids) != 1:
            raise SpendGuardConfigError(
                f"sidecar returned {len(outcome.reservation_ids)} "
                "reservations; v1 expects 1 (DESIGN.md §6)."
            )

        # Stash for the matching after_invocation.
        self._stash[invocation_id] = _PendingInvocation(
            decision_id=outcome.decision_id,
            reservation_ids=tuple(outcome.reservation_ids),
            llm_call_id=llm_call_id,
            run_id=run_id,
            step_id=step_id,
            estimator_amount_atomic=str(
                getattr(estimator_claims[0], "amount_atomic", "0") or "0"
            ),
            estimator_unit_id=self._unit_id,
            model_backend=model_backend,
            model_id=model_id,
        )

    # ─────────────────────────────────────────────────────────────────
    # after_invocation — commit / release + exception classification
    # ─────────────────────────────────────────────────────────────────

    async def after_invocation(self, event: Any) -> None:
        """Commit or release the reservation after each invocation.

        SUCCESS  → reconciler(inv, result) → emit_llm_call_post(SUCCESS).
        FAILURE  → emit_llm_call_post(FAILURE) with the original
                   estimator snapshot (best-effort cleanup); do NOT
                   mask the original exception.
        CANCELLED → emit_llm_call_post(CANCELLED) — caller cancelled.
        no-pending → no-op (PRE was skipped under fail-open or test).
        """
        inv = getattr(event, "invocation", None)
        invocation_id = ""
        if inv is not None:
            invocation_id = str(getattr(inv, "invocation_id", "") or "")
        # Fall back to event-level invocation_id if Strands exposes it
        # directly on AfterInvocationEvent (some 1.x point releases do).
        if not invocation_id:
            invocation_id = str(getattr(event, "invocation_id", "") or "")

        pending = self._stash.pop(invocation_id, None)
        if pending is None:
            # Either before_invocation was skipped (fail-open path) or
            # the event has no invocation_id. Silent no-op so we don't
            # surface as a runtime error.
            return

        exception = getattr(event, "exception", None)
        if exception is not None:
            await self._release_for_exception(pending, exception, invocation_id)
            return  # do NOT mask original exception

        result = getattr(event, "result", None)
        try:
            real_claims = self._claim_reconciler(inv, result)
        except Exception as rec_exc:  # noqa: BLE001
            _LOGGER.warning(
                "spendguard: strands claim_reconciler raised %r for "
                "invocation_id=%s; falling back to estimator snapshot.",
                rec_exc,
                invocation_id,
            )
            real_claims = []

        if real_claims:
            if len(real_claims) != 1:
                raise SpendGuardConfigError(
                    f"strands claim_reconciler returned {len(real_claims)} "
                    "claims; v1 contract requires exactly 1."
                )
            real_claim = real_claims[0]
            _validate_claim(
                real_claim,
                source="claim_reconciler",
                expected_unit_id=self._unit_id,
            )
            estimated_amount = str(getattr(real_claim, "amount_atomic", "0") or "0")
        else:
            estimated_amount = pending.estimator_amount_atomic

        # If the reconciler returned an empty/zero claim AND we have a
        # provider result, prefer the on-the-wire usage when present
        # (still respect the explicit reconciler value when non-zero).
        if estimated_amount in ("", "0") and result is not None:
            usage_total = _extract_usage_tokens(result)
            if usage_total > 0:
                estimated_amount = str(usage_total)

        provider_event_id = _extract_provider_event_id(result)

        try:
            await self._client.emit_llm_call_post(
                run_id=pending.run_id,
                step_id=pending.step_id,
                llm_call_id=pending.llm_call_id,
                decision_id=pending.decision_id,
                reservation_id=pending.reservation_ids[0],
                provider_reported_amount_atomic="",
                estimated_amount_atomic=estimated_amount,
                unit=self._unit,
                pricing=self._pricing,
                provider_event_id=provider_event_id,
                outcome="SUCCESS",
            )
        except SpendGuardError as exc:
            if self._fail_open_dev or not self._fail_closed:
                _LOGGER.warning(
                    "spendguard: strands commit failed under fail-open; "
                    "reservation will TTL-sweep invocation_id=%s err=%r",
                    invocation_id,
                    exc,
                )
                return
            raise

    async def _release_for_exception(
        self,
        pending: _PendingInvocation,
        exception: Any,
        invocation_id: str,
    ) -> None:
        """Best-effort release on a Strands-reported invocation exception.

        Emits ``emit_llm_call_post`` with the classified outcome
        (``FAILURE`` / ``CANCELLED``); the sidecar treats the call as
        a release-without-commit on those outcomes. If the release RPC
        itself errors, we log + swallow so we never mask the original
        exception Strands is about to propagate to the caller.
        """
        outcome = _classify_exception(exception)
        try:
            await self._client.emit_llm_call_post(
                run_id=pending.run_id,
                step_id=pending.step_id,
                llm_call_id=pending.llm_call_id,
                decision_id=pending.decision_id,
                reservation_id=pending.reservation_ids[0],
                provider_reported_amount_atomic="0",
                estimated_amount_atomic="0",
                unit=self._unit,
                pricing=self._pricing,
                provider_event_id="",
                outcome=outcome,
            )
        except SpendGuardError as rel_exc:
            _LOGGER.warning(
                "spendguard: strands release RPC failed for "
                "invocation_id=%s err=%r; reservation will TTL-sweep.",
                invocation_id,
                rel_exc,
            )
        except Exception as rel_exc:  # noqa: BLE001
            # Belt-and-braces: a transport-level failure (closed channel)
            # must NOT mask the original exception either.
            _LOGGER.warning(
                "spendguard: strands release best-effort raised %r for "
                "invocation_id=%s; ignoring (original exception preserved).",
                rel_exc,
                invocation_id,
            )

    # ─────────────────────────────────────────────────────────────────
    # Test surface — pending stash inspector
    # ─────────────────────────────────────────────────────────────────

    @property
    def pending_count(self) -> int:
        """Number of reservations currently awaiting after_invocation.

        Exposed for tests asserting stash isolation under concurrent
        ``asyncio.gather`` over multiple agent.invoke_async calls.
        Operators should treat this as a private metric.
        """
        return len(self._stash)


# ─────────────────────────────────────────────────────────────────────
# Optional run-context binding (parity with LangChain / MAF priors).
# Strands' bus carries invocation_id end-to-end so this is OPTIONAL; use
# only when bridging a Strands run to a cross-framework run_id (e.g. a
# parent LangChain run wrapping the Strands agent).
# ─────────────────────────────────────────────────────────────────────

_CURRENT_RUN_CONTEXT: contextvars.ContextVar[StrandsRunContext | None] = (
    contextvars.ContextVar(
        "spendguard_strands_run_context", default=None
    )
)


@asynccontextmanager
async def run_context(
    ctx: StrandsRunContext,
) -> AsyncIterator[StrandsRunContext]:
    """Bind a ``StrandsRunContext`` for the duration of the wrapped block.

    Usage::

        async with run_context(StrandsRunContext(run_id="my-run-1")):
            result = await agent.invoke_async(prompt="hello")
    """
    token = _CURRENT_RUN_CONTEXT.set(ctx)
    try:
        yield ctx
    finally:
        _CURRENT_RUN_CONTEXT.reset(token)


def current_run_context() -> StrandsRunContext | None:
    """Return the active ``StrandsRunContext`` or ``None`` if unbound."""
    return _CURRENT_RUN_CONTEXT.get()


__all__ = [
    "ClaimEstimator",
    "ClaimReconciler",
    "SpendGuardStrandsHookProvider",
    "current_run_context",
    "run_context",
]
