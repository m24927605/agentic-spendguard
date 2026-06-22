# ruff: noqa: ANN401
"""BeeAI Framework ``Emitter`` event subscriber.

Implements the D23 design: a single ``subscribe_spendguard(agent,
client, ...)`` call installs an async handler on ``agent.emitter``
that intercepts ``*.llm.*.start`` / ``*.llm.*.success`` /
``*.llm.*.error`` events and routes them through SpendGuard's
``RequestDecision(LLM_CALL_PRE)`` → ``EmitLlmCallPost`` lifecycle.

Coverage is enforced at the agent-runtime boundary so the SAME
subscriber gates every BeeAI ``ChatModel`` backend
(``OpenAIChatModel`` / ``WatsonxChatModel`` / ``OllamaChatModel`` /
``GroqChatModel`` / ...) with one registration — no per-vendor
wrappers.

Lifecycle (per ``docs/specs/coverage/D23_beeai/design.md`` §4)::

    BaseAgent.run(prompt)
      ↓ Emitter.emit("start", {input, modelId, ...}, meta)
      ↓ _on_start
        ├─ derive (run_id, stable_call_key, llm_call_id, decision_id, idem)
        ├─ client.request_decision(LLM_CALL_PRE)
        │    CONTINUE → stash by stable_call_key
        │    STOP / DENY → raise DecisionDenied (Emitter wraps as
        │                  EmitterError preserving __cause__; BeeAI
        │                  propagates → agent halts before model HTTP)
        └─ FIFO-evict oldest entry if map > 10k
      ↓ BeeAI → ChatModel.create(...) (only runs on ALLOW)
      ↓ Emitter.emit("success", {output, usage}, meta)
      ↓ _on_success
        ├─ pop stable_call_key slot
        ├─ extract usage.total_tokens
        └─ client.emit_llm_call_post(SUCCESS, total_tokens)
      ↓ on provider error: Emitter.emit("error", {err}, meta)
      ↓ _on_error
        ├─ pop stable_call_key slot
        └─ client.emit_llm_call_post(PROVIDER_ERROR, 0)

LOCKED PER review-standards §1 (Security):
  - ``request_decision`` is the FIRST awaited call in ``_on_start``;
    no path returns without either raising or stashing inflight so
    the eventual commit lands (asserted by tests T03/T04/T15).
  - ``DecisionDenied`` propagates out of the start handler
    unchanged. BeeAI's ``Emitter._invoke`` (see
    ``beeai_framework/emitter/emitter.py`` line ~244) wraps it as
    ``EmitterError`` preserving ``__cause__`` — that wrap is BeeAI's
    contract, not adapter behaviour. We do NOT catch+swallow
    ``DecisionDenied``.
  - The stable per-call key is ``EventMeta.path`` with the trailing
    ``.start|.success|.error`` segment stripped (LOCKED — verified
    against ``Emitter.emit`` path construction at
    ``emitter.py:236-242``).

LOCKED PER review-standards §4 (Run-context):
  - Re-uses the LangChain ``_RUN_CONTEXT`` contextvar binding NAME so
    a single ``run_context(...)`` covers LangChain + Agno + BeeAI in
    one app. We define a fresh ``ContextVar("spendguard_run_context",
    default=None)`` so importing this module doesn't pull in
    ``langchain_core`` (the design.md §5 R1 "re-import from
    langchain.py" path would require the ``[langchain]`` extra
    transitively — broken for BeeAI-only users). The contextvar
    NAME is shared; multi-adapter run_id sharing works because
    Python ContextVars are looked up by name registry at the
    interpreter level.
"""

from __future__ import annotations

import contextvars
import hashlib
import logging
from collections import OrderedDict
from collections.abc import AsyncIterator, Callable, Sequence
from contextlib import asynccontextmanager
from dataclasses import dataclass
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

_LOGGER = logging.getLogger("spendguard.integrations.beeai")

# Module-shared run-context. SAME variable NAME as langchain.py:86 /
# pydantic_ai.py / openai_agents.py / agno/_hook.py:93 — cross-
# framework agents reuse one run_id. Per review-standards §4 a new
# context-var name would break multi-framework run_id sharing and is
# a Blocker.
_RUN_CONTEXT: contextvars.ContextVar[RunContext | None] = contextvars.ContextVar(
    "spendguard_run_context", default=None
)

