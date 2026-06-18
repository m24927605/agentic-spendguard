"""``SpendGuardDSPyCallback`` — DSPy ``BaseCallback`` SpendGuard gate.

Implements the DSPy ``BaseCallback`` contract: ``on_lm_start`` (PRE
reserve) and ``on_lm_end`` (POST commit/release). Operator wires via
``dspy.configure(callbacks=[SpendGuardDSPyCallback(...)])`` with the
callback FIRST in the list so reserve precedes any user observer
callback.

Lifecycle (per design.md §4)::

    user → dspy.ChainOfThought("q -> a")(question="...")
           ↓
    dspy.LM("openai/gpt-4o-mini").__call__(prompt=..., messages=...)
           ↓
    DSPy iterates callbacks list:
      SpendGuardDSPyCallback.on_lm_start(call_id, instance, inputs)
           ├─ resolver(instance.model) → BudgetBinding
           ├─ estimator(inputs) → projected claims
           ├─ _SHIM_IN_FLIGHT.set(True)  (blocks D12 wrapper double-reserve)
           ├─ sidecar.RequestDecision    ←── BEFORE provider HTTP
           │    ALLOW → stash (call_id → state) in _PENDING; continue
           │    DENY  → raise DecisionDenied
           └─ DEGRADE → raise SidecarUnavailable (fail-closed)
           ↓
    dspy.LM invokes provider (LiteLLM or direct SDK)
           ↓
    SpendGuardDSPyCallback.on_lm_end(call_id, outputs, exception)
           ├─ pop state for call_id
           ├─ exception None → reconciler(outputs) → real claim
           │                 → sidecar.emit_llm_call_post(SUCCESS)
           ├─ exception → outcome = CANCELLED if CancelledError else FAILURE
           │            → sidecar.emit_llm_call_post(outcome=...)
           └─ _SHIM_IN_FLIGHT.reset(token)  (try/finally)

Per-call state lives in ``_PENDING: dict[call_id, _CallState]`` keyed
by DSPy's UUID. ``on_lm_end`` pops; a TTL sweep on every ``on_lm_start``
drops entries older than 5 min + WARN.
"""

from __future__ import annotations

import asyncio
import contextvars
import hashlib
import json
import logging
import os
import threading
import time
from collections.abc import Callable
from typing import Any

from ..._litellm_shim import _IN_FLIGHT as _SHIM_IN_FLIGHT
from ...client import DecisionOutcome, SpendGuardClient
from ...ids import (
    derive_idempotency_key,
    derive_uuid_from_signature,
    new_uuid7,
)
from ._errors import (
    DecisionDenied,
    SidecarUnavailable,
    SpendGuardConfigError,
    SpendGuardDegradeBlocked,
    SpendGuardError,
)
from ._options import BudgetBinding, RunContext, _CallState

# DSPy's typed callback base. The package barrel ``__init__.py`` carries
# the install-hint ImportError guard; here we import resiliently so the
# unit suite can load ``_wrapper`` directly via package-path bypass
# (mirrors ``strands/_hook_provider.py`` pattern).
try:  # pragma: no cover — branch chosen at import time
    from dspy.utils.callback import BaseCallback as _RealBaseCallback

    _DSPY_AVAILABLE = True
except ImportError:  # pragma: no cover — branch chosen at import time
    _RealBaseCallback = None  # type: ignore[assignment, misc]
    _DSPY_AVAILABLE = False


# Proto stubs — required for the default estimator's ``BudgetClaim``
# fallback. Same try/except pattern as ``litellm.py``.
try:  # pragma: no cover — proto stubs always present in normal builds
    from ..._proto.spendguard.common.v1 import common_pb2

    _PROTO_AVAILABLE = True
except ImportError:  # pragma: no cover
    common_pb2 = None  # type: ignore[assignment]
    _PROTO_AVAILABLE = False


log = logging.getLogger("spendguard.integrations.dspy")


# ─────────────────────────────────────────────────────────────────────
# Type aliases (public surface)
# ─────────────────────────────────────────────────────────────────────

ClaimEstimator = Callable[[dict[str, Any]], list[Any]]
"""Project a list of ``BudgetClaim`` proto messages from the inputs the
DSPy LM is about to dispatch. v1 contract: returns exactly 1 claim.
Inputs typically carry ``messages: list[dict]`` (chat) or
``prompt: str`` (completions)."""

