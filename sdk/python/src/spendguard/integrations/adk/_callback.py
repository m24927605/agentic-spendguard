"""``SpendGuardAdkCallback`` — single instance, two slots, both ADK callbacks.

Implements the Google ADK ``before_model_callback`` /
``after_model_callback`` contract:

  * PRE  — ``(callback_context, llm_request)`` → if ALLOW, return
    ``None`` (continue to model); if DENY, return a synthetic
    ``LlmResponse(error_code="SPENDGUARD_DENY", error_message=...)``
    which ADK treats as a terminal short-circuit (model is never
    called).
  * POST — ``(callback_context, llm_response)`` → on ALLOW, commit the
    reservation with the real usage from ``llm_response.usage_metadata``.
    On DENY (synthetic LlmResponse from PRE), this is a no-op.

Dispatch is by ``isinstance(payload, LlmRequest)`` first, then
``LlmResponse``. ADK passes both positional, so type discriminates
cleanly. Falls back to attribute-shape sniffing when the real ADK
classes can't be resolved (unit test fixtures use ``SimpleNamespace``
stubs that pass shape but not ``isinstance``).

Reservation handoff lives in ``callback_context.state`` — ADK constructs
a fresh ``CallbackContext`` per ``Runner.run_async`` invocation, so
concurrent runs are inherently isolated without a contextvar. Mirrors
the design.md §5 "single class, two callable slots" decision.

POC scope: streaming intra-turn gating, ``before_tool_callback`` /
``after_tool_callback`` wiring, and the TS / Go / Java ADK ports are
out of scope (D19 non-goals 3.2 / 3.3 / 3.1).
"""

from __future__ import annotations

import hashlib
import logging
import warnings
from collections.abc import Callable
from typing import Any

from ...client import DecisionOutcome, SpendGuardClient
from ...errors import DecisionDenied
from ...ids import (
    derive_idempotency_key,
    derive_uuid_from_signature,
)

# ADK callback types: optional at import time. The integration's public
# import path raises a helpful ImportError when the [adk] extra is not
# installed (see __init__.py module-level guard). Internally, we accept
# duck-typed stand-ins so the test suite can run without google-adk.
try:  # pragma: no cover — branch chosen at import time
    from google.adk.agents.callback_context import (  # type: ignore[import-not-found]
        CallbackContext as _RealCallbackContext,
    )
    from google.adk.models import (  # type: ignore[import-not-found]
        LlmRequest as _RealLlmRequest,
    )
    from google.adk.models import (
        LlmResponse as _RealLlmResponse,
    )

    _ADK_AVAILABLE = True
except ImportError:  # pragma: no cover — branch chosen at import time
    _RealCallbackContext = None  # type: ignore[assignment, misc]
    _RealLlmRequest = None  # type: ignore[assignment, misc]
    _RealLlmResponse = None  # type: ignore[assignment, misc]
    _ADK_AVAILABLE = False


_LOGGER = logging.getLogger(__name__)


# ─────────────────────────────────────────────────────────────────────
# Type aliases
# ─────────────────────────────────────────────────────────────────────

RunIdFn = Callable[[Any], str]
"""Override for deriving ``run_id`` from a ``CallbackContext``.

Default: ``ctx.invocation_id`` (ADK assigns one UUID per
``Runner.run_async``). Override for cross-framework correlation
(e.g. share a run_id with a parent LangChain run).
"""

ClaimEstimator = Callable[[Any], list[Any]]
"""Project a list of ``BudgetClaim`` proto messages from an ``LlmRequest``.

The default estimator dispatched from ``_default_estimator.py`` walks
``llm_request.contents`` for text parts and applies the family
tokenizer. Users can supply their own to e.g. include image tokens.
"""


# ─────────────────────────────────────────────────────────────────────
# Shape sniffing
# ─────────────────────────────────────────────────────────────────────


def _looks_like_request(payload: Any) -> bool:
    """Return True if ``payload`` quacks like an ADK ``LlmRequest``.

    Real ADK ``LlmRequest`` exposes ``.contents`` (list[Content]) and
    ``.model`` (str). Used as the unit-test fallback when google-adk
    isn't installed — production callers will be hitting the
    ``isinstance(payload, _RealLlmRequest)`` fast path instead.
    """
    return hasattr(payload, "contents") and not hasattr(payload, "usage_metadata")


