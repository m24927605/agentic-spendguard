"""``SpendGuardLlamaIndexHandler`` — LlamaIndex ``CallbackManager`` gating.

Implements ``BaseCallbackHandler``: a single instance registered via
``Settings.callback_manager = CallbackManager([handler])`` gates every
``CBEventType.LLM`` event published by LlamaIndex provider integrations
(``llama-index-llms-openai``, ``-anthropic``, ``-gemini``,
``-bedrock-converse``).

Coverage is enforced at the LLM-event boundary, not at the model
subclass, so the SAME handler instance gates every LlamaIndex provider
backend without per-vendor monkey-patching. Non-LLM events
(``EMBEDDING`` / ``RETRIEVE`` / ``CHUNK`` / ``QUERY`` /
``NODE_PARSING`` / etc.) are explicitly filtered at handler entry — one
enum compare for 80%+ of events.

Lifecycle (per design.md §4):

    Settings.callback_manager = CallbackManager([handler])
      ↓ query_engine.query("...")
      ↓ LLM._llm_predict / _chat
      ↓ on_event_start(CBEventType.LLM, payload=..., event_id=...)
        → _on_llm_start
          → claim_estimator(payload) → BudgetClaim
          → client.request_decision(LLM_CALL_PRE)
            CONTINUE = stash by event_id
            DENY     = raise SpendGuardLlamaIndexDenied
      ↓ (if ALLOW) provider HTTP
      ↓ on_event_end(CBEventType.LLM, payload=..., event_id=...)
        → _on_llm_end
          → pop by event_id
          → client.emit_llm_call_post(SUCCESS, total_tokens=...)
          → cleanup state

Sync-from-async bridging:

  LlamaIndex's callback contract is fully synchronous — handlers are
  invoked from inside ``LLM._chat`` / ``LLM._achat`` via the
  ``CallbackManager.event(...)`` context manager. The SpendGuard client
  surface is async-only. We bridge sync→async via a per-handler
  background thread+loop owned by the handler instance: each callback
  schedules its coroutine onto the background loop and blocks on the
  ``Future.result()`` until completion. This works whether the LlamaIndex
  query was sync (``.query()``) or async (``.aquery()``) because the
  background loop is *separate* from the calling thread's loop.
"""

from __future__ import annotations

import asyncio
import contextvars
import hashlib
import logging
import threading
from collections.abc import Callable, Iterator, Mapping
from contextlib import contextmanager
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
    SpendGuardLlamaIndexDenied,
)
from ._options import LlamaIndexRunContext, _PendingCall

# LlamaIndex's callback surface. The barrel ``__init__.py`` carries the
# install-hint guard so unit tests bypassing the barrel (via direct
# ``importlib.import_module``) still load this module without
# ``llama-index-core`` installed. Internally we accept duck-typed shapes
# (``CBEventType`` is an enum-like with a ``LLM`` attribute, ``EventPayload``
# is an enum-like with ``MESSAGES`` / ``PROMPT`` / ``RESPONSE`` /
# ``SERIALIZED`` attributes).
try:  # pragma: no cover — branch chosen at import time
    from llama_index.core.callbacks.base_handler import (  # type: ignore[import-not-found]
        BaseCallbackHandler as _RealBaseCallbackHandler,
    )
    from llama_index.core.callbacks.schema import (  # type: ignore[import-not-found]
        CBEventType as _RealCBEventType,
    )
    from llama_index.core.callbacks.schema import (  # type: ignore[import-not-found]
        EventPayload as _RealEventPayload,
    )

    _LLAMAINDEX_AVAILABLE = True
except ImportError:  # pragma: no cover — branch chosen at import time
    _RealBaseCallbackHandler = None  # type: ignore[assignment, misc]
    _RealCBEventType = None  # type: ignore[assignment, misc]
    _RealEventPayload = None  # type: ignore[assignment, misc]
    _LLAMAINDEX_AVAILABLE = False


_LOGGER = logging.getLogger("spendguard.integrations.llamaindex")


# ─────────────────────────────────────────────────────────────────────
# Type aliases
# ─────────────────────────────────────────────────────────────────────

RunIdFn = Callable[[Mapping[str, Any]], str]
"""Override for deriving ``run_id`` from event metadata.

Receives the LlamaIndex event payload dict; returns a non-empty string
to use as ``RequestDecision.ids.run_id``. Return empty string to defer
to the standard ``trace_id`` → ``parent_id`` → derived UUID cascade.
"""

