# ruff: noqa: ANN401
"""``wrap_instructor_client`` — Atomic Agents Instructor-wrap adapter.

Implements the D28 design: composition-based proxy around
``instructor.Instructor`` / ``instructor.AsyncInstructor`` that gates
EVERY provider HTTP call — including Instructor's internal
**validation-retry loop** — through SpendGuard's
``RequestDecision(LLM_CALL_PRE)`` → ``EmitLlmCallPost`` lifecycle.

Architectural rationale (per design.md §1 / review-standards §1.1)::

    BaseAgent.run(...)
      → self.client.chat.completions.create_with_completion(
            model=..., messages=..., response_model=output_schema, ...)
        → SpendGuardInstructorProxy.chat.completions.create_with_completion(...)
          ├─ inner.create_with_completion(...)          [Instructor's outer call]
          │   └─ instructor.core.retry.retry_sync(func=self.create_fn, ...)
          │       └─ for each attempt:
          │           ├─ inner.create_fn(messages, **kwargs)
          │           │     ↓ INTERCEPTED HERE
          │           └─ _gated_create_fn(messages, **kwargs)
          │               ├─ signature = blake2b(messages | model | response_model.qualname | tools)
          │               ├─ sidecar.RequestDecision(LLM_CALL_PRE)
          │               │     ALLOW → call original create_fn
          │               │     DENY  → raise DecisionDenied
          │               ├─ result = original_create_fn(messages, **kwargs)  [provider HTTP]
          │               └─ sidecar.emit_llm_call_post(SUCCESS|FAILURE|CANCELLED,
          │                                             estimated=usage.total_tokens)

DEVIATION-C vs design.md §4 (locked): the spec described wrapping
``chat.completions.create_with_completion`` directly with the claim
"Instructor's internal retries re-enter this proxy → each gets its
own reservation". Reality (verified against ``instructor==1.14.5``
and ``1.15.1``): Instructor's outer ``create_with_completion`` is
called ONCE; ``instructor.core.retry.retry_sync`` then calls
``self.create_fn`` directly per attempt (``create_fn`` is the raw
``openai_client.chat.completions.create`` method captured at
``from_openai(...)`` time — see ``instructor/core/retry.py:retry_sync``
``func(*args, **kwargs)`` per attempt). Wrapping the outer
``create_with_completion`` boundary fires the gate ONCE for the whole
retry loop — undercount.

The correct gate point is ``Instructor.create_fn``. The proxy
intercepts it at construction time, replacing it with a gated
closure. Each retry attempt's call to ``create_fn`` is gated. The
proxy's own ``chat.completions.create_with_completion`` is a
pass-through to ``inner.create_with_completion`` — no double gate.

Why wrap the Instructor object, NOT the raw provider SDK
(per design.md §1 / review-standards §1.1):

  - ``instructor.from_openai(openai.OpenAI(...))`` captures the
    OpenAI client's ``chat.completions.create`` method as
    ``Instructor.create_fn``. Wrapping the raw OpenAI client AFTER
    construction would NOT update ``create_fn`` — Instructor's
    retry loop would still call the original unwrapped method.

  - Wrapping ``Instructor.create_fn`` directly (composition + closure)
    captures every retry naturally because the retry loop calls
    ``create_fn`` per attempt. Each retry's ``messages`` differs by
    the injected validation-error message (see ``instructor.core.retry``
    ``handle_reask_kwargs``), so ``_signature(kwargs)`` diverges —
    fresh ``llm_call_id`` per attempt — without an explicit retry
    counter (review-standards §2.2 makes an explicit counter a
    Blocker).

Per review-standards §1.2:
  - ``SpendGuardInstructorProxy`` / ``SpendGuardAsyncInstructorProxy``
    inherit from ``_ProxyBase`` (plain object). Inheriting from
    ``instructor.Instructor`` / ``instructor.AsyncInstructor`` is a
    Blocker — those classes use ``__init_subclass__`` machinery and
    accept private kwargs that will break under upstream churn.
  - Constructor does NOT call ``instructor.Instructor.__init__`` or
    ``instructor.AsyncInstructor.__init__``.
  - ``__getattr__`` delegates unknown attribute lookups to
    ``self._inner``. Required so ``proxy.mode``, ``proxy.create_kwargs``,
    and any future Instructor attrs remain reachable.

Sync ↔ async dispatch (per review-standards §1.4):
  - Factory dispatches on ``isinstance(client, AsyncInstructor)``
    FIRST (because ``AsyncInstructor`` is the more specific type),
    then ``Instructor``. Reversing the order silently routes async
    clients to the sync proxy and is a Blocker.

Sync proxy ↔ async sidecar bridging:
  - Instructor's sync ``create_with_completion`` is invoked from
    synchronous user code (``BaseAgent.run``). The SpendGuard sidecar
    only exposes async methods. We bridge via ``asyncio.run(...)`` —
    matches the DSPy callback pattern.  ``asyncio.run`` raises
    ``RuntimeError`` from inside a running loop; we surface that as
    ``SpendGuardConfigError`` so the operator gets a clear hint to
    use ``AsyncInstructor`` instead.

Shared run-context (per review-standards §1.5):
  - ``RunContext`` / ``run_context()`` / ``current_run_context()`` are
    IMPORTED from ``spendguard.integrations.openai_agents``, NOT
    redefined. Polyglot stacks mixing OpenAI Agents / AutoGen /
    Atomic Agents in one app share a single trace because all four
    adapters read the same module-level ``spendguard_run_context``
    contextvar.
  - Resilient import: ``..openai_agents`` ImportErrors when the
    ``[openai-agents]`` extra isn't installed. The shared contextvar
    mechanics (``RunContext`` dataclass + ``run_context`` async ctx
    manager + ``current_run_context`` lookup) don't depend on the
    openai-agents package — only the integration's wrapper class
    does. We import the three symbols defensively and fall back to a
    local mirror (same contextvar NAME) when the barrel fails.

Per review-standards §6 fail-closed is the only mode. No
``SPENDGUARD_ATOMIC_AGENTS_FAIL_OPEN`` env knob exists.
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
# instructor import — required for proxy construction but defensive
# at import time so the test suite can load _hook directly via
# package-path bypass (mirrors dspy/_wrapper.py + autogen/_hook.py).
# ─────────────────────────────────────────────────────────────────────

try:  # pragma: no cover — branch chosen at import time
    from instructor import (  # type: ignore[import-not-found]
        AsyncInstructor as _AsyncInstructor,
    )
    from instructor import (  # type: ignore[import-not-found]
        Instructor as _Instructor,
    )

    _INSTRUCTOR_AVAILABLE = True
except ImportError:  # pragma: no cover — branch chosen at import time
    _Instructor = None  # type: ignore[assignment, misc]
    _AsyncInstructor = None  # type: ignore[assignment, misc]
    _INSTRUCTOR_AVAILABLE = False


# ─────────────────────────────────────────────────────────────────────
# Shared run-context — REUSED from openai_agents per review-standards
# §1.5. Polyglot agent stacks share a single trace because all
# integration adapters read the same module-level
# ``spendguard_run_context`` contextvar.
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
    # with this adapter. Byte-for-byte identical contextvar binding to
    # openai_agents so cross-framework run_id sharing still works at
    # runtime via the interpreter-level contextvar name registry.
    import contextvars
    from contextlib import asynccontextmanager
    from dataclasses import dataclass

    _RUN_CONTEXT: contextvars.ContextVar["RunContext | None"] = (
        contextvars.ContextVar("spendguard_run_context", default=None)
    )

    @dataclass(frozen=True, slots=True)
    class RunContext:  # type: ignore[no-redef]
        """Per ``BaseAgent.run()`` identifiers.

        Mirrors ``spendguard.integrations.openai_agents.RunContext``
        when the ``[openai-agents]`` extra is not installed. Same
        contextvar NAME means a parent LangChain / Pydantic-AI /
        Strands / BeeAI / AutoGen run shares the run_id with this
        adapter regardless of which fallback branch fired at import
        time.
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
                "spendguard.integrations.atomic_agents called outside an "
                "active run_context(). Wrap your BaseAgent.run invocation:\n\n"
                "    async with run_context(RunContext(run_id=...)):\n"
                "        result = agent.run({...})\n"
            )
        return ctx