def _looks_like_response(payload: Any) -> bool:
    """Return True if ``payload`` quacks like an ADK ``LlmResponse``.

    Distinguishes from ``LlmRequest`` by the presence of
    ``usage_metadata`` / ``error_code`` / ``candidates`` —
    fields that only the response shape carries.
    """
    return (
        hasattr(payload, "usage_metadata")
        or hasattr(payload, "error_code")
        or hasattr(payload, "candidates")
    )


# ─────────────────────────────────────────────────────────────────────
# Build deny response
# ─────────────────────────────────────────────────────────────────────


def _build_deny_response(exc: DecisionDenied) -> Any:
    """Build a synthetic ``LlmResponse`` representing a SpendGuard deny.

    Per design.md §5, the deny path uses ADK's documented short-circuit
    channel: a non-None ``LlmResponse`` with ``error_code`` set causes
    ADK to skip the model invocation entirely and treat the turn as
    terminal error. We do **not** raise — raising would surface as an
    ADK runtime error, breaking the user's own ``after_model_callback``
    chain (if any).

    Reason codes are comma-joined into ``error_message``; defaults to
    ``BUDGET_EXHAUSTED`` when ``reason_codes`` is empty (review-standards
    §4.1).
    """
    reasons = ",".join(getattr(exc, "reason_codes", None) or ["BUDGET_EXHAUSTED"])
    msg = f"SpendGuard denied LLM call: {reasons}"

    # Try to construct the real LlmResponse. If google-adk isn't
    # installed (unit-test path), fall back to a duck-typed namespace
    # carrying the same fields — production code paths route the real
    # type out of the gate.
    if _RealLlmResponse is not None:  # pragma: no branch
        try:
            return _RealLlmResponse(error_code="SPENDGUARD_DENY", error_message=msg)
        except TypeError:  # pragma: no cover — ADK 1.x kwargs stable
            # Future ADK release renamed kwargs. Surface a clear,
            # supportable error rather than silently failing the deny.
            _LOGGER.warning(
                "spendguard.integrations.adk: LlmResponse(error_code=...) "
                "construction failed; ADK release may have changed the "
                "short-circuit channel."
            )
            raise

    # Unit-test fallback: SimpleNamespace-style stand-in.
    from types import SimpleNamespace

    return SimpleNamespace(
        error_code="SPENDGUARD_DENY",
        error_message=msg,
        usage_metadata=None,
        response_id=None,
    )


# ─────────────────────────────────────────────────────────────────────
# Main callback class
# ─────────────────────────────────────────────────────────────────────