ClaimEstimator = Callable[[Mapping[str, Any]], list[Any]]
"""Project a list of ``BudgetClaim`` proto messages from an event payload.

Receives the LlamaIndex ``payload`` dict passed to ``on_event_start``.
Returns exactly one ``common_pb2.BudgetClaim`` (v1 contract); future
multi-claim batching deferred.
"""


# ─────────────────────────────────────────────────────────────────────
# Handler base class — pick real LlamaIndex ABC when available so the
# CallbackManager isinstance check succeeds; fall back to a plain base
# class in unit tests where ``llama-index-core`` isn't installed.
# ─────────────────────────────────────────────────────────────────────

if _RealBaseCallbackHandler is not None:  # pragma: no cover — chosen at import
    _HandlerBase = _RealBaseCallbackHandler
else:
    class _HandlerBase:  # type: ignore[no-redef]
        """Unit-test stand-in for ``BaseCallbackHandler``.

        Accepts the same constructor kwargs LlamaIndex's real base
        class requires so subclass ``super().__init__(...)`` calls
        match either path. The real handler's ``start_trace`` /
        ``end_trace`` slots remain abstract on the real base — we
        override them.
        """

        def __init__(
            self,
            event_starts_to_ignore: list[Any] | None = None,
            event_ends_to_ignore: list[Any] | None = None,
        ) -> None:
            self.event_starts_to_ignore = event_starts_to_ignore or []
            self.event_ends_to_ignore = event_ends_to_ignore or []

        def on_event_start(
            self,
            event_type: Any,  # noqa: ANN401
            payload: dict[str, Any] | None = None,
            event_id: str = "",
            parent_id: str = "",
            **kwargs: Any,  # noqa: ANN401
        ) -> str:
            return event_id

        def on_event_end(
            self,
            event_type: Any,  # noqa: ANN401
            payload: dict[str, Any] | None = None,
            event_id: str = "",
            **kwargs: Any,  # noqa: ANN401
        ) -> None:
            return None

        def start_trace(self, trace_id: str | None = None) -> None:
            return None

        def end_trace(
            self,
            trace_id: str | None = None,
            trace_map: dict[str, list[str]] | None = None,
        ) -> None:
            return None


# Resolve the LLM event-type sentinel.  When ``llama-index-core`` isn't
# installed, fall back to the documented string ``"llm"`` so unit tests
# can construct payload dicts with the same key the real enum yields
# via its ``.value``.
if _RealCBEventType is not None:  # pragma: no cover — chosen at import
    _LLM_EVENT = _RealCBEventType.LLM
else:
    _LLM_EVENT = "llm"


# Resolve EventPayload keys.  In unit tests these are plain strings;
# the real ``EventPayload`` is an Enum whose ``.value`` equals these
# strings — payload dicts are keyed by the enum member itself but enum
# members are hashable and dict lookups against ``EventPayload.MESSAGES``
# match the dict key. We use the real enum when present.
if _RealEventPayload is not None:  # pragma: no cover — chosen at import
    _PAYLOAD_MESSAGES = _RealEventPayload.MESSAGES
    _PAYLOAD_PROMPT = _RealEventPayload.PROMPT
    _PAYLOAD_RESPONSE = _RealEventPayload.RESPONSE
    _PAYLOAD_SERIALIZED = _RealEventPayload.SERIALIZED
else:
    _PAYLOAD_MESSAGES = "messages"  # type: ignore[assignment]
    _PAYLOAD_PROMPT = "prompt"  # type: ignore[assignment]
    _PAYLOAD_RESPONSE = "response"  # type: ignore[assignment]
    _PAYLOAD_SERIALIZED = "serialized"  # type: ignore[assignment]


# ─────────────────────────────────────────────────────────────────────
# Sync-from-async bridge — per-handler background event loop in a
# dedicated daemon thread. LlamaIndex callbacks are sync; SpendGuard
# client is async-only. We schedule each coroutine onto the background
# loop via ``run_coroutine_threadsafe`` and block on ``Future.result()``.
#
# Owning the loop here avoids depending on the calling thread's loop
# state — the bridge works equally well from sync code (``.query()``)
# and from an async ``.aquery()`` invocation (callbacks still execute
# synchronously per LlamaIndex's documented contract).
# ─────────────────────────────────────────────────────────────────────