log = logging.getLogger("spendguard.integrations.atomic_agents")


# ─────────────────────────────────────────────────────────────────────
# Type aliases (public surface)
# ─────────────────────────────────────────────────────────────────────

ClaimEstimator = Callable[[dict[str, Any]], list[Any]]
"""Project ``BudgetClaim`` list from Instructor's create-call kwargs.

The estimator gets the FULL kwargs dict that ``BaseAgent.run`` /
``agent.run`` will forward to ``Instructor.chat.completions.create*``.
Keys typically present: ``model``, ``messages``, ``response_model``,
``tools``, ``tool_choice``, ``max_retries``, ``validation_context``,
plus any provider-specific kwargs.

Per design.md §5 / review-standards §1.3: NO default ``claim_estimator``
ships with the adapter because Instructor's polyglot routing
(OpenAI / Anthropic / Gemini / Cohere) makes any single default
wrong. The operator picks the estimator matching the inner
Instructor's provider; ``spendguard.integrations.openai_agents._default_estimator``
covers the OpenAI case as a starting point.

v1 contract: returns >= 1 claim. Empty list is a config error.
"""


# ─────────────────────────────────────────────────────────────────────
# Helpers (module-level so they're testable + reusable)
# ─────────────────────────────────────────────────────────────────────


def _signature(kwargs: dict[str, Any]) -> str:
    """Derive a stable 16-byte BLAKE2b signature from Instructor kwargs.

    Includes:
      * ``model`` — review-standards §3.2: ``gpt-4o`` → ``gpt-4o-mini``
        swap MUST yield a fresh ``llm_call_id`` so a tenant can't
        change cost class under one reservation. Blocker if omitted.
      * ``messages`` — load-bearing for validation-retry divergence
        (each retry injects a validation-error message → signature
        differs naturally → fresh reservation per attempt).
      * ``response_model`` identity (qualified class name) —
        review-standards §3.1: omitting lets a tenant flip schema
        mid-reservation. Blocker if missing.
      * ``tools`` / ``tool_choice`` — affect provider routing and
        therefore cost.

    Per review-standards §3.3 the signature is deterministic — uses
    ``repr()`` on inputs; the signature is opaque to the rest of the
    pipeline (we hash it, never log raw messages — Atomic Agents'
    Pydantic schemas commonly carry user PII).

    blake2b-16 matches the OpenAI Agents / Agno / DSPy / AutoGen
    integrations' signature width so cross-framework ID derivation is
    symmetric.
    """
    response_model = kwargs.get("response_model")
    if response_model is None:
        rm_repr = ""
    else:
        # qualified class name — stable across processes for a given
        # Pydantic model class. ``__qualname__`` includes the enclosing
        # class for nested types so two distinct response schemas with
        # the same short name on different agents don't collide.
        rm_module = getattr(response_model, "__module__", "")
        rm_qualname = getattr(
            response_model, "__qualname__", type(response_model).__name__
        )
        rm_repr = f"{rm_module}.{rm_qualname}"
    text = (
        f"model={kwargs.get('model')!r}|"
        f"messages={kwargs.get('messages')!r}|"
        f"response_model={rm_repr}|"
        f"tools={kwargs.get('tools')!r}|"
        f"tool_choice={kwargs.get('tool_choice')!r}"
    )
    return hashlib.blake2b(text.encode("utf-8"), digest_size=16).hexdigest()