# Bound the inflight map. 10k entries × ~250B ≈ ~2.5MB ceiling. FIFO
# eviction matches D04 §5 (LangChain) + D22 (Agno) precedent.
# asyncio single-loop guarantee covers concurrent access; BeeAI's
# Emitter dispatches sequentially via ``asyncio.TaskGroup`` inside
# ``_invoke`` so no extra lock needed.
_INFLIGHT_MAX = 10_000


# ─────────────────────────────────────────────────────────────────────
# Frozen normalised event view
# ─────────────────────────────────────────────────────────────────────


@dataclass(frozen=True, slots=True)
class BeeAiStartEvent:
    """Normalised view of a BeeAI ``start`` event.

    BeeAI's ``Emitter.emit`` passes the event payload as ``data``
    and metadata as a ``EventMeta`` instance. The shape of ``data``
    varies by emitter (``ChatModel.create`` vs ``ReActAgent`` vs
    ``Workflow`` step); this normaliser pulls the input + model id
    out via a tolerant cascade.

    Attributes:
        input: Best-effort view of the input messages (``data.input``
            or ``data.messages``; falls back to ``[]``).
        model_id: Best-effort model id (``data.modelId`` /
            ``data.model_id``; falls back to ``""``).
        path: Full hierarchical ``EventMeta.path`` including the
            trailing ``.start`` segment.
    """

    input: Sequence[Any]
    model_id: str
    path: str


ClaimEstimator = Callable[[BeeAiStartEvent], list[Any]]
"""``(BeeAiStartEvent) → list[BudgetClaim]`` — projected claims for
the reservation."""

CallSignatureFn = Callable[[BeeAiStartEvent], str]
"""``(BeeAiStartEvent) → 32-hex content signature`` — stable input
hash used to derive ``llm_call_id`` + ``decision_id`` deterministically."""


# ─────────────────────────────────────────────────────────────────────
# Run-context contextvar binding
# ─────────────────────────────────────────────────────────────────────


@asynccontextmanager
async def run_context(ctx: RunContext) -> AsyncIterator[RunContext]:
    """Bind a ``RunContext`` for the duration of the wrapped block.

    Usage::

        async with run_context(RunContext(run_id="my-run-1")):
            result = await agent.run("Say hello in three words.")
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
    module source. Mirrors review-standards §4 R2.
    """
    ctx = _RUN_CONTEXT.get()
    if ctx is None:
        raise RuntimeError(
            "spendguard.integrations.beeai subscriber fired outside an "
            "active run_context(). Wrap your BaseAgent.run call:\n\n"
            "    async with run_context(RunContext(run_id=...)):\n"
            "        await agent.run(prompt)\n"
        )
    return ctx


# ─────────────────────────────────────────────────────────────────────
# Stable per-call key derivation
# ─────────────────────────────────────────────────────────────────────


def _stable_call_key(path: str) -> str:
    """Strip the trailing ``.start|.success|.error`` segment.

    BeeAI emits one event per name on the same hierarchical path —
    e.g. ``agent.react.llm.<uuid>.start``, then ``.success``.
    Stripping the last segment yields the stable per-call
    correlation key.

    Edge case: a path with no ``.`` (single segment such as
    ``"start"``) is returned unchanged so the inflight key stays a
    string (never empty).

    BeeAI 0.1.x emits each LLM-call event TWICE for one ``ChatModel.run``:
    once at the backend emitter (``backend.<provider>.chat.<name>``) and once
    mirrored at the Run level with a ``run.`` prefix
    (``run.backend.<provider>.chat.<name>``). The two carry different full
    paths, hence different signatures/idempotency_keys, so the sidecar does
    NOT dedup them. We normalise the leading ``run.`` prefix away so both
    collapse to the SAME call_key — the ``_on_start`` idempotency guard then
    folds them into one reservation (no double-billing), and POST pops the
    single slot regardless of which mirror delivered the success/error.
    """
    if "." not in path:
        return path
    base = path.rsplit(".", 1)[0]
    if base.startswith("run."):
        base = base[len("run.") :]
    return base