class _AsyncBridge:
    """Per-handler background thread running an asyncio loop.

    Owns one daemon thread and one ``asyncio.AbstractEventLoop``. Each
    ``run(coro)`` call schedules the coroutine on the background loop
    and blocks on the resulting ``Future`` until completion. The loop
    survives the lifetime of the bridge instance; tear-down via
    ``close()`` stops the loop and joins the thread.

    Mirrors the standard "asyncio worker thread" pattern documented in
    PEP 3156 (the asyncio package design rationale §"calling async code
    from sync code without a running loop"). Cleanest known way to
    bridge a sync callback into an async client without nest_asyncio.
    """

    def __init__(self) -> None:
        self._loop: asyncio.AbstractEventLoop | None = None
        self._thread: threading.Thread | None = None
        self._ready = threading.Event()
        self._closed = False
        # Start-lock guards the lazy thread spin-up so concurrent
        # callers don't race past the `self._thread is None` check.
        self._start_lock = threading.Lock()

    def _start(self) -> None:
        """Spin up the background thread + loop on first use (lazy).

        Thread-safe: concurrent callers serialize on ``self._start_lock``
        and only the first caller spins up the thread; subsequent
        callers observe ``self._thread is not None`` after acquiring the
        lock and wait on the same ``self._ready`` event until the loop
        is visible on the bridge instance.
        """
        # Fast path: already started AND loop visible — no lock contention.
        if self._thread is not None and self._loop is not None:
            return
        with self._start_lock:
            # Double-check under the lock — another thread may have
            # initialised between the fast-path check and the acquire.
            if self._thread is None:

                def _run() -> None:
                    loop = asyncio.new_event_loop()
                    self._loop = loop
                    asyncio.set_event_loop(loop)
                    self._ready.set()
                    try:
                        loop.run_forever()
                    finally:
                        try:
                            loop.close()
                        except Exception:  # noqa: BLE001, S110 — shutdown best-effort
                            pass  # noqa: S110

                self._thread = threading.Thread(
                    target=_run,
                    name="spendguard-llamaindex-bridge",
                    daemon=True,
                )
                self._thread.start()
        # Outside the lock: every caller (start owner + concurrent
        # callers serialized through the same lock) blocks on the
        # ready event until the spawned thread has installed the loop.
        self._ready.wait(timeout=5.0)
        if self._loop is None:
            raise RuntimeError(
                "spendguard-llamaindex-bridge failed to start its asyncio "
                "loop within 5s; check thread allowance / resource limits."
            )

    def run(self, coro: Any) -> Any:  # noqa: ANN401 — async client returns vary
        """Run ``coro`` on the background loop; block on result.

        Args:
            coro: Awaitable scheduled onto the background loop.

        Returns:
            The coroutine's return value.

        Raises:
            Whatever the coroutine raises is re-raised in the calling
            thread (via ``Future.result()``'s exception propagation).
        """
        if self._closed:
            raise RuntimeError(
                "spendguard-llamaindex-bridge has been closed; "
                "construct a fresh handler."
            )
        self._start()
        assert self._loop is not None  # _start() guarantees this
        fut = asyncio.run_coroutine_threadsafe(coro, self._loop)
        return fut.result()

    def close(self) -> None:
        """Stop the loop + join the background thread."""
        if self._closed:
            return
        self._closed = True
        loop = self._loop
        thread = self._thread
        if loop is not None and not loop.is_closed():
            loop.call_soon_threadsafe(loop.stop)
        if thread is not None:
            thread.join(timeout=2.0)


# ─────────────────────────────────────────────────────────────────────
# Helpers
# ─────────────────────────────────────────────────────────────────────


def _resolve_messages(payload: Mapping[str, Any]) -> Any:  # noqa: ANN401
    """Pull the messages / prompt field from a LlamaIndex LLM payload.

    Prefers ``EventPayload.MESSAGES`` (chat path) over
    ``EventPayload.PROMPT`` (text completion path). Returns ``""``
    when neither is present.
    """
    msgs = payload.get(_PAYLOAD_MESSAGES)
    if msgs is not None:
        return msgs
    prompt = payload.get(_PAYLOAD_PROMPT)
    if prompt is not None:
        return prompt
    return ""