ClaimReconciler = Callable[[Any], list[Any]]
"""Project a reconciled ``BudgetClaim`` list from the DSPy outputs.
Receives the ``outputs`` argument DSPy passes to ``on_lm_end``
(typically a list of ``LMResponse`` objects with ``.usage`` dicts).
v1 contract: returns exactly 1 claim."""

BudgetResolver = Callable[[str], BudgetBinding]
"""Resolve a DSPy LM ``model`` string (e.g. ``"openai/gpt-4o-mini"``)
to the budget / window / unit / pricing tuple the reservation will
debit. Called once per ``on_lm_start``."""


# ─────────────────────────────────────────────────────────────────────
# Module-level state
# ─────────────────────────────────────────────────────────────────────

# Per-call state map. DSPy provides a UUID ``call_id`` to both
# start/end hooks; this dict bridges the two sync callbacks (which
# run on the same thread because DSPy hook dispatch is sequential).
# Keyed on the DSPy UUID so concurrent calls (asyncio.gather over
# multiple dspy.Predict instances) never collide.
_PENDING: dict[str, _CallState] = {}

# Upper bound on time between on_lm_start and on_lm_end before the
# sweep drops the entry as orphaned. 5 minutes matches the sidecar's
# default reservation TTL so a swept entry corresponds to a reservation
# the ledger will TTL-release naturally.
_PENDING_TTL_SECONDS = 300


# ─────────────────────────────────────────────────────────────────────
# Sync-from-async bridge — per-callback background event loop in a
# dedicated daemon thread. DSPy callbacks are sync; the SpendGuard
# client is async-only and its grpc.aio channel binds to the loop it
# is first used on. Bare ``asyncio.run`` per call spins up AND tears
# down a fresh loop each invocation, so the second call drives the
# channel against a *closed* loop — a fail-open hazard because the POST
# path swallows the resulting loop error and returns "success" without
# a recorded commit.
#
# Owning ONE persistent loop for the callback's lifetime keeps the
# channel bound to a single live loop across on_lm_start / on_lm_end.
# Mirrors the proven llamaindex bridge (``llamaindex/_hook.py``).
# ─────────────────────────────────────────────────────────────────────