class SpendGuardAdkCallback:
    """Single instance, two slots — register to both ``before_`` and ``after_model_callback``.

    Stateless across requests; per-request reservation_id is stashed in
    ``callback_context.state["spendguard.reservation_id"]``. Multiple
    concurrent agent runs are safe because ADK constructs a fresh
    ``CallbackContext`` per ``Runner.run_async`` invocation.

    Integration shape::

        from google.adk.agents import LlmAgent
        from spendguard import SpendGuardClient
        from spendguard.integrations.adk import SpendGuardAdkCallback
        from spendguard._proto.spendguard.common.v1 import common_pb2

        client = SpendGuardClient(socket_path=..., tenant_id=...)
        await client.connect()
        await client.handshake()

        cb = SpendGuardAdkCallback(
            client=client,
            budget_id="b1",
            window_instance_id="w1",
            unit=common_pb2.UnitRef(...),
            pricing=common_pb2.PricingFreeze(...),
        )

        agent = LlmAgent(
            model="gemini-2.0-flash",
            before_model_callback=cb,
            after_model_callback=cb,
        )

    POC scope:
      - Streaming intra-turn gating: not supported. Gating is at turn
        boundary (parity with LangChain / openai-agents priors).
      - Tool callbacks: out of scope; gating sits at the model boundary.
      - Raising on deny: never. Deny is communicated through the
        documented ADK short-circuit channel (synthetic LlmResponse).
    """

    _STATE_RSV_KEY = "spendguard.reservation_id"
    _STATE_DECISION_KEY = "spendguard.decision_id"
    _STATE_STEP_KEY = "spendguard.step_id"
    _STATE_LLM_CALL_KEY = "spendguard.llm_call_id"
    _STATE_DENIED_KEY = "spendguard.denied"

    def __init__(
        self,
        *,
        client: SpendGuardClient,
        budget_id: str,
        window_instance_id: str,
        unit: Any,
        pricing: Any,
        claim_estimator: ClaimEstimator | None = None,
        run_id_fn: RunIdFn | None = None,
    ) -> None:
        """Bind a SpendGuard callback to an ADK ``LlmAgent``.

        Args:
            client: A connected ``SpendGuardClient`` (handshake must have
                completed). Owned by the caller; not closed by the callback.
            budget_id: Budget the reservation debits.
            window_instance_id: Time-window scope on the budget.
            unit: ``common_pb2.UnitRef`` proto message identifying the
                unit (token kind, model family).
            pricing: ``common_pb2.PricingFreeze`` proto message.
            claim_estimator: Optional callable projecting BudgetClaims
                from the inbound ``LlmRequest``. When ``None``, the
                default estimator dispatches off the request's ``model``
                string (Gemini / OpenAI via LiteLlm prefix strip /
                Anthropic / chars/4 fallback).
            run_id_fn: Optional callable deriving ``run_id`` from a
                ``CallbackContext``. Defaults to ``ctx.invocation_id``.
        """
        self._client = client
        self._budget_id = budget_id
        self._window_instance_id = window_instance_id
        self._unit = unit
        self._pricing = pricing
        self._run_id_fn = run_id_fn

        # Cache the resolved estimator at init time so we do exactly one
        # default-estimator lookup per callback instance — never per call
        # (review-standards.md §8 — perf review).
        if claim_estimator is None:
            from .._default_estimator import adk_default_claim_estimator

            self._claim_estimator: ClaimEstimator = adk_default_claim_estimator(
                budget_id=budget_id,
                window_instance_id=window_instance_id,
                unit=unit,
                model="",  # resolved lazily per-call from request.model
            )
        else:
            self._claim_estimator = claim_estimator

    # ─────────────────────────────────────────────────────────────────
    # Public dispatch
    # ─────────────────────────────────────────────────────────────────

    async def __call__(
        self,
        callback_context: Any = None,
        payload: Any = None,
        *,
        llm_request: Any = None,
        llm_response: Any = None,
        **_kwargs: Any,
    ) -> Any:
        """ADK callback entry point. Dispatches PRE vs POST by payload type.

        Returns ``None`` on the ALLOW PRE path (continue to model), a
        synthetic ``LlmResponse`` on the DENY PRE path (short-circuit),
        and ``None`` on the POST path.

        ADK 1.35+ invokes the model callbacks with KEYWORD arguments —
        ``before_model_callback(callback_context=..., llm_request=...)`` and
        ``after_model_callback(callback_context=..., llm_response=...)`` — so
        we accept ``llm_request`` / ``llm_response`` and normalise them to the
        single ``payload`` the dispatch below discriminates on (older /
        positional callers and the unit suite still pass ``payload``).

        Dispatch order:
          1. ``isinstance(payload, LlmRequest)`` — fast path when ADK
             is installed.
          2. ``isinstance(payload, LlmResponse)`` — fast path when ADK
             is installed.
          3. Shape-based fallback — for unit tests with SimpleNamespace
             stubs.
        """
        if payload is None:
            payload = llm_request if llm_request is not None else llm_response
        # Fast path: real ADK types available.
        if _RealLlmRequest is not None and isinstance(payload, _RealLlmRequest):
            return await self._before(callback_context, payload)
        if _RealLlmResponse is not None and isinstance(payload, _RealLlmResponse):
            await self._after(callback_context, payload)
            return None

        # Fallback: shape sniff (unit tests).
        if _looks_like_request(payload):
            return await self._before(callback_context, payload)
        if _looks_like_response(payload):
            await self._after(callback_context, payload)
            return None

        # Unknown payload — log and treat as a no-op rather than crash
        # the ADK runtime. Defensive: this branch is unreachable when
        # operators register the callback in the documented way.
        _LOGGER.warning(
            "SpendGuardAdkCallback received unknown payload type %r; "
            "ignoring. Expected LlmRequest or LlmResponse.",
            type(payload).__name__,
        )
        return None

    # ─────────────────────────────────────────────────────────────────
    # PRE / POST internals
    # ─────────────────────────────────────────────────────────────────

    async def _before(self, ctx: Any, req: Any) -> Any:
        """Reserve budget before the model call.

        ALLOW → stash reservation_id / decision_id / step_id /
        llm_call_id in ``ctx.state`` (companion keys) and return
        ``None`` (continue to model).

        DENY → set the ``spendguard.denied`` flag in ``ctx.state`` and
        return a synthetic ``LlmResponse`` carrying
        ``error_code="SPENDGUARD_DENY"`` (ADK terminates the turn).
        """
        run_id = self._run_id_fn(ctx) if self._run_id_fn else self._extract_invocation_id(ctx)
        signature = self._signature_for(req)
        llm_call_id = str(derive_uuid_from_signature(signature, scope="llm_call_id"))
        decision_id = str(derive_uuid_from_signature(signature, scope="decision_id"))
        step_id = f"{run_id}:adk-call:{signature[:16]}"
        idempotency_key = derive_idempotency_key(
            tenant_id=self._client.tenant_id,
            session_id=self._client.session_id,
            run_id=run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )

        try:
            outcome: DecisionOutcome = await self._client.request_decision(
                trigger="LLM_CALL_PRE",
                run_id=run_id,
                step_id=step_id,
                llm_call_id=llm_call_id,
                tool_call_id="",
                decision_id=decision_id,
                route="llm.call",
                projected_claims=self._claim_estimator(req),
                idempotency_key=idempotency_key,
            )
        except DecisionDenied as exc:
            # Stash deny marker so _after knows to skip commit. We do
            # not stash any reservation/decision id on the deny path —
            # the deny carries no reservation. Defense-in-depth.
            self._set_state(ctx, self._STATE_DENIED_KEY, True)
            return _build_deny_response(exc)

        # ALLOW — stash the four companion keys.
        if outcome.reservation_ids:
            self._set_state(ctx, self._STATE_RSV_KEY, outcome.reservation_ids[0])
        self._set_state(ctx, self._STATE_DECISION_KEY, outcome.decision_id)
        self._set_state(ctx, self._STATE_STEP_KEY, step_id)
        self._set_state(ctx, self._STATE_LLM_CALL_KEY, llm_call_id)
        return None  # Continue to model.

    async def _after(self, ctx: Any, resp: Any) -> None:
        """Commit the reservation with real usage from the model response.

        Skips when:
          * The ``spendguard.denied`` flag is set (PRE returned a
            synthetic LlmResponse and ADK echoed it here).
          * ``ctx.state`` lacks any of the four PRE-stashed keys
            (defensive: caller may have only registered the after
            callback).
        """
        if self._get_state(ctx, self._STATE_DENIED_KEY):
            return

        rsv_id = self._get_state(ctx, self._STATE_RSV_KEY)
        decision_id = self._get_state(ctx, self._STATE_DECISION_KEY)
        step_id = self._get_state(ctx, self._STATE_STEP_KEY)
        llm_call_id = self._get_state(ctx, self._STATE_LLM_CALL_KEY)
        if not (rsv_id and decision_id and step_id and llm_call_id):
            # Partial state → never reserved. Don't commit what we
            # didn't reserve. Silent return (no exception).
            return

        total_tokens = self._extract_total_tokens(resp)
        provider_event_id = self._extract_provider_event_id(resp)
        run_id = (
            self._run_id_fn(ctx) if self._run_id_fn else self._extract_invocation_id(ctx)
        )

        await self._client.emit_llm_call_post(
            run_id=run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            decision_id=decision_id,
            reservation_id=rsv_id,
            provider_reported_amount_atomic="",
            estimated_amount_atomic=str(total_tokens),
            unit=self._unit,
            pricing=self._pricing,
            provider_event_id=provider_event_id,
            outcome="SUCCESS",
        )

    # ─────────────────────────────────────────────────────────────────
    # State accessors — abstract over the two shapes of ctx.state we see
    # (dict and Pydantic-model-with-__getitem__).
    # ─────────────────────────────────────────────────────────────────

    @staticmethod
    def _set_state(ctx: Any, key: str, value: Any) -> None:
        """Write ``value`` to ``ctx.state[key]``.

        ADK ``CallbackContext.state`` is the documented per-invocation
        dict-like surface. ``__setitem__`` is the contract; fall back
        to ``setattr`` if a stub exposes attribute access instead
        (only used in unit tests).
        """
        state = getattr(ctx, "state", None)
        if state is None:
            return
        try:
            state[key] = value
            return
        except TypeError:  # pragma: no cover — attribute-stub fallback
            setattr(state, key.replace(".", "_"), value)

    @staticmethod
    def _get_state(ctx: Any, key: str) -> Any:
        """Read ``ctx.state[key]`` or ``None`` if missing."""
        state = getattr(ctx, "state", None)
        if state is None:
            return None
        try:
            # dict-like or mapping (covers `dict`, ADK `State`, etc.).
            return state.get(key) if hasattr(state, "get") else state[key]
        except (KeyError, TypeError):  # pragma: no cover
            return getattr(state, key.replace(".", "_"), None)

    @staticmethod
    def _extract_invocation_id(ctx: Any) -> str:
        """Default ``run_id`` derivation: ``ctx.invocation_id`` or empty."""
        return str(getattr(ctx, "invocation_id", "") or "")

    # ─────────────────────────────────────────────────────────────────
    # Signature derivation (stable across retries)
    # ─────────────────────────────────────────────────────────────────

    @staticmethod
    def _signature_for(req: Any) -> str:
        """Derive a stable 32-hex-char signature from request contents.

        Width matches the LangChain / openai_agents priors (blake2b
        digest_size=16). ``contents`` is coerced via ``repr`` for
        stability — ADK ``Content`` is a pydantic model with a
        deterministic ``repr``.
        """
        contents = repr(getattr(req, "contents", []))
        model = str(getattr(req, "model", "") or "")
        payload = f"{model}|{contents}"
        return hashlib.blake2b(payload.encode("utf-8"), digest_size=16).hexdigest()

    # ─────────────────────────────────────────────────────────────────
    # Usage extraction — by shape, not by model string
    # ─────────────────────────────────────────────────────────────────

    @staticmethod
    def _extract_total_tokens(resp: Any) -> int:
        """Pull the total token count from an ADK ``LlmResponse``.

        Extraction order (review-standards §4.3):
          1. Gemini canonical: ``usage_metadata.total_token_count``.
          2. Gemini split: ``prompt_token_count + candidates_token_count``.
          3. LiteLlm-wrapped OpenAI shape: ``usage_metadata.total_tokens``.
          4. Default: ``0``.

        Each branch is a positive-only check; we do not implicitly
        treat 0 as "present". Pure attribute access — no try/except,
        no exception swallowing.
        """
        usage = getattr(resp, "usage_metadata", None)
        if usage is None:
            return 0
        # 1) Gemini canonical
        total = getattr(usage, "total_token_count", None)
        if isinstance(total, int) and total > 0:
            return total
        # 2) Gemini split
        prompt = getattr(usage, "prompt_token_count", None) or 0
        cands = getattr(usage, "candidates_token_count", None) or 0
        if prompt or cands:
            return int(prompt) + int(cands)
        # 3) LiteLlm/OpenAI shape
        total = getattr(usage, "total_tokens", None)
        if isinstance(total, int):
            return total
        return 0

    @staticmethod
    def _extract_provider_event_id(resp: Any) -> str:
        """Pull a provider-side event id from an ADK ``LlmResponse``.

        ADK ≥ 1.2 attaches a ``response_id`` on ``LlmResponse``; older
        releases sometimes used ``id``. Falls back to the empty string —
        commit always fires.
        """
        rid = getattr(resp, "response_id", None) or getattr(resp, "id", None)
        return str(rid) if isinstance(rid, str) else ""


# ─────────────────────────────────────────────────────────────────────
# Default estimator factory wiring
# ─────────────────────────────────────────────────────────────────────

# We need to add `adk_default_claim_estimator` to the shared
# `_default_estimator.py` module. Since this file is loaded lazily by
# the callback's __init__, the wiring lives in `_default_estimator.py`
# itself (mirrors langchain / openai_agents). Imported lazily above.

__all__ = [
    "ClaimEstimator",
    "RunIdFn",
    "SpendGuardAdkCallback",
]


# Re-export for parity with _default_estimator helper.
def _emit_chars4_warning(model: str) -> None:  # pragma: no cover — only used in fallback
    """One-shot warning when the chars/4 fallback is used.

    Per design.md §5 the default estimator dispatches off model name;
    unknown models drop through to chars/4 with a single warn per
    (model, process). The estimator side of this logic lives in
    ``_default_estimator.adk_default_claim_estimator``; this helper is
    exported only so tests can suppress / capture the warning.
    """
    warnings.warn(
        f"spendguard.integrations.adk: unknown model {model!r}; "
        "using chars/4 fallback for token estimation.",
        UserWarning,
        stacklevel=2,
    )