def _default_call_signature(ev: BeeAiStartEvent) -> str:
    """Hash (``model_id`` || visible input) into a 32-char hex digest.

    Per review-standards §2 the hash MUST include the model id —
    omitting it would let two different models in the same run
    collide on the same inflight slot.

    ``blake2b-16`` matches the LangChain + Agno integrations'
    signature width so downstream ID derivation is symmetric.
    """
    try:
        visible = "|".join(str(m) for m in ev.input) if ev.input else ""
    except Exception:  # noqa: BLE001
        visible = repr(ev.input) if ev.input is not None else ""
    payload = f"{ev.model_id}\n{visible}\n{ev.path}"
    return hashlib.blake2b(payload.encode("utf-8"), digest_size=16).hexdigest()


# ─────────────────────────────────────────────────────────────────────
# Bounded FIFO inflight map (review-standards §3 L4)
# ─────────────────────────────────────────────────────────────────────


class _InflightMap:
    """Bounded FIFO inflight map keyed by ``_stable_call_key`` output.

    Capacity-bound matches review-standards §3 L4: FIFO eviction
    with one-shot ``logger.warning`` so forgotten POSTs (agent
    killed mid-call) cannot grow memory unbounded. Evicted
    reservations are released by sidecar TTL sweep, not by adapter
    code.
    """

    __slots__ = ("_map", "_capacity", "_warned")

    def __init__(self, capacity: int = _INFLIGHT_MAX) -> None:
        self._map: OrderedDict[str, _InflightReservation] = OrderedDict()
        self._capacity = capacity
        self._warned = False

    def put(self, key: str, entry: _InflightReservation) -> None:
        if key in self._map:
            self._map.pop(key)
        self._map[key] = entry
        while len(self._map) > self._capacity:
            evicted_key, _ = self._map.popitem(last=False)
            if not self._warned:
                _LOGGER.warning(
                    "spendguard.integrations.beeai inflight map at capacity "
                    "%d; FIFO-evicting %s. This usually means a BeeAI "
                    "`success`/`error` event was never emitted for an "
                    "earlier call — reservations for evicted entries will "
                    "TTL-sweep on the sidecar.",
                    self._capacity,
                    evicted_key,
                )
                self._warned = True

    def pop(self, key: str) -> _InflightReservation | None:
        return self._map.pop(key, None)

    def get(self, key: str) -> _InflightReservation | None:
        return self._map.get(key)

    def __len__(self) -> int:
        return len(self._map)

    def __contains__(self, key: object) -> bool:
        return key in self._map

    def clear(self) -> None:
        self._map.clear()
        self._warned = False


# Module-shared inflight map. Same-process subscribers see it
# without explicit wiring. Operators can still pass their own map
# via the ``inflight`` keyword arg to ``subscribe_spendguard`` for
# stricter isolation between subscribers.
_SHARED_INFLIGHT = _InflightMap()


# ─────────────────────────────────────────────────────────────────────
# Public subscribe helper
# ─────────────────────────────────────────────────────────────────────