def _resolve_serialized_model(payload: Mapping[str, Any]) -> str:
    """Extract ``payload[EventPayload.SERIALIZED]["model"]`` defensively."""
    serialized = payload.get(_PAYLOAD_SERIALIZED)
    if isinstance(serialized, Mapping):
        model = serialized.get("model")
        if isinstance(model, str):
            return model
    return ""


def _extract_total_tokens(response: Any) -> int:  # noqa: ANN401 — provider shapes vary
    """Pull total token count from a LlamaIndex provider response.

    Extraction order matches design.md §5:

      1. OpenAI: ``response.raw["usage"]["total_tokens"]``
      2. Anthropic: ``response.raw["usage"]["input_tokens"] + ["output_tokens"]``
      3. Gemini: ``response.raw["usage_metadata"]["total_token_count"]``
      4. Bedrock Converse: ``response.raw["usage"]["inputTokens"] + ["outputTokens"]``
      5. Default: 0.
    """
    raw = getattr(response, "raw", None)
    if raw is None:
        return 0
    if isinstance(raw, Mapping):
        usage = raw.get("usage")
        if isinstance(usage, Mapping):
            # 1) OpenAI universal total
            total = usage.get("total_tokens")
            if isinstance(total, int) and total > 0:
                return total
            # 2) Anthropic split
            inp = usage.get("input_tokens")
            out = usage.get("output_tokens")
            if isinstance(inp, int) or isinstance(out, int):
                summed = int(inp or 0) + int(out or 0)
                if summed > 0:
                    return summed
            # 4) Bedrock Converse split
            binp = usage.get("inputTokens")
            bout = usage.get("outputTokens")
            if isinstance(binp, int) or isinstance(bout, int):
                summed = int(binp or 0) + int(bout or 0)
                if summed > 0:
                    return summed
        # 3) Gemini metadata
        meta = raw.get("usage_metadata")
        if isinstance(meta, Mapping):
            total = meta.get("total_token_count")
            if isinstance(total, int) and total > 0:
                return total
    return 0


def _extract_provider_event_id(response: Any) -> str:  # noqa: ANN401 — provider shapes vary
    """Pull provider-reported event id from a LlamaIndex response.

    Most providers expose ``response.raw["id"]`` (OpenAI / Anthropic).
    Bedrock Converse uses ``response.raw["response_id"]``. Returns empty
    string when neither is present so the commit row always fires.
    """
    raw = getattr(response, "raw", None)
    if isinstance(raw, Mapping):
        rid = raw.get("id") or raw.get("response_id")
        if isinstance(rid, str):
            return rid
    return ""


# ─────────────────────────────────────────────────────────────────────
# Main handler class
# ─────────────────────────────────────────────────────────────────────