def _extract_total_tokens(raw_completion: Any) -> int:
    """Extract usage total tokens from a raw ``ChatCompletion``.

    Per review-standards §2.4 the precedence is:
      1. ``usage.total_tokens`` when present and an int.
      2. ``usage.prompt_tokens + usage.completion_tokens`` fallback.
      3. ``0`` when ``usage`` is absent / non-numeric.

    Returning ``0`` for absent usage is the correct fail-soft signal —
    the projector still commits the reservation with a zero estimated
    amount. Raising here would block the audit chain (Blocker per
    review-standards §2.4).
    """
    if raw_completion is None:
        return 0
    usage = getattr(raw_completion, "usage", None)
    if usage is None:
        return 0
    total = getattr(usage, "total_tokens", None)
    if isinstance(total, int):
        return total
    prompt = getattr(usage, "prompt_tokens", 0) or 0
    completion = getattr(usage, "completion_tokens", 0) or 0
    try:
        return int(prompt) + int(completion)
    except (TypeError, ValueError):
        # Defensive: usage fields might be non-numeric on a custom
        # provider transport. Better to report 0 than crash the
        # audit chain.
        return 0


def _extract_provider_event_id(raw_completion: Any) -> str:
    """Read ``ChatCompletion.id`` for audit correlation.

    OpenAI-shaped ``ChatCompletion`` carries an ``id`` like
    ``chatcmpl-abc123``. Returns ``""`` when absent; raising here is
    a Blocker per review-standards §2.4.
    """
    if raw_completion is None:
        return ""
    return str(getattr(raw_completion, "id", "") or "")


def _classify_exception(exc: BaseException) -> str:
    """Classify an inner-call exception into a POST outcome label.

    Per review-standards §2.3 we use ``type(exc).__name__ ==
    "CancelledError"`` to avoid cross-loop ``isinstance`` mismatches
    across ``asyncio`` / ``trio`` / ``anyio``. Atomic Agents runs on
    ``asyncio`` (BaseAgent is sync; async variant uses asyncio); the
    sync proxy bridges via ``asyncio.run`` per file-level docs.

    Returns:
      * ``"CANCELLED"`` when the exception type name matches.
      * ``"FAILURE"`` for every other exception.
    """
    if type(exc).__name__ == "CancelledError":
        return "CANCELLED"
    return "FAILURE"


def _unpack_create_result(method_name: str, result: Any) -> Any:
    """Extract the raw ``ChatCompletion`` from an Instructor result.

    ``create_with_completion`` returns ``tuple[ParsedModel, ChatCompletion]``.
    ``create`` returns ``ParsedModel`` whose ``_raw_response`` attr
    holds the raw ``ChatCompletion`` (Instructor's documented private
    attribute since 1.5.x).

    Per review-standards §1.3: falling through to a zero-cost POST
    without consulting ``_raw_response`` on the ``.create()`` path is
    a Blocker (undercount path).
    """
    if method_name == "create_with_completion":
        # Instructor returns (parsed, raw_completion). Defensive
        # unpack — older Instructor versions / custom Mode handlers
        # might return a different shape; treat any non-2-tuple as
        # raw=None so the POST emits a zero-cost commit rather than
        # crashing the audit chain.
        if isinstance(result, tuple) and len(result) == 2:
            return result[1]
        return None
    # .create() returns parsed-only; raw is on the private attr.
    return getattr(result, "_raw_response", None)


# ─────────────────────────────────────────────────────────────────────
# Async sidecar bridging for the sync proxy
# ─────────────────────────────────────────────────────────────────────


class _SyncInAsyncContext(SpendGuardConfigError):
    """Sync proxy method invoked from inside a running event loop.

    Instructor's sync ``Instructor.chat.completions.create*`` is meant
    for synchronous user code. The SpendGuard sidecar exposes only
    async methods, so the sync proxy uses ``asyncio.run(...)`` to
    bridge. ``asyncio.run`` raises ``RuntimeError`` from inside a
    running loop; we surface this typed exception instead so the
    caller gets a clear, actionable hint.

    Resolution: either run BaseAgent from a sync entrypoint, OR
    switch to ``instructor.from_openai(AsyncOpenAI(...))`` so the
    factory dispatches to ``SpendGuardAsyncInstructorProxy`` (which
    awaits the sidecar directly — no ``asyncio.run`` needed).
    """


def _guard_async_context() -> None:
    """Raise ``_SyncInAsyncContext`` if invoked inside a running loop.

    Detects a running loop via ``asyncio.get_running_loop()`` (raises
    when no loop is running). Required guard before ``asyncio.run(...)``
    in the sync proxy gated-call path.
    """
    try:
        asyncio.get_running_loop()
    except RuntimeError:
        return
    raise _SyncInAsyncContext(
        "SpendGuardInstructorProxy.chat.completions.create* invoked "
        "from inside a running event loop. Atomic Agents sync "
        "`BaseAgent.run` must be called from a sync entrypoint, OR "
        "switch to `instructor.from_openai(AsyncOpenAI(...))` so the "
        "factory dispatches to SpendGuardAsyncInstructorProxy "
        "(which awaits the sidecar directly without asyncio.run)."
    )