class _AsyncBridge:
    """Per-callback background thread running a persistent asyncio loop.

    Owns one daemon thread and one ``asyncio.AbstractEventLoop``. Each
    ``run(coro)`` call schedules the coroutine on the background loop
    via ``run_coroutine_threadsafe`` and blocks on the resulting
    ``concurrent.futures.Future`` until completion. The loop survives
    the lifetime of the bridge instance so the client's grpc.aio
    channel stays bound to one live loop across every callback.

    Mirrors ``spendguard.integrations.llamaindex._hook._AsyncBridge``.
    """

    def __init__(self) -> None:
        self._loop: asyncio.AbstractEventLoop | None = None
        self._thread: threading.Thread | None = None
        self._ready = threading.Event()
        self._closed = False
        self._start_lock = threading.Lock()

    def _start(self) -> None:
        """Spin up the background thread + loop on first use (lazy)."""
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
                    name="spendguard-dspy-bridge",
                    daemon=True,
                )
                self._thread.start()
        # Every caller blocks on the ready event until the spawned thread
        # has installed the loop.
        self._ready.wait(timeout=5.0)
        if self._loop is None:
            raise RuntimeError(
                "spendguard-dspy-bridge failed to start its asyncio loop "
                "within 5s; check thread allowance / resource limits."
            )

    def run(self, coro: Any) -> Any:  # noqa: ANN401 — async client returns vary
        """Run ``coro`` on the background loop; block on its result.

        Whatever the coroutine raises is re-raised in the calling thread
        via ``Future.result()`` exception propagation, so fail-closed
        reserve errors still surface to the DSPy caller unchanged.
        """
        if self._closed:
            raise RuntimeError(
                "spendguard-dspy-bridge has been closed; construct a "
                "fresh callback."
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
# Callback base class — pick real ABC when dspy is available so
# isinstance and DSPy's dispatch both work; fall back to plain base
# class in unit tests where dspy-ai isn't installed.
# ─────────────────────────────────────────────────────────────────────

if _RealBaseCallback is not None:  # pragma: no cover — chosen at import
    _CallbackBase = _RealBaseCallback
else:
    class _CallbackBase:  # type: ignore[no-redef]
        """Unit-test stand-in for ``dspy.utils.callback.BaseCallback``.

        Mirrors the DSPy GA contract surface — ``on_lm_start`` and
        ``on_lm_end`` are the only two hooks D21 binds. The rest
        (``on_tool_*`` / ``on_module_*``) are out of scope per
        design.md §3 and are not stubbed here so a future
        binding gets a clear AttributeError rather than silently
        no-oping.
        """


# ─────────────────────────────────────────────────────────────────────
# Helpers (module-level so they're testable + reusable)
# ─────────────────────────────────────────────────────────────────────


def _signature_from_inputs(inputs: dict[str, Any]) -> str:
    """Stable 16-byte BLAKE2b hex hash over the LM inputs.

    Used to derive the ``llm_call_id`` / ``decision_id`` per call so a
    DSPy retry firing ``on_lm_start`` again with identical inputs hits
    the sidecar idempotency cache. ``default=str`` keeps the hash
    stable when ``inputs`` carries non-serializable objects (Pydantic
    models, datetime, etc.).
    """
    try:
        payload = json.dumps(inputs, sort_keys=True, default=str)
    except (TypeError, ValueError):
        # Last-ditch fallback for inputs that even ``default=str``
        # can't serialize. ``repr`` is deterministic within a session.
        payload = repr(inputs)
    return hashlib.blake2b(payload.encode("utf-8"), digest_size=16).hexdigest()


def _extract_total_tokens(outputs: Any) -> int:
    """Extract a total token count from DSPy ``outputs``.

    DSPy >= 2.6 ``LMResponse`` exposes ``.usage`` dict; some custom
    LMs return bare lists of strings. Be defensive and never raise —
    every path that calls this is inside ``on_lm_end`` which must
    NEVER raise (DSPy treats callback exceptions as runtime failures).

    Extraction order:
      1. ``outputs[0].usage["total_tokens"]`` (DSPy GA shape).
      2. ``outputs.usage["total_tokens"]`` (single-response shape).
      3. ``outputs[0].usage["prompt_tokens"] + ["completion_tokens"]``
         (OpenAI shape when ``total_tokens`` missing).
      4. ``outputs[0].usage["input_tokens"] + ["output_tokens"]``
         (Anthropic shape).
      5. Default: 0.
    """
    if outputs is None:
        return 0
    # Prefer list-indexed first element; fall back to outputs itself.
    if isinstance(outputs, list) and outputs:
        first = outputs[0]
    elif isinstance(outputs, list):
        # Empty list — nothing to extract.
        return 0
    else:
        first = outputs
    usage = getattr(first, "usage", None)
    if not isinstance(usage, dict):
        return 0
    # 1) Universal total
    total = usage.get("total_tokens")
    if isinstance(total, int) and total > 0:
        return int(total)
    # 3) OpenAI shape
    inp = usage.get("prompt_tokens") or 0
    out = usage.get("completion_tokens") or 0
    if isinstance(inp, int) and isinstance(out, int) and (inp + out) > 0:
        return int(inp) + int(out)
    # 4) Anthropic shape
    inp = usage.get("input_tokens") or 0
    out = usage.get("output_tokens") or 0
    if isinstance(inp, int) and isinstance(out, int) and (inp + out) > 0:
        return int(inp) + int(out)
    return 0


def _extract_provider_event_id(outputs: Any) -> str:
    """Extract a provider event id from DSPy ``outputs``.

    Falls through gracefully — never raises. Used to thread the
    upstream provider's response id into the audit chain so operators
    can correlate SpendGuard rows with their provider dashboards.
    """
    if outputs is None:
        return ""
    if isinstance(outputs, list) and outputs:
        first = outputs[0]
    elif isinstance(outputs, list):
        return ""
    else:
        first = outputs
    rid = getattr(first, "id", None) or getattr(first, "response_id", None)
    return str(rid) if isinstance(rid, str) else ""


def _classify_exception(exc: BaseException | None) -> str:
    """Classify an ``on_lm_end`` exception into a release outcome.

    Order matters: ``asyncio.CancelledError`` is a subclass of
    ``BaseException`` (not ``Exception``) so we check it first.

    Returns:
      * ``"SUCCESS"`` when ``exc`` is None.
      * ``"CANCELLED"`` when ``exc`` is ``asyncio.CancelledError``.
      * ``"FAILURE"`` for every other exception.
    """
    if exc is None:
        return "SUCCESS"
    if isinstance(exc, asyncio.CancelledError):
        return "CANCELLED"
    return "FAILURE"


# ─────────────────────────────────────────────────────────────────────
# Main callback class
# ─────────────────────────────────────────────────────────────────────


class SpendGuardDSPyCallback(_CallbackBase):  # type: ignore[misc, valid-type]
    """DSPy callback that reserves before each LM call and commits after.

    Register on the global DSPy callback list::

        dspy.configure(
            lm=dspy.LM("openai/gpt-4o-mini"),
            callbacks=[SpendGuardDSPyCallback(
                client=client,
                budget_resolver=resolve_budget_for_model,
                claim_reconciler=reconcile_from_dspy_usage,
            )],
        )

    Placement: this callback MUST appear FIRST in the callbacks list
    so reserve precedes any user observer callback (which would
    otherwise fire before SpendGuard and could mutate inputs the
    estimator hashes).

    Args:
        client: A connected + handshook ``SpendGuardClient``. Owned by
            the caller; not closed by the callback.
        budget_resolver: Receives the LM's ``model`` string and returns
            a ``BudgetBinding`` to debit. Called once per
            ``on_lm_start``.
        claim_estimator: Optional projector from ``inputs`` to a
            single-element ``BudgetClaim`` list. When ``None``, the
            callback uses a built-in chars/4 fallback (parity with
            other adapters' default estimator).
        claim_reconciler: REQUIRED. Receives the DSPy ``outputs`` and
            returns a single-element reconciled ``BudgetClaim`` list.
            Reads ``.usage`` for real provider-reported tokens.
        run_context_factory: Optional factory returning a ``RunContext``
            per call. When omitted, the callback mints a fresh UUIDv7
            per LM call so each call gets its own run scope. Provide
            a factory bridging a parent LangChain / Strands / pydantic
            run when you want shared correlation.
        fail_closed: When ``True`` (default), DEGRADE and sidecar
            unavailability raise ``SpendGuardDegradeBlocked`` /
            ``SidecarUnavailable``. ``SPENDGUARD_DSPY_FAIL_OPEN=1``
            also forces fail-open (dev only).
        route: Route string for the ``RequestDecision`` RPC. Defaults
            to ``"llm.call"``; override only for custom audit
            categorisation.
    """

    class SyncInAsyncContext(SpendGuardConfigError):
        """``on_lm_start`` invoked from inside a running event loop.

        DSPy 2.6 callbacks are sync; we use ``asyncio.run`` to dispatch
        the sidecar RPC. ``asyncio.run`` itself raises ``RuntimeError``
        from inside a running loop — we raise this typed exception
        instead so callers get a clear, actionable hint (run dspy from
        a sync entrypoint, or pre-emit reservations via the client
        directly).
        """

    def __init__(
        self,
        *,
        client: SpendGuardClient,
        budget_resolver: BudgetResolver,
        claim_estimator: ClaimEstimator | None = None,
        claim_reconciler: ClaimReconciler,
        run_context_factory: Callable[[], RunContext] | None = None,
        fail_closed: bool = True,
        route: str = "llm.call",
    ) -> None:
        super().__init__()
        if client is None:
            raise SpendGuardConfigError(
                "SpendGuardDSPyCallback(client=...) is required; got None."
            )
        if budget_resolver is None:
            raise SpendGuardConfigError(
                "SpendGuardDSPyCallback(budget_resolver=...) is required; "
                "DSPy gates per-LM-call so the resolver maps the LM's "
                "model string to the budget binding."
            )
        if claim_reconciler is None:
            raise SpendGuardConfigError(
                "SpendGuardDSPyCallback(claim_reconciler=...) is required; "
                "DSPy LMResponse usage shape varies per LM subclass so the "
                "caller must own the reconciliation projection."
            )
        self._client = client
        self._budget_resolver = budget_resolver
        self._claim_estimator = claim_estimator
        self._claim_reconciler = claim_reconciler
        self._run_context_factory = run_context_factory or (
            lambda: RunContext(run_id=str(new_uuid7()))
        )
        self._fail_closed = fail_closed
        self._route = route
        # Persistent background loop owned by this callback. Every
        # sidecar RPC (reserve in on_lm_start, commit/release in
        # on_lm_end) is dispatched onto it so the client's grpc.aio
        # channel stays bound to one live loop for the callback's
        # lifetime. Lazily started on first use.
        self._bridge = _AsyncBridge()
        self._fail_open_dev: bool = (
            os.environ.get("SPENDGUARD_DSPY_FAIL_OPEN") == "1"
        )
        if self._fail_open_dev:
            log.warning(
                "spendguard.integrations.dspy: SPENDGUARD_DSPY_FAIL_OPEN=1 "
                "— fail-open; sidecar errors will allow LM calls. DEV ONLY."
            )

    # ─────────────────────────────────────────────────────────────────
    # on_lm_start — reserve + stash
    # ─────────────────────────────────────────────────────────────────

    def on_lm_start(
        self,
        call_id: str,
        instance: Any,
        inputs: dict[str, Any],
    ) -> None:
        """Reserve before each ``dspy.LM`` call.

        ALLOW → stash by ``call_id`` and return; D12's wrapper (if
        installed) short-circuits via the ``_SHIM_IN_FLIGHT``
        contextvar so no double reserve fires.

        DENY → raise ``DecisionDenied`` (DSPy surfaces to caller).

        DEGRADE → raise ``SpendGuardDegradeBlocked`` (fail-closed)
        or warn + return (fail-open via env flag).

        ``asyncio.CancelledError`` is a subclass of ``BaseException``
        not ``Exception`` so the cleanup in the DENY path uses a
        ``finally`` clause to guarantee the contextvar is reset on
        every non-ALLOW path.
        """
        # Sweep first so the resolver/estimator/contextvar work doesn't
        # leak state from a dropped call.
        self._sweep_pending()
        # Guard before we touch contextvars or dispatch RPC — async
        # context detection must NOT mutate state.
        self._guard_async_context()
        if not call_id:
            raise SpendGuardConfigError(
                "SpendGuardDSPyCallback.on_lm_start received empty call_id; "
                "DSPy 2.6+ guarantees a stable UUID — check DSPy version."
            )

        model_str = ""
        if instance is not None:
            model_str = str(getattr(instance, "model", "") or "")

        binding = self._budget_resolver(model_str)
        if binding is None:
            raise SpendGuardConfigError(
                "SpendGuardDSPyCallback.budget_resolver returned None for "
                f"model={model_str!r}; must return a BudgetBinding."
            )

        signature = _signature_from_inputs(inputs)
        rc = self._run_context_factory()
        run_id = str(rc.run_id) if rc is not None else str(new_uuid7())
        # Truncate the call_id slug for human-readable step_id (full
        # call_id still threads through the stash + idempotency key).
        step_id = f"dspy:{call_id[:16]}"
        llm_call_id = str(
            derive_uuid_from_signature(signature, scope="llm_call_id")
        )
        decision_id = str(
            derive_uuid_from_signature(signature, scope="decision_id")
        )
        idempotency_key = derive_idempotency_key(
            tenant_id=self._client.tenant_id,
            session_id=self._client.session_id,
            run_id=run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )
        projected_claims = (
            self._claim_estimator(inputs)
            if self._claim_estimator is not None
            else self._default_estimator(inputs, binding)
        )
        if len(projected_claims) != 1:
            raise SpendGuardConfigError(
                f"DSPy claim_estimator returned {len(projected_claims)} "
                "claims; v1 contract requires exactly 1."
            )
        estimator_amount = str(
            getattr(projected_claims[0], "amount_atomic", "0") or "0"
        )

        decision_context = {
            "integration": "dspy",
            "lm_model": model_str,
        }

        # Block D12 wrapper from double-reserving the same call. The
        # token is stored on the stash so on_lm_end (success or
        # failure) can reset it deterministically.
        token: contextvars.Token[bool] = _SHIM_IN_FLIGHT.set(True)

        try:
            outcome: DecisionOutcome = self._bridge.run(
                self._client.request_decision(
                    trigger="LLM_CALL_PRE",
                    run_id=run_id,
                    step_id=step_id,
                    llm_call_id=llm_call_id,
                    tool_call_id="",
                    decision_id=decision_id,
                    route=self._route,
                    projected_claims=projected_claims,
                    idempotency_key=idempotency_key,
                    projected_unit=binding.unit,
                    decision_context_json=decision_context,
                )
            )
        except DecisionDenied:
            # DENY propagates to DSPy; reset contextvar via finally
            # below so the next call has a clean slate.
            _SHIM_IN_FLIGHT.reset(token)
            raise
        except SpendGuardError as exc:
            _SHIM_IN_FLIGHT.reset(token)
            if self._fail_open_dev or not self._fail_closed:
                log.warning(
                    "spendguard.integrations.dspy: fail-open — allowing "
                    "LM call despite sidecar error %r (DEV ONLY).",
                    exc,
                )
                return
            raise SidecarUnavailable(
                f"sidecar pre-call failed: {exc}"
            ) from exc
        except BaseException:
            # Belt-and-braces: any exception during request_decision
            # (including KeyboardInterrupt / CancelledError) must reset
            # the contextvar before propagating.
            _SHIM_IN_FLIGHT.reset(token)
            raise

        decision_name = getattr(outcome, "decision", "")
        if decision_name == "DEGRADE":
            if self._fail_open_dev or not self._fail_closed:
                log.warning(
                    "spendguard.integrations.dspy: DEGRADE under fail-open "
                    "— allowing LM call; commit will NOT fire (DEV ONLY)."
                )
                _SHIM_IN_FLIGHT.reset(token)
                return
            _SHIM_IN_FLIGHT.reset(token)
            raise SpendGuardDegradeBlocked(
                "sidecar returned DEGRADE; DSPy callback fails closed."
            )

        reservation_id = (
            outcome.reservation_ids[0]
            if outcome.reservation_ids
            else None
        )

        # Stash for the matching on_lm_end. Keyed by DSPy's call_id
        # (stable UUID) so concurrent calls never collide.
        _PENDING[call_id] = _CallState(
            decision_id=outcome.decision_id,
            reservation_id=reservation_id,
            llm_call_id=llm_call_id,
            step_id=step_id,
            run_id=run_id,
            unit=binding.unit,
            pricing=binding.pricing,
            inputs_signature=signature,
            estimator_amount_atomic=estimator_amount,
            model_str=model_str,
            shim_token=token,
        )

    # ─────────────────────────────────────────────────────────────────
    # on_lm_end — commit / release + exception classification
    # ─────────────────────────────────────────────────────────────────

    def on_lm_end(
        self,
        call_id: str,
        outputs: Any,
        exception: BaseException | None,
    ) -> None:
        """Commit or release the reservation after the LM call.

        SUCCESS (``exception is None``)  → reconciler(outputs) → commit.
        FAILURE (any other exception)    → release with FAILURE outcome.
        CANCELLED (``CancelledError``)   → release with CANCELLED outcome.

        Missing stash entry → WARN + return; this can happen when
        ``on_lm_end`` fires without a matching ``on_lm_start`` (DSPy
        bug or operator misconfiguration) or after a TTL sweep dropped
        the entry. NEVER raise — DSPy treats callback exceptions as
        runtime failures that would mask the original LM exception.
        """
        state = _PENDING.pop(call_id, None)
        if state is None:
            log.warning(
                "spendguard.integrations.dspy: on_lm_end fired with no "
                "matching on_lm_start state for call_id=%s; either a "
                "TTL sweep dropped it, or on_lm_start was never called.",
                call_id,
            )
            return

        try:
            if state.reservation_id is None:
                # Fail-open / DEGRADE-with-fail-open path skipped the
                # reserve, so there is nothing to commit.
                return

            outcome_label = _classify_exception(exception)
            if exception is None:
                # SUCCESS path — reconcile real usage.
                try:
                    real_claims = self._claim_reconciler(outputs)
                except Exception as rec_exc:  # noqa: BLE001
                    log.warning(
                        "spendguard.integrations.dspy: claim_reconciler "
                        "raised %r for call_id=%s; falling back to "
                        "estimator snapshot.",
                        rec_exc,
                        call_id,
                    )
                    real_claims = []
                if real_claims:
                    if len(real_claims) != 1:
                        log.warning(
                            "spendguard.integrations.dspy: claim_reconciler "
                            "returned %d claims; v1 expects 1 — using first.",
                            len(real_claims),
                        )
                    real_claim = real_claims[0]
                    estimated_amount = str(
                        getattr(real_claim, "amount_atomic", "0") or "0"
                    )
                else:
                    # Reconciler returned no claim; prefer on-the-wire
                    # usage if available, else estimator snapshot.
                    usage_total = _extract_total_tokens(outputs)
                    estimated_amount = (
                        str(usage_total)
                        if usage_total > 0
                        else state.estimator_amount_atomic
                    )
                provider_event_id = _extract_provider_event_id(outputs)
            else:
                # FAILURE / CANCELLED path — emit release.
                estimated_amount = "0"
                provider_event_id = ""

            try:
                self._bridge.run(
                    self._client.emit_llm_call_post(
                        run_id=state.run_id,
                        step_id=state.step_id,
                        llm_call_id=state.llm_call_id,
                        decision_id=state.decision_id,
                        reservation_id=state.reservation_id,
                        provider_reported_amount_atomic="",
                        estimated_amount_atomic=estimated_amount,
                        unit=state.unit,
                        pricing=state.pricing,
                        provider_event_id=provider_event_id,
                        outcome=outcome_label,
                    )
                )
            except SpendGuardError as post_exc:
                # Best-effort: log and swallow so we never mask the
                # original LM exception DSPy is about to propagate.
                log.warning(
                    "spendguard.integrations.dspy: emit_llm_call_post "
                    "failed for call_id=%s err=%r; reservation will "
                    "TTL-sweep.",
                    call_id,
                    post_exc,
                )
            except RuntimeError as rt_exc:
                # The background bridge can raise RuntimeError if its loop
                # failed to start or the bridge was closed; tolerate it so
                # the original LM exception path is preserved. The
                # reservation TTL-sweeps as the durable backstop.
                log.warning(
                    "spendguard.integrations.dspy: emit_llm_call_post "
                    "raised RuntimeError for call_id=%s err=%r; bridge "
                    "loop unavailable — reservation will TTL-sweep.",
                    call_id,
                    rt_exc,
                )
            except Exception as misc_exc:  # noqa: BLE001
                # Belt-and-braces: never raise from on_lm_end.
                log.warning(
                    "spendguard.integrations.dspy: emit_llm_call_post "
                    "best-effort raised %r for call_id=%s; reservation "
                    "will TTL-sweep.",
                    misc_exc,
                    call_id,
                )
        finally:
            # Reset the contextvar regardless of how the body exits.
            # Use a try/except to tolerate cross-context token errors
            # (rare; happens when a token is restored in a different
            # contextvars context than the one it was set in).
            if state.shim_token is not None:
                try:
                    _SHIM_IN_FLIGHT.reset(state.shim_token)
                except (ValueError, LookupError):
                    # Token belongs to a different context — best
                    # effort, clear via a fresh set+reset cycle.
                    pass

    # ─────────────────────────────────────────────────────────────────
    # Helpers
    # ─────────────────────────────────────────────────────────────────

    def _guard_async_context(self) -> None:
        """Raise ``SyncInAsyncContext`` if invoked inside a running loop.

        DSPy 2.6 callbacks are sync; we use ``asyncio.run`` for the
        sidecar dispatch. ``asyncio.run`` itself would raise
        ``RuntimeError`` from inside a running loop with a confusing
        message — we raise this typed exception with a clear hint
        instead. The check is sticky (``try/except RuntimeError``)
        because ``asyncio.get_running_loop()`` raises when no loop is
        active, which is the success case.
        """
        try:
            asyncio.get_running_loop()
        except RuntimeError:
            return
        raise SpendGuardDSPyCallback.SyncInAsyncContext(
            "SpendGuardDSPyCallback.on_lm_start cannot be invoked from "
            "inside a running event loop (DSPy callbacks are sync and we "
            "would call asyncio.run from within a loop). Run dspy calls "
            "from a sync entrypoint or pre-emit reservations via the "
            "SpendGuardClient directly."
        )

    def _sweep_pending(self) -> None:
        """Drop ``_PENDING`` entries older than the TTL.

        Fast-path returns when ``_PENDING`` is empty. Otherwise does a
        linear scan and pops stale entries. Logs a WARN per dropped
        entry so operators see leaks in the demo log.
        """
        if not _PENDING:
            return
        now = time.monotonic()
        stale = [
            cid
            for cid, st in _PENDING.items()
            if (now - st.started_at) > _PENDING_TTL_SECONDS
        ]
        for cid in stale:
            log.warning(
                "spendguard.integrations.dspy: TTL-sweeping stale "
                "call_id=%s (no on_lm_end after %ds)",
                cid,
                _PENDING_TTL_SECONDS,
            )
            # Reset the contextvar token if the swept entry had one, so
            # a leaked True doesn't permanently block D12.
            state = _PENDING.pop(cid, None)
            if state is not None and state.shim_token is not None:
                try:
                    _SHIM_IN_FLIGHT.reset(state.shim_token)
                except (ValueError, LookupError):
                    pass

    def _default_estimator(
        self,
        inputs: dict[str, Any],
        binding: BudgetBinding,
    ) -> list[Any]:
        """Fallback estimator when ``claim_estimator`` is None.

        DSPy inputs typically carry ``messages: list[dict]`` (chat) or
        ``prompt: str`` (completions). Estimate via chars/4 — same
        heuristic as the other adapters' default fallback.

        Returns a single-element list with a ``common_pb2.BudgetClaim``
        when proto is available; falls back to a duck-typed object
        with the same field shape when proto isn't (unit test path).
        """
        chars = 0
        messages = inputs.get("messages") if isinstance(inputs, dict) else None
        if isinstance(messages, list):
            for m in messages:
                if isinstance(m, dict):
                    content = m.get("content", "")
                    if isinstance(content, str):
                        chars += len(content)
                    else:
                        chars += len(str(content))
        prompt = inputs.get("prompt") if isinstance(inputs, dict) else None
        if isinstance(prompt, str):
            chars += len(prompt)
        elif prompt is not None:
            chars += len(str(prompt))
        projected = max(50, chars // 4)
        if _PROTO_AVAILABLE and common_pb2 is not None:
            return [
                common_pb2.BudgetClaim(
                    budget_id=binding.budget_id,
                    unit=binding.unit,
                    amount_atomic=str(projected),
                    direction=common_pb2.BudgetClaim.DEBIT,
                    window_instance_id=binding.window_instance_id,
                )
            ]
        # Duck-typed fallback for unit-test path where proto isn't
        # importable. Carries the same surface the validator reads.
        from types import SimpleNamespace

        return [
            SimpleNamespace(
                budget_id=binding.budget_id,
                unit=binding.unit,
                amount_atomic=str(projected),
                window_instance_id=binding.window_instance_id,
            )
        ]

    def close(self) -> None:
        """Stop the background bridge loop + join its thread.

        Operators should call this on application shutdown. The client
        is owned by the caller and is NOT closed here. Idempotent.
        """
        self._bridge.close()

    def __del__(self) -> None:  # pragma: no cover — GC timing-dependent
        """Best-effort bridge tear-down at GC time."""
        try:
            self._bridge.close()
        except Exception:  # noqa: BLE001, S110 — GC best-effort
            pass  # noqa: S110

    # ─────────────────────────────────────────────────────────────────
    # Test surface — inspectors
    # ─────────────────────────────────────────────────────────────────

    @property
    def pending_count(self) -> int:
        """Number of LM calls currently awaiting ``on_lm_end``.

        Exposed for tests asserting stash isolation under concurrent
        DSPy dispatch. Operators should treat this as a private metric.
        """
        return len(_PENDING)


__all__ = [
    "BudgetResolver",
    "ClaimEstimator",
    "ClaimReconciler",
    "SpendGuardDSPyCallback",
    "_PENDING",
    "_PENDING_TTL_SECONDS",
    "_classify_exception",
    "_extract_provider_event_id",
    "_extract_total_tokens",
    "_signature_from_inputs",
]