class SpendGuardLlamaIndexHandler(_HandlerBase):  # type: ignore[misc, valid-type]
    """LlamaIndex ``BaseCallbackHandler`` that gates ``CBEventType.LLM`` events.

    Drop-in via::

        Settings.callback_manager = CallbackManager([handler])

    Filters all non-LLM events at handler entry. Per-event state keyed
    by ``event_id`` in ``self._state`` survives between
    ``on_event_start`` (reserve) and ``on_event_end`` (commit). The
    same handler instance gates every LlamaIndex provider backend
    (OpenAI / Anthropic / Gemini / Bedrock Converse) — vendor
    detection is by response shape, not class-name parsing.

    Sync-from-async: LlamaIndex's callback contract is synchronous;
    the SpendGuard client is async. The handler owns a per-instance
    background asyncio loop in a daemon thread and dispatches each
    callback's coroutine onto it via ``run_coroutine_threadsafe``.
    This avoids nest_asyncio and works equally for sync ``.query()``
    and async ``.aquery()`` invocations (the callback itself remains
    synchronous per LlamaIndex contract).

    Args:
        client: A connected + handshook ``SpendGuardClient``. Owned by
            the caller; not closed by the handler.
        budget_id: Budget the reservation debits.
        window_instance_id: Time-window scope on the budget.
        unit: ``common_pb2.UnitRef`` describing the unit binding.
        pricing: ``common_pb2.PricingFreeze`` for ledger lookup.
        claim_estimator: Optional projector from the event payload to a
            single-element BudgetClaim list. When ``None``, the default
            estimator (``_default_estimator.llamaindex_default_claim_estimator``)
            dispatches off the model name discovered in
            ``payload[EventPayload.SERIALIZED]["model"]``.
        run_id_fn: Optional override for deriving ``run_id`` from the
            event payload. When provided and returning a non-empty
            string, wins over ``self._trace_id`` and ``parent_id``.
    """

    def __init__(
        self,
        *,
        client: SpendGuardClient,
        budget_id: str,
        window_instance_id: str,
        unit: Any,  # noqa: ANN401 — common_pb2.UnitRef
        pricing: Any,  # noqa: ANN401 — common_pb2.PricingFreeze
        claim_estimator: ClaimEstimator | None = None,
        run_id_fn: RunIdFn | None = None,
    ) -> None:
        """Construct the handler and warm up the async bridge."""
        super().__init__(
            event_starts_to_ignore=[],
            event_ends_to_ignore=[],
        )
        if client is None:
            raise SpendGuardConfigError(
                "SpendGuardLlamaIndexHandler(client=...) is required; got None."
            )
        if not budget_id:
            raise SpendGuardConfigError(
                "SpendGuardLlamaIndexHandler(budget_id=...) required."
            )
        if not window_instance_id:
            raise SpendGuardConfigError(
                "SpendGuardLlamaIndexHandler(window_instance_id=...) required."
            )

        self._client = client
        self._budget_id = budget_id
        self._window_instance_id = window_instance_id
        self._unit = unit
        self._pricing = pricing
        self._run_id_fn = run_id_fn
        self._trace_id: str | None = None
        self._state: dict[str, _PendingCall] = {}

        # Sync bridge: lazily-started background loop owned by this
        # handler instance.
        self._bridge = _AsyncBridge()

        # Default claim estimator: load lazily so the import doesn't
        # fire when callers supply their own estimator. We hold a
        # factory cache keyed by model so a stream of mixed-model
        # events doesn't rebuild the encoder per call.
        self._explicit_estimator = claim_estimator
        self._default_estimator_cache: dict[
            str, Callable[[Mapping[str, Any]], list[Any]]
        ] = {}

    # ─────────────────────────────────────────────────────────────────
    # BaseCallbackHandler contract — sync entry points
    # ─────────────────────────────────────────────────────────────────

    def on_event_start(
        self,
        event_type: Any,  # noqa: ANN401 — LlamaIndex CBEventType is Enum / str
        payload: dict[str, Any] | None = None,
        event_id: str = "",
        parent_id: str = "",
        **kwargs: Any,  # noqa: ANN401 — ABC forwards arbitrary kwargs
    ) -> str:
        """Fire ``request_decision`` on ``CBEventType.LLM`` events.

        Non-LLM events are filtered with a single enum compare and
        early-return ``event_id`` unchanged (per LlamaIndex contract).
        """
        if event_type != _LLM_EVENT:
            return event_id
        self._on_llm_start(payload or {}, event_id, parent_id)
        return event_id

    def on_event_end(
        self,
        event_type: Any,  # noqa: ANN401 — LlamaIndex CBEventType is Enum / str
        payload: dict[str, Any] | None = None,
        event_id: str = "",
        **kwargs: Any,  # noqa: ANN401 — ABC forwards arbitrary kwargs
    ) -> None:
        """Fire ``emit_llm_call_post`` on ``CBEventType.LLM`` events."""
        if event_type != _LLM_EVENT:
            return
        self._on_llm_end(payload or {}, event_id)

    def start_trace(self, trace_id: str | None = None) -> None:
        """Capture trace id for ``run_id`` resolution.

        LlamaIndex's ``CallbackManager.start_trace_with_id(trace_id)``
        invokes this on every registered handler. We stash the id on
        the instance; ``on_event_start`` uses it as the ``run_id`` when
        no override is set.
        """
        self._trace_id = trace_id

    def end_trace(
        self,
        trace_id: str | None = None,
        trace_map: dict[str, list[str]] | None = None,
    ) -> None:
        """Clear stashed trace id only when the passed id matches.

        Mismatched ids are a no-op — LlamaIndex may end a different
        trace than the one we tracked when handlers are shared across
        ``CallbackManager`` instances.
        """
        if trace_id is not None and trace_id == self._trace_id:
            self._trace_id = None

    # ─────────────────────────────────────────────────────────────────
    # PRE — reserve via request_decision
    # ─────────────────────────────────────────────────────────────────

    def _on_llm_start(
        self,
        payload: Mapping[str, Any],
        event_id: str,
        parent_id: str,
    ) -> None:
        """Reserve before the LlamaIndex provider HTTP call.

        CONTINUE → stash by ``event_id`` and return.
        DENY     → raise ``SpendGuardLlamaIndexDenied`` (no stash, no commit).

        The raise short-circuits the LLM call: LlamaIndex's
        ``CallbackManager.event(...)`` context manager propagates the
        exception out through the enclosing ``LLM.chat`` / ``LLM.predict``
        before the inner HTTP fires.
        """
        run_id = self._resolve_run_id(payload, parent_id)
        signature = self._signature_for(payload)
        llm_call_id = str(derive_uuid_from_signature(signature, scope="llm_call_id"))
        decision_id = str(derive_uuid_from_signature(signature, scope="decision_id"))
        step_id = f"{run_id}:li-call:{signature[:16]}"
        idempotency_key = derive_idempotency_key(
            tenant_id=self._client.tenant_id,
            session_id=self._client.session_id,
            run_id=run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )

        claims = self._estimate_claims(payload)

        try:
            outcome: DecisionOutcome = self._bridge.run(
                self._client.request_decision(
                    trigger="LLM_CALL_PRE",
                    run_id=run_id,
                    step_id=step_id,
                    llm_call_id=llm_call_id,
                    tool_call_id="",
                    decision_id=decision_id,
                    route="llm.call",
                    projected_claims=claims,
                    idempotency_key=idempotency_key,
                )
            )
        except DecisionDenied as exc:
            raise SpendGuardLlamaIndexDenied(
                reason_codes=list(getattr(exc, "reason_codes", []) or []),
                decision_id=str(getattr(exc, "decision_id", "") or ""),
            ) from exc

        if outcome.reservation_ids:
            self._state[event_id] = _PendingCall(
                reservation_id=outcome.reservation_ids[0],
                decision_id=outcome.decision_id,
                step_id=step_id,
                llm_call_id=llm_call_id,
                run_id=run_id,
                signature=signature,
            )

    # ─────────────────────────────────────────────────────────────────
    # POST — commit via emit_llm_call_post
    # ─────────────────────────────────────────────────────────────────

    def _on_llm_end(self, payload: Mapping[str, Any], event_id: str) -> None:
        """Commit / cleanup after the LlamaIndex provider HTTP completes.

        Pops state first so a no-pending event (DENY raised in start, or
        a misconfigured non-LLM event reaching us) is a silent no-op
        with no RPC fired.
        """
        pending = self._state.pop(event_id, None)
        if pending is None:
            # Either _on_llm_start never stashed (DENY raised) or this
            # is a no-op event. Silent return — no RPCs.
            return

        response = payload.get(_PAYLOAD_RESPONSE)
        total_tokens = _extract_total_tokens(response)
        provider_event_id = _extract_provider_event_id(response)

        try:
            self._bridge.run(
                self._client.emit_llm_call_post(
                    run_id=pending.run_id,
                    step_id=pending.step_id,
                    llm_call_id=pending.llm_call_id,
                    decision_id=pending.decision_id,
                    reservation_id=pending.reservation_id,
                    provider_reported_amount_atomic="",
                    estimated_amount_atomic=str(total_tokens),
                    unit=self._unit,
                    pricing=self._pricing,
                    provider_event_id=provider_event_id,
                    outcome="SUCCESS",
                )
            )
        except SpendGuardError as exc:
            # Commit RPC failed — log and swallow so we don't surface as
            # a runtime error to the LlamaIndex caller. The reservation
            # will be reaped by sidecar TTL sweep.
            _LOGGER.warning(
                "spendguard: llamaindex commit RPC failed for event_id=%s "
                "err=%r; reservation will TTL-sweep.",
                event_id,
                exc,
            )

    # ─────────────────────────────────────────────────────────────────
    # Signature + run_id resolution
    # ─────────────────────────────────────────────────────────────────

    def _signature_for(self, payload: Mapping[str, Any]) -> str:
        """Hash the LlamaIndex payload's visible model + messages.

        ``blake2b(digest_size=16)`` yields a 32-char hex digest for
        symmetry with the LangChain prior. Identical
        ``(model, messages)`` → identical signature → identical derived
        ``decision_id`` / ``llm_call_id`` (deterministic retry).
        """
        model = _resolve_serialized_model(payload)
        messages = _resolve_messages(payload)
        body = f"{model}|{messages!r}"
        return hashlib.blake2b(body.encode("utf-8"), digest_size=16).hexdigest()

    def _resolve_run_id(
        self,
        payload: Mapping[str, Any],
        parent_id: str,
    ) -> str:
        """Resolve ``run_id`` per the design.md §5 cascade.

        Order: ``run_id_fn`` (when non-empty) → ``self._trace_id``
        (set via ``start_trace``) → ``parent_id`` (LlamaIndex event
        graph) → derived UUID from the payload signature.
        """
        if self._run_id_fn is not None:
            override = self._run_id_fn(payload)
            if override:
                return str(override)
        if self._trace_id:
            return self._trace_id
        if parent_id:
            return parent_id
        return str(
            derive_uuid_from_signature(
                self._signature_for(payload), scope="run_id"
            )
        )

    # ─────────────────────────────────────────────────────────────────
    # Claim estimator dispatch + cache
    # ─────────────────────────────────────────────────────────────────

    def _estimate_claims(self, payload: Mapping[str, Any]) -> list[Any]:
        """Project a BudgetClaim list from the payload.

        Caller-supplied ``claim_estimator`` wins when non-None. Otherwise
        we dispatch the default estimator off the model field in
        ``payload[EventPayload.SERIALIZED]["model"]`` and cache the
        resulting closure per model so a multi-model query engine
        doesn't rebuild encoders on every call.
        """
        if self._explicit_estimator is not None:
            return self._explicit_estimator(payload)
        model = _resolve_serialized_model(payload)
        estimator = self._default_estimator_cache.get(model)
        if estimator is None:
            from ..._proto.spendguard.common.v1 import (  # noqa: PLC0415
                common_pb2,  # type: ignore[attr-defined] # noqa: F401
            )
            from .._default_estimator import (  # noqa: PLC0415
                llamaindex_default_claim_estimator,
            )

            estimator = llamaindex_default_claim_estimator(
                budget_id=self._budget_id,
                window_instance_id=self._window_instance_id,
                unit=self._unit,
                model=model,
            )
            self._default_estimator_cache[model] = estimator
        return estimator(payload)

    # ─────────────────────────────────────────────────────────────────
    # Cleanup
    # ─────────────────────────────────────────────────────────────────

    def close(self) -> None:
        """Stop the background bridge loop + thread.

        Operators should call this on application shutdown. The bridge
        is daemon-threaded so unclean exits won't hang; explicit close
        is the clean path.
        """
        self._bridge.close()

    def __del__(self) -> None:
        """Best-effort bridge tear-down at GC time."""
        try:
            self._bridge.close()
        except Exception:  # noqa: BLE001, S110 — GC best-effort
            pass  # noqa: S110