# ─────────────────────────────────────────────────────────────────────
# Proxy classes
# ─────────────────────────────────────────────────────────────────────


class _ChatCompletionsPassthrough:
    """Pass-through for ``Instructor.chat.completions``.

    The gate fires at ``Instructor.create_fn`` (the per-attempt
    boundary), not at this layer. Atomic Agents calls
    ``proxy.chat.completions.create_with_completion(...)`` — we
    delegate verbatim to ``inner.chat.completions.create_with_completion``
    (which in Instructor 1.14+ resolves to
    ``inner.create_with_completion`` because ``Instructor.chat`` is
    ``self`` and ``Instructor.completions`` is ``self``).

    Per DEVIATION-C the gate cannot live here — the retry loop calls
    ``create_fn`` directly, bypassing this surface. The gate IS at
    ``create_fn``; this surface is a thin proxy so Atomic Agents'
    duck-type check (`callable(client.chat.completions.create_with_completion)`)
    still succeeds and a call here reaches Instructor's retry loop
    which then calls the gated ``create_fn`` per attempt.
    """

    def __init__(self, proxy: "_ProxyBase") -> None:
        self._proxy = proxy

    def create(self, *args: Any, **kwargs: Any) -> Any:
        return self._proxy._inner.chat.completions.create(*args, **kwargs)

    def create_with_completion(self, *args: Any, **kwargs: Any) -> Any:
        return self._proxy._inner.chat.completions.create_with_completion(
            *args, **kwargs
        )


class _ChatNamespace:
    """Mirrors ``Instructor.chat`` (holds ``completions``).

    The gate lives at ``Instructor.create_fn`` — this namespace tree
    is a thin pass-through so Atomic Agents'
    ``client.chat.completions.create_with_completion(...)`` reaches
    Instructor's outer create which then drives the retry loop and
    calls our gated ``create_fn`` per attempt.
    """

    def __init__(self, completions: _ChatCompletionsPassthrough) -> None:
        self.completions = completions


class _ProxyBase:
    """Composition-based base for sync + async Instructor proxies.

    Per review-standards §1.2:
      - Inherits from plain object (no ``Instructor`` /
        ``AsyncInstructor`` subclass — those use ``__init_subclass__``
        magic and accept private kwargs).
      - Does NOT call ``inner.__init__()``.
      - ``__getattr__`` delegates unknown attribute lookups to
        ``self._inner`` so ``proxy.mode``, ``proxy.create_kwargs``,
        ``proxy.default_model``, and any future Instructor attribute
        remains reachable.
      - ``__getattr__`` carries NO side effects (no logging, no
        metrics, no caching) per review-standards §1.2.
    """

    def __init__(
        self,
        *,
        inner: Any,
        spendguard_client: SpendGuardClient,
        budget_id: str,
        window_instance_id: str,
        unit: Any,
        pricing: Any,
        claim_estimator: ClaimEstimator,
        route: str = "llm.call",
    ) -> None:
        if inner is None:
            raise SpendGuardConfigError(
                "SpendGuardInstructorProxy(inner=...) is required; got None."
            )
        if spendguard_client is None:
            raise SpendGuardConfigError(
                "SpendGuardInstructorProxy(spendguard_client=...) is required; "
                "got None."
            )
        if not budget_id:
            raise SpendGuardConfigError(
                "SpendGuardInstructorProxy(budget_id=...) is required."
            )
        if not window_instance_id:
            raise SpendGuardConfigError(
                "SpendGuardInstructorProxy(window_instance_id=...) is required."
            )
        unit_id = getattr(unit, "unit_id", "") if unit is not None else ""
        if not unit_id:
            raise SpendGuardConfigError(
                "SpendGuardInstructorProxy unit.unit_id is required."
            )
        if claim_estimator is None:
            raise SpendGuardConfigError(
                "SpendGuardInstructorProxy(claim_estimator=...) is required; "
                "design.md §5 locks no default estimator because Instructor's "
                "polyglot routing (OpenAI / Anthropic / Gemini / Cohere) "
                "makes any single default wrong."
            )
        self._inner = inner
        self._client = spendguard_client
        self._budget_id = budget_id
        self._window_instance_id = window_instance_id
        self._unit = unit
        self._pricing = pricing
        self._claim_estimator = claim_estimator
        self._route = route

    def __getattr__(self, name: str) -> Any:
        """Delegate unknown attribute lookups to the inner Instructor.

        Only fires when normal attribute resolution misses; our
        explicit attrs (``chat``, ``_inner``, ``_client``, etc.)
        shadow correctly because Python looks them up on the instance
        dict / class dict before ``__getattr__``.

        Per review-standards §1.2: NO side effects here (no logging,
        no metric emission, no cache write). Blocker if added.
        """
        # AttributeError-ing through to __getattr__ on attribute names
        # starting with underscore would create infinite recursion
        # because most introspection helpers reach for ``_inner``
        # before it's set. Guard explicitly.
        if name.startswith("_"):
            raise AttributeError(name)
        return getattr(self._inner, name)

    # ─────────────────────────────────────────────────────────────────
    # Shared gated-call core. Sync + async proxies wrap this with the
    # appropriate await / asyncio.run plumbing.
    # ─────────────────────────────────────────────────────────────────

    def _derive_ids(
        self, ctx: "RunContext", signature: str
    ) -> tuple[str, str, str, str]:
        """Derive deterministic (llm_call_id, decision_id, step_id, idem_key).

        Each Instructor validation retry naturally diverges on
        ``messages`` → different ``signature`` → different
        ``llm_call_id`` → different ``idempotency_key``. Per
        review-standards §2.2 this is the load-bearing mechanism that
        gives each retry its own reservation, with NO explicit retry
        counter (review-standards §2.2 makes an explicit counter a
        Blocker — it would create fragile coupling to Instructor's
        internal retry state).
        """
        llm_call_id = str(
            derive_uuid_from_signature(signature, scope="llm_call_id")
        )
        decision_id = str(
            derive_uuid_from_signature(signature, scope="decision_id")
        )
        step_id = f"{ctx.run_id}:atomic-agents:{signature[:16]}"
        idempotency_key = derive_idempotency_key(
            tenant_id=self._client.tenant_id,
            session_id=self._client.session_id,
            run_id=ctx.run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )
        return llm_call_id, decision_id, step_id, idempotency_key

    def _build_decision_context(self) -> dict[str, Any]:
        """Stable audit-context tag for canonical_events filtering.

        Captures the inner Instructor's class name + the inner
        client's class name (when reachable) for dashboards grouping
        by Instructor mode / provider backend.
        """
        decision_context: dict[str, Any] = {"integration": "atomic_agents"}
        inner_type = type(self._inner).__name__
        if inner_type:
            decision_context["inner_client"] = inner_type
        # Best-effort: Instructor wraps a provider client at
        # ``self.client`` or ``self.create_kwargs`` depending on Mode.
        # Tag the provider class name when reachable; never raise.
        try:
            provider_client = getattr(self._inner, "client", None)
            if provider_client is not None:
                decision_context["provider_client"] = type(
                    provider_client
                ).__name__
        except Exception:  # noqa: BLE001
            # Defensive: Instructor exposes a __getattr__ that may
            # raise for unknown providers. Audit tag is best-effort.
            pass
        return decision_context