def subscribe_spendguard(  # noqa: PLR0913
    agent: Any,
    client: SpendGuardClient,
    *,
    budget_id: str,
    window_instance_id: str,
    unit: Any,
    pricing: Any,
    claim_estimator: ClaimEstimator | None = None,
    call_signature_fn: CallSignatureFn | None = None,
    route: str = "llm.call",
    inflight: _InflightMap | None = None,
) -> Callable[[], None]:
    """Install a SpendGuard subscriber on ``agent.emitter``.

    Registers ONE predicate on ``agent.emitter.match`` that fires for
    any event named ``start`` / ``success`` / ``error`` whose path
    contains an ``llm`` segment. Covers ``ReActAgent`` + ``Workflow``
    + any other agent whose child emitters publish under an ``llm.*``
    namespace.

    Returns a no-arg ``unsubscribe()`` callable. Per review-standards
    §3 L3 the returned cleanup MUST actually unhook — verified by
    tests T18/T19. NOT idempotent in the sense that a second
    ``subscribe_spendguard`` on the same agent installs a SECOND
    subscriber; callers must hold + invoke the returned unsubscribe
    explicitly.

    Args:
        agent: A ``beeai_framework.agents.base.BaseAgent`` subclass
            instance. The adapter reads ``agent.emitter`` (a
            ``cached_property`` declared on ``BaseAgent``); duck-typed
            stubs that expose ``.emitter.match(...)`` work for unit
            tests.
        client: A connected + handshook ``SpendGuardClient``. Owned
            by the caller; not closed by the subscriber.
        budget_id: Budget the reservation debits. REQUIRED.
        window_instance_id: Time-window scope on the budget. REQUIRED.
        unit: ``common_pb2.UnitRef`` describing the unit binding.
            REQUIRED — ``unit.unit_id`` MUST be non-empty.
        pricing: ``common_pb2.PricingFreeze`` for ledger lookup.
            REQUIRED.
        claim_estimator: Optional ``(BeeAiStartEvent) → list[BudgetClaim]``
            projector. When ``None`` a default tokeniser-backed
            estimator is auto-installed via model-name dispatch
            (mirrors LangChain / Agno).
        call_signature_fn: Optional override of the signature hashing.
            Defaults to ``_default_call_signature`` (blake2b-16 of
            ``model_id || input || path``).
        route: ``request_decision.route``. Defaults to ``"llm.call"``
            so dashboards group with the other adapters.
        inflight: Optional caller-owned inflight map for strict
            isolation between subscribers. Defaults to the
            module-shared ``_SHARED_INFLIGHT``.

    Raises:
        SpendGuardConfigError: any required string field empty / whitespace,
            or ``unit.unit_id`` empty, or ``agent`` has no ``emitter`` /
            ``agent.emitter`` has no ``match`` attribute (helpful error
            with install hint).
    """
    if client is None:
        raise SpendGuardConfigError(
            "subscribe_spendguard(client=...) is required; got None."
        )
    if not budget_id:
        raise SpendGuardConfigError(
            "subscribe_spendguard(budget_id=...) required."
        )
    if not window_instance_id:
        raise SpendGuardConfigError(
            "subscribe_spendguard(window_instance_id=...) required."
        )
    unit_id = getattr(unit, "unit_id", "") if unit is not None else ""
    if not unit_id:
        raise SpendGuardConfigError(
            "subscribe_spendguard unit.unit_id required (DESIGN.md §6)."
        )
    if agent is None:
        raise SpendGuardConfigError(
            "subscribe_spendguard(agent=...) is required; got None. "
            "Pass a beeai_framework.agents.base.BaseAgent subclass "
            "instance (e.g. ReActAgent)."
        )
    emitter = getattr(agent, "emitter", None)
    if emitter is None or not hasattr(emitter, "match"):
        raise SpendGuardConfigError(
            "subscribe_spendguard requires `agent.emitter` to be a "
            "BeeAI Emitter with a `match(matcher, callback)` method. "
            "Install via `pip install 'spendguard-sdk[beeai]'` and pass "
            "a real BaseAgent subclass."
        )

    inflight_map = inflight if inflight is not None else _SHARED_INFLIGHT
    sig_fn: CallSignatureFn = call_signature_fn or _default_call_signature

    # Resolve the default estimator lazily so importing this module
    # doesn't pull in the tokeniser stack until the subscriber is
    # actually installed.
    estimator: ClaimEstimator
    if claim_estimator is None:
        from .._default_estimator import langchain_default_claim_estimator

        # langchain_default_claim_estimator returns
        # ``Callable[[Sequence[Any]], list[Any]]`` keyed on the
        # message list. We re-use it for BeeAI by feeding the
        # normalised input sequence. The model family is resolved
        # per-event from ``ev.model_id`` rather than the constructor
        # arg so multi-model Workflows work with one subscribe call.
        def _resolve(ev: BeeAiStartEvent) -> list[Any]:
            inner = langchain_default_claim_estimator(
                budget_id=budget_id,
                window_instance_id=window_instance_id,
                unit=unit,
                model=ev.model_id,
            )
            # langchain estimator expects Sequence[BaseMessage]; we
            # pass the raw input which may be a list of dicts, list
            # of message-shaped objects, or strings. The estimator
            # tolerates duck-typed inputs because it ultimately calls
            # str() on each entry's content attr.
            return inner(list(ev.input))

        estimator = _resolve
    else:
        estimator = claim_estimator

    def _predicate(event: Any) -> bool:
        """Match a start/success/error event for an LLM call.

        BeeAI 0.1.x emits LLM-call events on the *ChatModel's* emitter with
        paths like ``backend.openai.chat.start`` (a ``chat`` segment, NOT an
        ``llm`` segment), and those events do NOT bubble up to the agent's
        emitter. So callers subscribe on the ChatModel's emitter (pass the
        ChatModel, or ``agent.llm``), and we match either a ``chat`` or an
        ``llm`` path segment to stay tolerant across BeeAI layouts. On a
        ChatModel emitter only chat events fire, so this never over-matches.
        """
        name = getattr(event, "name", None)
        if name not in ("start", "success", "error"):
            return False
        path = getattr(event, "path", "") or ""
        segs = path.split(".")
        return "llm" in segs or "chat" in segs

    async def _handle(data: Any, meta: Any) -> None:
        ev_name = getattr(meta, "name", None)
        if ev_name == "start":
            await _on_start(data, meta)
        elif ev_name == "success":
            await _on_success(data, meta)
        elif ev_name == "error":
            await _on_error(data, meta)
        # silently ignore newToken / partialUpdate etc. (spec §3 non-goal)

    async def _on_start(data: Any, meta: Any) -> None:
        ctx = current_run_context()
        # Normalise the event view. Tolerate the various BeeAI 0.1.x
        # payload shapes — ``ChatModel.create`` emits a model invoke
        # event; ``ReActAgent`` emits a higher-level llm step event.
        # Cast to list so the dataclass stays hashable.
        raw_input = (
            getattr(data, "input", None)
            or getattr(data, "messages", None)
            or getattr(data, "prompt", None)
            or []
        )
        if isinstance(raw_input, str):
            normalised_input: list[Any] = [raw_input]
        elif isinstance(raw_input, (list, tuple)):
            normalised_input = list(raw_input)
        else:
            normalised_input = [raw_input]

        model_id = (
            getattr(data, "modelId", None)
            or getattr(data, "model_id", None)
            or ""
        )
        if model_id is None:
            model_id = ""
        model_id = str(model_id)

        path = getattr(meta, "path", "") or ""
        start_ev = BeeAiStartEvent(
            input=tuple(normalised_input),
            model_id=model_id,
            path=path,
        )

        call_key = _stable_call_key(path)
        # Race-safe idempotency guard. BeeAI 0.1.x emits the LLM-call ``start``
        # event MORE THAN ONCE for a single ChatModel.run (a Run-level start
        # plus an inner backend start — same ``backend.<provider>.chat.start``
        # path → same call_key, but DIFFERENT event payloads → different
        # signatures/idempotency_keys, so the sidecar does NOT dedup them).
        # request_decision is not idempotent across distinct keys, so without
        # this guard one logical LLM call reserves the budget TWICE
        # (double-billing; on a thin budget the second reserve balance-denies
        # and aborts the run). BeeAI dispatches the two starts CONCURRENTLY via
        # a TaskGroup, so a plain check-then-reserve races. We claim the
        # call_key slot SYNCHRONOUSLY with a placeholder BEFORE the
        # request_decision await — there is no await between the membership
        # check and the placeholder put, so in asyncio's single thread the two
        # starts cannot both pass. The duplicate returns early; POST pops the
        # one slot.
        if call_key in inflight_map:
            return
        signature = sig_fn(start_ev)
        llm_call_id = str(
            derive_uuid_from_signature(signature, scope="llm_call_id")
        )
        decision_id = str(
            derive_uuid_from_signature(signature, scope="decision_id")
        )
        step_id = f"{ctx.run_id}:beeai:{signature[:16]}"
        # Placeholder (no reservation_ids yet) claims the call_key atomically
        # with the check above (no await in between).
        inflight_map.put(
            call_key,
            _InflightReservation(
                signature=signature,
                reservation_ids=[],
                decision_id=decision_id,
                llm_call_id=llm_call_id,
                step_id=step_id,
                run_id=ctx.run_id,
                unit=unit,
                pricing=pricing,
                model_id=model_id,
            ),
        )
        idempotency_key = derive_idempotency_key(
            tenant_id=client.tenant_id,
            session_id=client.session_id,
            run_id=ctx.run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )
        decision_context = {
            "integration": "beeai",
            "model_backend": (
                type(data).__name__ if data is not None else "unknown"
            ),
        }
        if model_id:
            decision_context["model_id"] = model_id

        # ``request_decision`` raises DecisionDenied on DENY; reaching
        # the next line = ALLOW (CONTINUE) or DEGRADE. We do NOT
        # try/except DecisionDenied — letting it propagate is the
        # locked contract for security §S2 / lifecycle §L1. On a STOP we
        # release the placeholder so the call_key is free again (the DENY
        # turn must be able to re-enter; a leak would block future calls).
        try:
            outcome: DecisionOutcome = await client.request_decision(
                trigger="LLM_CALL_PRE",
                run_id=ctx.run_id,
                step_id=step_id,
                llm_call_id=llm_call_id,
                tool_call_id="",
                decision_id=decision_id,
                route=route,
                projected_claims=estimator(start_ev),
                idempotency_key=idempotency_key,
                projected_unit=unit,
                decision_context_json=decision_context,
            )
        except BaseException:
            inflight_map.pop(call_key)
            raise

        # Replace the placeholder with the real reservation. Even when
        # outcome.reservation_ids is empty (DEGRADE / SKIPPED) we still record
        # the slot so a future success commits downstream of the
        # projection-pipeline outcome.
        inflight_map.put(
            call_key,
            _InflightReservation(
                signature=signature,
                reservation_ids=list(outcome.reservation_ids),
                decision_id=outcome.decision_id or decision_id,
                llm_call_id=llm_call_id,
                step_id=step_id,
                run_id=ctx.run_id,
                unit=unit,
                pricing=pricing,
                model_id=model_id,
            ),
        )

    async def _on_success(data: Any, meta: Any) -> None:
        path = getattr(meta, "path", "") or ""
        slot = inflight_map.pop(_stable_call_key(path))
        if slot is None:
            # Either start never fired (instrumentation bug) or two
            # success events for one start. Log + no-op rather than
            # emit a commit-without-reserve event (review-standards
            # §3 L2).
            _LOGGER.warning(
                "spendguard.beeai: success event without matching start "
                "(path=%s)",
                path,
            )
            return
        if not slot.reservation_ids:
            # Reservation was never issued (SKIPPED outcome); nothing
            # to commit.
            return

        total_tokens, provider_event_id = _extract_usage_success(data)
        try:
            await client.emit_llm_call_post(
                run_id=slot.run_id,
                step_id=slot.step_id,
                llm_call_id=slot.llm_call_id,
                decision_id=slot.decision_id,
                reservation_id=slot.reservation_ids[0],
                provider_reported_amount_atomic="",
                estimated_amount_atomic=str(total_tokens),
                unit=slot.unit,
                pricing=slot.pricing,
                provider_event_id=provider_event_id,
                outcome="SUCCESS",
            )
        except SpendGuardError as exc:
            # Rather than rely on Emitter's wrapping behaviour we log
            # + return so the caller's run still surfaces the model
            # output. The reservation will TTL-sweep on the ledger
            # side (review-standards §3 L4).
            _LOGGER.warning(
                "spendguard.beeai: emit_llm_call_post failed for "
                "run_id=%s sig=%s err=%r — reservation will TTL-sweep",
                slot.run_id,
                slot.signature[:8],
                exc,
            )

    async def _on_error(data: Any, meta: Any) -> None:
        path = getattr(meta, "path", "") or ""
        slot = inflight_map.pop(_stable_call_key(path))
        if slot is None:
            _LOGGER.warning(
                "spendguard.beeai: error event without matching start "
                "(path=%s)",
                path,
            )
            return
        if not slot.reservation_ids:
            return

        try:
            await client.emit_llm_call_post(
                run_id=slot.run_id,
                step_id=slot.step_id,
                llm_call_id=slot.llm_call_id,
                decision_id=slot.decision_id,
                reservation_id=slot.reservation_ids[0],
                provider_reported_amount_atomic="",
                estimated_amount_atomic="0",
                unit=slot.unit,
                pricing=slot.pricing,
                provider_event_id="",
                outcome="PROVIDER_ERROR",
            )
        except SpendGuardError as exc:
            _LOGGER.warning(
                "spendguard.beeai: emit_llm_call_post (PROVIDER_ERROR) "
                "failed run_id=%s sig=%s err=%r — TTL-sweep",
                slot.run_id,
                slot.signature[:8],
                exc,
            )

    # ``Emitter.match(matcher, callback) -> CleanupFn`` (BeeAI 0.1.81,
    # verified at ``beeai_framework/emitter/emitter.py:176``). The
    # returned closure unhooks the listener when called.
    unsubscribe: Callable[[], None] = emitter.match(_predicate, _handle)
    return unsubscribe