# ─────────────────────────────────────────────────────────────────────
# Optional run-context binding — parity with LangChain / Strands priors.
# LlamaIndex's event graph already carries event_id end-to-end so this
# is OPTIONAL; use only when bridging a LlamaIndex query to a
# cross-framework run_id (e.g. a parent LangChain run wrapping the
# query engine).
# ─────────────────────────────────────────────────────────────────────

_CURRENT_RUN_CONTEXT: contextvars.ContextVar[LlamaIndexRunContext | None] = (
    contextvars.ContextVar(
        "spendguard_llamaindex_run_context", default=None
    )
)


@contextmanager
def run_context(ctx: LlamaIndexRunContext) -> Iterator[LlamaIndexRunContext]:
    """Bind a ``LlamaIndexRunContext`` for the duration of the wrapped block.

    Usage::

        with run_context(LlamaIndexRunContext(run_id="my-run-1")):
            response = query_engine.query("...")
    """
    token = _CURRENT_RUN_CONTEXT.set(ctx)
    try:
        yield ctx
    finally:
        _CURRENT_RUN_CONTEXT.reset(token)


def current_run_context() -> LlamaIndexRunContext | None:
    """Return the active ``LlamaIndexRunContext`` or ``None`` if unbound."""
    return _CURRENT_RUN_CONTEXT.get()


__all__ = [
    "ClaimEstimator",
    "RunIdFn",
    "SpendGuardLlamaIndexHandler",
    "current_run_context",
    "run_context",
]