class SpendGuardInstructorProxy(_ProxyBase):
    """Sync Instructor proxy. Atomic Agents ``BaseAgent.run`` (sync).

    Per DEVIATION-C / file-level docs: the gate sits at the raw
    provider method (``inner.client.chat.completions.create``) — the
    function Instructor's ``retry_sync`` calls per attempt. We wrap
    that raw method with a gated closure, then re-run
    ``instructor.patch(create=gated_raw, mode=inner.mode)`` to mint a
    new ``create_fn`` that drives Instructor's retry loop against the
    gated raw method.

    Why not wrap ``inner.create_fn`` directly: Instructor's
    ``patch(...)`` captures the raw method via closure at patch time;
    the patched ``create_fn`` calls ``retry_sync(func=raw_create,
    ...)`` which calls ``raw_create(*args, **kwargs)`` per attempt.
    Replacing ``inner.create_fn`` with a gated closure intercepts
    ONLY the outer call (once per ``create_with_completion`` call) —
    the retry loop INSIDE the patched ``create_fn`` continues to call
    the un-wrapped raw method. Wrapping the raw method directly is
    the load-bearing intercept point.

    Per review-standards §1.4 / file-level docs: invoked from
    synchronous user code, bridges to the async sidecar via
    ``asyncio.run(...)``. Raises ``_SyncInAsyncContext`` (a typed
    ``SpendGuardConfigError`` subclass) when invoked inside a running
    event loop — operator should switch to ``AsyncInstructor``.
    """

    def __init__(self, **kwargs: Any) -> None:
        super().__init__(**kwargs)
        # Build the chat namespace tree — pass-through; the gate lives
        # at the raw provider method (see DEVIATION-C in file-level
        # docs).
        self.chat = _ChatNamespace(_ChatCompletionsPassthrough(self))
        # Wrap the raw provider method and re-patch the inner
        # Instructor so its retry loop drives our gated raw method
        # per attempt. Each retry attempt re-enters the gate naturally.
        self._original_create_fn = self._inner.create_fn
        self._original_raw_create = self._resolve_raw_create(self._inner)
        gated_raw = self._make_gated_raw_create(self._original_raw_create)
        # Expose the gated raw method as ``self._gated_raw_create`` so
        # unit tests can drive it directly without going through
        # Instructor's retry layer. Production callers route via
        # ``inner.create_fn`` (the re-patched function) which drives
        # ``retry_sync(func=gated_raw, ...)``.
        self._gated_raw_create = gated_raw
        # Re-patch using instructor.patch — preserves Mode-specific
        # behavior (TOOLS / JSON / FUNCTIONS / etc.).
        try:
            import instructor as _instructor_mod
        except ImportError as exc:  # pragma: no cover — already guarded at module load
            raise SpendGuardConfigError(
                "instructor is not importable; cannot re-patch create_fn. "
                "Install via: pip install 'spendguard-sdk[atomic-agents]'"
            ) from exc
        self._inner.create_fn = _instructor_mod.patch(
            create=gated_raw, mode=self._inner.mode
        )

    @staticmethod
    def _resolve_raw_create(inner: Any) -> Callable[..., Any]:
        """Locate the raw provider ``create`` method behind Instructor.

        Tries ``inner.client.chat.completions.create`` first (standard
        OpenAI / Anthropic / Azure shape). Falls back to walking
        ``inner.create_fn.__wrapped__`` (instructor.patch uses
        ``functools.wraps`` so the original is reachable).
        """
        client = getattr(inner, "client", None)
        if client is not None:
            try:
                return client.chat.completions.create
            except AttributeError:
                pass
        # Fallback: walk __wrapped__ chain.
        fn = getattr(inner, "create_fn", None)
        if fn is not None and hasattr(fn, "__wrapped__"):
            return fn.__wrapped__
        raise SpendGuardConfigError(
            "Could not locate the raw provider create method on the "
            "Instructor instance. Expected `inner.client.chat.completions.create` "
            "or `inner.create_fn.__wrapped__`. Pass an Instructor built "
            "via `instructor.from_openai(client)` so the raw method is "
            "reachable for per-attempt gating."
        )

    def _make_gated_raw_create(
        self, raw_create: Callable[..., Any]
    ) -> Callable[..., Any]:
        """Build a sync gated wrapper around the raw provider create.

        Each invocation (one per Instructor retry attempt) goes
        through PRE / inner / POST. Per DEVIATION-C this is the
        load-bearing intercept that fires per retry — the raw
        provider method is what Instructor's ``retry_sync`` calls in
        a loop.
        """
        proxy = self

        from functools import wraps

        @wraps(raw_create)
        def _gated_raw_create(*args: Any, **kwargs: Any) -> Any:
            proxy._sync_gate_attempt(args=args, kwargs=kwargs)
            try:
                result = raw_create(*args, **kwargs)
            except BaseException as exc:
                proxy._sync_post_failure(exc)
                raise
            proxy._sync_post_success(result)
            return result

        return _gated_raw_create

    # ── Per-attempt gate state (set by PRE, consumed by POST) ────────
    # The gate fires PRE before the original create_fn call, captures
    # outcome + run_id + ids on the proxy instance, then POST reads
    # them after the inner call returns. Sequential per-thread call
    # pattern (Instructor's sync retry loop is sequential) means we
    # don't need a per-call dict; one set of fields is sufficient.
    #
    # NOTE: this DOES make SpendGuardInstructorProxy non-reentrant
    # across concurrent threads sharing one proxy. Atomic Agents'
    # sync BaseAgent.run is single-threaded per agent instance, so
    # this is consistent with the upstream concurrency model.

    def _sync_gate_attempt(self, *, args: tuple[Any, ...], kwargs: dict[str, Any]) -> None:
        """Run PRE on a per-attempt basis. Stash outcome for POST.

        Bridges to the async sidecar via ``asyncio.run(...)``.
        """
        _guard_async_context()
        ctx = current_run_context()
        # Instructor's retry loop passes the create_fn args verbatim;
        # the first positional or messages= kwarg + model= kwarg are
        # the load-bearing signature contributors. Build the kwargs
        # dict for _signature.
        sig_kwargs = dict(kwargs)
        signature = _signature(sig_kwargs)
        llm_call_id, decision_id, step_id, idem_key = self._derive_ids(
            ctx, signature
        )
        projected_claims = self._claim_estimator(sig_kwargs)
        decision_context = self._build_decision_context()

        outcome: DecisionOutcome = asyncio.run(
            self._client.request_decision(
                trigger="LLM_CALL_PRE",
                run_id=ctx.run_id,
                step_id=step_id,
                llm_call_id=llm_call_id,
                tool_call_id="",
                decision_id=decision_id,
                route=self._route,
                projected_claims=projected_claims,
                idempotency_key=idem_key,
                projected_unit=self._unit,
                decision_context_json=decision_context,
            )
        )
        # Stash for POST.
        self._pending_attempt = {
            "ctx": ctx,
            "outcome": outcome,
            "llm_call_id": llm_call_id,
            "step_id": step_id,
            "signature": signature,
        }

    def _sync_post_failure(self, exc: BaseException) -> None:
        """POST fires for FAILURE / CANCELLED when a reservation exists."""
        pending = getattr(self, "_pending_attempt", None)
        if not pending:
            return
        outcome = pending["outcome"]
        ctx = pending["ctx"]
        signature = pending["signature"]
        try:
            if outcome.reservation_ids:
                outcome_kind = _classify_exception(exc)
                try:
                    asyncio.run(
                        self._client.emit_llm_call_post(
                            run_id=ctx.run_id,
                            step_id=pending["step_id"],
                            llm_call_id=pending["llm_call_id"],
                            decision_id=outcome.decision_id,
                            reservation_id=outcome.reservation_ids[0],
                            provider_reported_amount_atomic="",
                            estimated_amount_atomic="0",
                            unit=self._unit,
                            pricing=self._pricing,
                            provider_event_id="",
                            outcome=outcome_kind,
                        )
                    )
                except Exception as post_exc:  # noqa: BLE001
                    log.warning(
                        "spendguard.integrations.atomic_agents: "
                        "emit_llm_call_post failed on exception path "
                        "(run_id=%s sig=%s err=%r) — reservation will "
                        "TTL-sweep",
                        ctx.run_id, signature[:8], post_exc,
                    )
        finally:
            self._pending_attempt = None

    def _sync_post_success(self, raw_completion: Any) -> None:
        """POST commits with usage from the raw ChatCompletion.

        Instructor's ``create_fn`` returns the raw provider
        ``ChatCompletion``; ``process_response`` parses it AFTER.
        We commit before the parse step which is the correct gate
        ordering (the parse happens inside Instructor; a parse-fail
        triggers retry, and the next attempt is gated separately).
        """
        pending = getattr(self, "_pending_attempt", None)
        if not pending:
            return
        outcome = pending["outcome"]
        ctx = pending["ctx"]
        total_tokens = _extract_total_tokens(raw_completion)
        provider_event_id = _extract_provider_event_id(raw_completion)
        try:
            if outcome.reservation_ids:
                asyncio.run(
                    self._client.emit_llm_call_post(
                        run_id=ctx.run_id,
                        step_id=pending["step_id"],
                        llm_call_id=pending["llm_call_id"],
                        decision_id=outcome.decision_id,
                        reservation_id=outcome.reservation_ids[0],
                        provider_reported_amount_atomic="",
                        estimated_amount_atomic=str(total_tokens),
                        unit=self._unit,
                        pricing=self._pricing,
                        provider_event_id=provider_event_id,
                        outcome="SUCCESS",
                    )
                )
        finally:
            self._pending_attempt = None