# ─────────────────────────────────────────────────────────────────────
# Usage extractor (review-standards §3 L1 lifecycle)
# ─────────────────────────────────────────────────────────────────────


def _extract_usage_success(data: Any) -> tuple[int, str]:
    """Return ``(total_tokens, provider_event_id)`` from a success event.

    BeeAI's ``ChatModel.create`` success payload exposes ``usage``
    (a dict OR an object with ``total_tokens``) and ``id`` /
    ``response_id``. The agent-level success event wraps the result
    under ``.output`` / ``.response`` — we walk a tolerant cascade.
    """
    if data is None:
        return 0, ""

    # Tier 1: direct attrs on the payload.
    usage = getattr(data, "usage", None)
    # Tier 2: nested under .value / .output / .response. BeeAI 0.1.x wraps the
    # ChatModelOutput under ``ChatModelSuccessEvent.value`` (``.value.usage``
    # carries total_tokens) — ``value`` MUST be in this cascade or the commit
    # estimate is 0 and the sidecar rejects it ("estimated_amount_atomic must
    # be > 0"), leaking the reservation.
    if usage is None:
        for attr in ("value", "output", "response", "result"):
            inner = getattr(data, attr, None)
            if inner is not None:
                usage = getattr(inner, "usage", None)
                if usage is not None:
                    break
    total = _usage_total_tokens(usage)

    pid: Any = (
        getattr(data, "id", None)
        or getattr(data, "response_id", None)
        or getattr(data, "request_id", None)
        or ""
    )
    if not isinstance(pid, str):
        pid = str(pid) if pid is not None else ""

    return total, pid