class SpendGuardAsyncInstructorProxy(_ProxyBase):
    """Async Instructor proxy. Atomic Agents async path.

    Operator wraps ``instructor.from_openai(AsyncOpenAI(...))``; the
    factory dispatches here. Direct ``await`` on the sidecar — no
    ``asyncio.run`` bridging needed.

    Same DEVIATION-C / raw-method-intercept pattern as the sync
    proxy: ``retry_async`` calls the raw provider method per attempt;
    we wrap that raw method with an awaitable gated closure, then
    re-patch the inner Instructor.
    """

    def __init__(self, **kwargs: Any) -> None:
        super().__init__(**kwargs)
        self.chat = _ChatNamespace(_ChatCompletionsPassthrough(self))
        self._original_create_fn = self._inner.create_fn
        self._original_raw_create = SpendGuardInstructorProxy._resolve_raw_create(
            self._inner
        )
        gated_raw = self._make_gated_raw_create(self._original_raw_create)
        self._gated_raw_create = gated_raw
        try:
            import instructor as _instructor_mod
        except ImportError as exc:  # pragma: no cover
            raise SpendGuardConfigError(
                "instructor is not importable; cannot re-patch create_fn."
            ) from exc
        self._inner.create_fn = _instructor_mod.patch(
            create=gated_raw, mode=self._inner.mode
        )

    def _make_gated_raw_create(
        self, raw_create: Callable[..., Any]
    ) -> Callable[..., Any]:
        """Build an async gated wrapper around the raw provider create.

        For ``AsyncOpenAI`` / ``AsyncAnthropic`` the raw method is
        already a coroutine; we ``await`` it inside the gate.
        """
        proxy = self
        from functools import wraps

        @wraps(raw_create)
        async def _gated_raw_create(*args: Any, **kwargs: Any) -> Any:
            await proxy._async_gate_attempt(args=args, kwargs=kwargs)
            try:
                result = await raw_create(*args, **kwargs)
            except BaseException as exc:
                await proxy._async_post_failure(exc)
                raise
            await proxy._async_post_success(result)
            return result

        return _gated_raw_create

    async def _async_gate_attempt(
        self, *, args: tuple[Any, ...], kwargs: dict[str, Any]
    ) -> None:
        """Async PRE — same shape as sync, awaits the sidecar directly."""
        ctx = current_run_context()
        sig_kwargs = dict(kwargs)
        signature = _signature(sig_kwargs)
        llm_call_id, decision_id, step_id, idem_key = self._derive_ids(
            ctx, signature
        )
        projected_claims = self._claim_estimator(sig_kwargs)
        decision_context = self._build_decision_context()

        outcome: DecisionOutcome = await self._client.request_decision(
            trigger="LLM_CALL_PRE",
            run_id=ctx.run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            tool_call_id="",
            decision_id=decision_id,
            route=self._route,
            projected_claims=projected_claims,
            idempotency_key=idem_key,
            projected_unit=self._unit,
            decision_context_json=decision_context,
        )
        self._pending_attempt = {
            "ctx": ctx,
            "outcome": outcome,
            "llm_call_id": llm_call_id,
            "step_id": step_id,
            "signature": signature,
        }

    async def _async_post_failure(self, exc: BaseException) -> None:
        pending = getattr(self, "_pending_attempt", None)
        if not pending:
            return
        outcome = pending["outcome"]
        ctx = pending["ctx"]
        signature = pending["signature"]
        try:
            if outcome.reservation_ids:
                outcome_kind = _classify_exception(exc)
                try:
                    await self._client.emit_llm_call_post(
                        run_id=ctx.run_id,
                        step_id=pending["step_id"],
                        llm_call_id=pending["llm_call_id"],
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
                        "spendguard.integrations.atomic_agents: "
                        "emit_llm_call_post failed on exception path "
                        "(run_id=%s sig=%s err=%r) — reservation will "
                        "TTL-sweep",
                        ctx.run_id, signature[:8], post_exc,
                    )
        finally:
            self._pending_attempt = None

    async def _async_post_success(self, raw_completion: Any) -> None:
        pending = getattr(self, "_pending_attempt", None)
        if not pending:
            return
        outcome = pending["outcome"]
        ctx = pending["ctx"]
        total_tokens = _extract_total_tokens(raw_completion)
        provider_event_id = _extract_provider_event_id(raw_completion)
        try:
            if outcome.reservation_ids:
                await self._client.emit_llm_call_post(
                    run_id=ctx.run_id,
                    step_id=pending["step_id"],
                    llm_call_id=pending["llm_call_id"],
                    decision_id=outcome.decision_id,
                    reservation_id=outcome.reservation_ids[0],
                    provider_reported_amount_atomic="",
                    estimated_amount_atomic=str(total_tokens),
                    unit=self._unit,
                    pricing=self._pricing,
                    provider_event_id=provider_event_id,
                    outcome="SUCCESS",
                )
        finally:
            self._pending_attempt = None


# ─────────────────────────────────────────────────────────────────────
# Factory — sync ↔ async dispatch
# ─────────────────────────────────────────────────────────────────────


def wrap_instructor_client(
    client: Any,
    *,
    spendguard_client: SpendGuardClient,
    budget_id: str,
    window_instance_id: str,
    unit: Any,
    pricing: Any,
    claim_estimator: ClaimEstimator,
    route: str = "llm.call",
) -> "SpendGuardInstructorProxy | SpendGuardAsyncInstructorProxy":
    """Wrap an ``Instructor`` / ``AsyncInstructor`` for SpendGuard gating.

    Per review-standards §1.4:
      - Dispatches on ``isinstance(client, AsyncInstructor)`` FIRST
        (because ``AsyncInstructor`` is the more specific type), then
        ``Instructor``.
      - Reversing this order silently routes async clients to the
        sync proxy and is a Blocker.

    Per review-standards §1.1:
      - Accepts ONLY ``instructor.Instructor`` or
        ``instructor.AsyncInstructor``. Accepting a bare
        ``openai.OpenAI`` / ``anthropic.Anthropic`` is a Blocker —
        that path silently undercounts Instructor's validation
        retries (rejected alternative per design.md §1). The
        ``TypeError`` message points the operator at
        ``instructor.from_openai(...)``.

    Args:
        client: A live ``instructor.Instructor`` (sync) or
            ``instructor.AsyncInstructor`` (async). Owned by the
            caller; the proxy holds it by reference and does NOT
            close it.
        spendguard_client: A connected + handshook ``SpendGuardClient``.
        budget_id: Budget the reservation debits. REQUIRED.
        window_instance_id: Time-window scope on the budget. REQUIRED.
        unit: ``common_pb2.UnitRef`` describing the unit binding.
            REQUIRED — ``unit.unit_id`` must be non-empty.
        pricing: ``common_pb2.PricingFreeze`` for ledger lookup.
            REQUIRED.
        claim_estimator: ``(kwargs_dict) → list[BudgetClaim]``
            projector. REQUIRED — design.md §5 locks "No default
            ``claim_estimator``" because Instructor's polyglot routing
            makes any single default wrong.
        route: ``request_decision.route``. Defaults to ``"llm.call"``
            so dashboards group with the other framework integrations.

    Returns:
        ``SpendGuardInstructorProxy`` when the inner client is a sync
        ``instructor.Instructor``; ``SpendGuardAsyncInstructorProxy``
        when it's ``instructor.AsyncInstructor``.

    Raises:
        TypeError: when ``client`` is neither ``Instructor`` nor
            ``AsyncInstructor``. The error message points at
            ``instructor.from_openai(...)`` so the operator gets a
            clear path forward (review-standards §1.1).
        SpendGuardConfigError: from ``_ProxyBase.__init__`` when any
            required field is missing or invalid.
    """
    # When instructor isn't importable, _INSTRUCTOR_AVAILABLE is False
    # and _Instructor / _AsyncInstructor are None. Surface a helpful
    # error rather than failing later on the isinstance check.
    if not _INSTRUCTOR_AVAILABLE:  # pragma: no cover — covered by barrel guard
        raise ImportError(
            "spendguard.integrations.atomic_agents.wrap_instructor_client "
            "requires the [atomic-agents] extra (transitively the "
            "`instructor` package). "
            "Install with: pip install 'spendguard-sdk[atomic-agents]'"
        )

    # Order matters: AsyncInstructor before Instructor (the latter is
    # the more general type per Instructor's public API).
    if isinstance(client, _AsyncInstructor):
        return SpendGuardAsyncInstructorProxy(
            inner=client,
            spendguard_client=spendguard_client,
            budget_id=budget_id,
            window_instance_id=window_instance_id,
            unit=unit,
            pricing=pricing,
            claim_estimator=claim_estimator,
            route=route,
        )
    if isinstance(client, _Instructor):
        return SpendGuardInstructorProxy(
            inner=client,
            spendguard_client=spendguard_client,
            budget_id=budget_id,
            window_instance_id=window_instance_id,
            unit=unit,
            pricing=pricing,
            claim_estimator=claim_estimator,
            route=route,
        )
    # Reject raw provider SDKs explicitly. Per review-standards §1.1
    # the message MUST point at instructor.from_openai (and not just
    # bare "invalid client").
    raise TypeError(
        f"wrap_instructor_client expects instructor.Instructor or "
        f"instructor.AsyncInstructor; got {type(client).__name__}. "
        f"If you have a raw provider client (e.g. openai.OpenAI() or "
        f"anthropic.Anthropic()), wrap it first via "
        f"instructor.from_openai(client) / instructor.from_anthropic("
        f"client) so SpendGuard intercepts Instructor's validation-"
        f"retry loop (see docs/integrations/atomic-agents)."
    )


__all__ = [
    "ClaimEstimator",
    "RunContext",
    "SpendGuardAsyncInstructorProxy",
    "SpendGuardInstructorProxy",
    "_SyncInAsyncContext",
    "_classify_exception",
    "_extract_provider_event_id",
    "_extract_total_tokens",
    "_guard_async_context",
    "_signature",
    "_unpack_create_result",
    "current_run_context",
    "run_context",
    "wrap_instructor_client",
]