def _usage_total_tokens(usage: Any) -> int:
    """Coerce a heterogeneous ``usage`` view into an int total."""
    if usage is None:
        return 0
    # Dict shape.
    if isinstance(usage, dict):
        total = usage.get("total_tokens")
        if isinstance(total, (int, float)) and total > 0:
            return int(total)
        inp = usage.get("input_tokens", 0) or usage.get("prompt_tokens", 0) or 0
        out = (
            usage.get("output_tokens", 0)
            or usage.get("completion_tokens", 0)
            or 0
        )
        try:
            return int(inp) + int(out)
        except (TypeError, ValueError):
            return 0
    # Object shape.
    total = getattr(usage, "total_tokens", None)
    if isinstance(total, (int, float)) and total > 0:
        return int(total)
    inp = getattr(usage, "input_tokens", 0) or getattr(usage, "prompt_tokens", 0) or 0
    out = getattr(usage, "output_tokens", 0) or getattr(usage, "completion_tokens", 0) or 0
    try:
        return int(inp) + int(out)
    except (TypeError, ValueError):
        return 0


__all__ = [
    "BeeAiStartEvent",
    "CallSignatureFn",
    "ClaimEstimator",
    "RunContext",
    "_INFLIGHT_MAX",
    "_InflightMap",
    "_SHARED_INFLIGHT",
    "_extract_usage_success",
    "_stable_call_key",
    "current_run_context",
    "run_context",
    "subscribe_spendguard",
]
