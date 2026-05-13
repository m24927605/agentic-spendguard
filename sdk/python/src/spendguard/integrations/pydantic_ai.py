"""Pydantic-AI Model wrapper that gates each LLM call through the sidecar.

L3 capability per Sidecar Architecture §3.3 — every llm_call.pre trigger
boundary runs through the sidecar's 8-stage decision transaction, and
every llm_call.post emits a typed LlmCallPostPayload that drives the
ledger's commit_or_release lifecycle.

Integration shape:

    inner = OpenAIModel("gpt-4o-mini")     # any pydantic_ai.models.Model
    client = SpendGuardClient(socket_path=..., tenant_id=...)
    await client.handshake()

    guarded = SpendGuardModel(
        inner=inner,
        client=client,
        budget_id="...",
        window_instance_id="...",
        unit=UnitRef(unit_id="...", token_kind="output_token", model_family="gpt-4"),
        pricing=PricingFreeze(...),
        claim_estimator=my_estimator,    # messages -> projected claims
    )
    agent = pydantic_ai.Agent(model=guarded)
    async with run_context(RunContext(run_id=...)):
        await agent.run("Hello")

The wrapper does NOT manage Pydantic-AI's `UsageLimits` directly —
those remain a belt-and-suspenders fallback the caller can keep in
parallel. The sidecar is the source of truth for budget enforcement.

Idempotency model (per Trace §3.4):
  Pydantic-AI's Agent run loop calls `Model.request()` on each step,
  AND will re-enter `request()` with the same inputs on transient
  provider error. To make a retry collapse onto the original sidecar
  decision (rather than spawn a second reservation), we derive the
  step_id, llm_call_id, decision-trace-id, and idempotency_key from a
  hash of the messages + model_settings + run_id (see
  ids.default_call_signature). Two calls with bit-identical inputs
  produce identical identifiers; the sidecar cache + ledger UNIQUE
  collapse the retry into the original.

POC-deferred:
  - Tool call gating (TOOL_CALL_PRE) is not wired here — wrap the
    Pydantic-AI tool decorator separately.
  - Sub-agent budget grants are out of scope (Contract §8 surface).
  - DEGRADE mutation patches are surfaced as APPLY_FAILED rather than
    applied; RFC 6902 patch translation across pydantic-ai message
    structures is left to follow-on work.
"""

from __future__ import annotations

import asyncio
import contextvars
from collections.abc import AsyncIterator, Callable, Sequence
from contextlib import asynccontextmanager
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any

try:
    from spendguard._proto.spendguard.common.v1 import common_pb2
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard proto stubs missing. Run `make proto` first."
    ) from exc

from ..client import DecisionOutcome, SpendGuardClient
from ..errors import (
    DecisionDenied,
    DecisionSkipped,
    MutationApplyFailed,
    SpendGuardError,
)
from ..ids import default_call_signature, derive_idempotency_key, derive_uuid_from_signature

# Pydantic-AI's Agent.__init__ does an isinstance(model, Model) check via
# `models.infer_model`; a duck-typed wrapper falls through and pydantic-ai
# tries to .split(':') the value. So we MUST inherit from the real Model
# base class — making `pydantic_ai` a hard runtime dep of this module.
# (The proto/UDS client modules are unaffected and still importable
# without pydantic-ai installed.)
try:
    from pydantic_ai.models import Model as _PydanticAIModel
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.pydantic_ai requires the [pydantic-ai] "
        "extra. Install with: pip install 'spendguard-sdk[pydantic-ai]'"
    ) from exc

if TYPE_CHECKING:
    from pydantic_ai.messages import ModelMessage, ModelResponse
    from pydantic_ai.models import ModelRequestParameters
    from pydantic_ai.settings import ModelSettings


# -------------------------------------------------------------------------
# Run-scoped context (set by the caller for each Agent.run() invocation).
# -------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class RunContext:
    """Per-Agent-run identifiers.

    `run_id` MUST be stable for the duration of a single Agent.run() —
    if it changes, retries within that run won't collapse onto the
    original idempotency key. Callers typically mint it once with
    `new_uuid7()` and bind via `run_context()`.
    """

    run_id: str
    parent_run_id: str = ""
    budget_grant_jti: str = ""
    route: str = "llm.call"
    traceparent: str = ""
    tracestate: str = ""


_RUN_CONTEXT: contextvars.ContextVar[RunContext | None] = contextvars.ContextVar(
    "spendguard_run_context", default=None
)


@asynccontextmanager
async def run_context(ctx: RunContext) -> AsyncIterator[RunContext]:
    """Bind a RunContext for the duration of a Pydantic-AI Agent.run().

    Usage:

        async with run_context(RunContext(run_id="...")):
            await agent.run("Hello")

    SpendGuardModel.request() reads the active context to attach
    run/step ids to its sidecar calls. Without this binding, the
    wrapper fails closed (raises SpendGuardError).

    `ContextVar` is asyncio-task-safe: child tasks spawned by `await`
    inherit the binding; `asyncio.gather` / `asyncio.TaskGroup` siblings
    each see the parent binding at spawn time. Concurrent Agent.run()
    invocations in the same event loop should each be wrapped in their
    own `run_context()` block — they will not leak across binding.
    """
    token = _RUN_CONTEXT.set(ctx)
    try:
        yield ctx
    finally:
        _RUN_CONTEXT.reset(token)


def current_run_context() -> RunContext:
    ctx = _RUN_CONTEXT.get()
    if ctx is None:
        raise SpendGuardError(
            "no SpendGuard RunContext bound; wrap Agent.run() in "
            "`async with run_context(RunContext(run_id=...)):`"
        )
    return ctx


# -------------------------------------------------------------------------
# Claim estimator + call-signature interfaces
# -------------------------------------------------------------------------


ClaimEstimator = Callable[
    ["Sequence[ModelMessage]", "ModelSettings | None"],
    list[common_pb2.BudgetClaim],
]
"""Function signature for projecting BudgetClaims from messages.

Called BEFORE each LLM call. Estimators may use a tokenizer
(tiktoken / anthropic-tokenizer) or a chars/4 heuristic. Returning an
empty list is allowed for non-budgeted models (the sidecar will
short-circuit to CONTINUE on policy that requires no claim).
"""


CallSignatureFn = Callable[["Sequence[ModelMessage]", "ModelSettings | None"], str]
"""Function signature for a custom call-content hash.

The default (`ids.default_call_signature`) hashes pydantic
`model_dump_json` bytes when available. Callers needing version-stable
or cross-runtime portable signatures should provide their own.
"""


def _flatten_messages_to_prompt(messages: "Sequence[ModelMessage]") -> str:
    """Cost Advisor P0.5: flatten Pydantic-AI messages into one prompt
    string for HMAC-SHA256 fingerprinting.

    Uses pydantic v2 ``model_dump_json(exclude_none=True)`` when
    available so two semantically-identical message sequences produce
    byte-equal output. Falls back to ``repr()`` for non-pydantic items.

    The sidecar's ``prompt_hash`` is computed over the result; rules
    dedupe retried LLM calls by ``(run_id, prompt_hash)``. Two calls
    with bit-identical message bodies produce the same hash — which is
    what makes ``failed_retry_burn_v1`` etc. work.
    """
    parts = []
    for msg in messages:
        if hasattr(msg, "model_dump_json"):
            try:
                parts.append(msg.model_dump_json(exclude_none=True))
                continue
            except Exception:  # noqa: BLE001
                pass
        parts.append(repr(msg))
    return "\n".join(parts)


# -------------------------------------------------------------------------
# SpendGuardModel
# -------------------------------------------------------------------------


@dataclass(slots=True)
class _CallIdentity:
    """Stable identity for one logical LLM call (survives retries)."""

    signature: str            # 32-char hex blake2b digest
    step_id: str              # f"{run_id}:call:{signature[:16]}"
    llm_call_id: str          # UUID derived from signature
    trace_decision_id: str    # UUID for SpendGuardIds.decision_id (trace anchor)
    idempotency_key: str


class SpendGuardModel(_PydanticAIModel):
    """Pydantic-AI Model that runs each request through the sidecar.

    Implements the duck-typed `pydantic_ai.models.Model` interface
    (`request`, `request_stream`, `model_name`, `system`) by delegating
    to an inner Model and bracketing the call with sidecar IPC.
    """

    def __init__(
        self,
        *,
        inner: "Model",
        client: SpendGuardClient,
        budget_id: str,
        window_instance_id: str,
        unit: common_pb2.UnitRef,
        pricing: common_pb2.PricingFreeze,
        claim_estimator: ClaimEstimator,
        call_signature_fn: CallSignatureFn | None = None,
        provider_event_id_extractor: Callable[["ModelResponse"], str] | None = None,
    ) -> None:
        if not budget_id:
            raise ValueError("budget_id is required")
        if not window_instance_id:
            raise ValueError("window_instance_id is required")

        self._inner = inner
        self._client = client
        self._budget_id = budget_id
        self._window_instance_id = window_instance_id
        self._unit = unit
        self._pricing = pricing
        self._claim_estimator = claim_estimator
        self._call_signature_fn = call_signature_fn or default_call_signature
        self._provider_event_id_extractor = provider_event_id_extractor or (lambda _r: "")

    # -- Pydantic-AI Model surface ----------------------------------------

    @property
    def model_name(self) -> str:
        return getattr(self._inner, "model_name", "spendguard")

    @property
    def system(self) -> str:
        return getattr(self._inner, "system", "spendguard")

    @property
    def base_url(self) -> str | None:
        return getattr(self._inner, "base_url", None)

    @property
    def profile(self) -> Any:
        return getattr(self._inner, "profile", None)

    def customize_request_parameters(
        self, model_request_parameters: "ModelRequestParameters"
    ) -> "ModelRequestParameters":
        """Forward newer base-Model hooks to the inner model.

        Pydantic-AI's base Model has a few helpers that aren't part of
        the historical contract; wrapper passes them through so the
        Agent run loop sees inner-model behaviour unchanged.
        """
        inner_fn = getattr(self._inner, "customize_request_parameters", None)
        if inner_fn is None:
            return model_request_parameters
        return inner_fn(model_request_parameters)

    async def request(
        self,
        messages: "Sequence[ModelMessage]",
        model_settings: "ModelSettings | None",
        model_request_parameters: "ModelRequestParameters",
        run_context: Any | None = None,
    ) -> Any:
        """Gated request: sidecar Decision → inner.request() → publish outcome.

        Returns the tuple Pydantic-AI's Agent expects:
        `(ModelResponse, Usage)`. We mirror the inner model's return
        shape rather than synthesizing it so usage accounting (the
        adapter is NOT the source of truth for usage) flows from the
        provider through to the framework's `UsageLimits` machinery.

        `run_context` is Pydantic-AI's per-run context (distinct from
        the spendguard `RunContext`). We forward it untouched to the
        inner model so any pydantic-ai-internal state reaches the
        underlying provider.
        """
        ctx = current_run_context()
        identity = self._derive_call_identity(ctx, messages, model_settings)

        outcome = await self._call_request_decision(
            ctx, identity, messages, model_settings
        )

        confirmed = False
        try:
            applied_messages, applied_settings, mutation_applied = (
                self._apply_mutation_if_any(outcome, messages, model_settings)
            )
            inner_result = await self._call_inner_request(
                applied_messages,
                applied_settings,
                model_request_parameters,
                run_context,
            )
            # Modern pydantic-ai (>=0.0.40) returns `(ModelResponse, Usage)`;
            # older versions returned just `ModelResponse`. Tolerate both
            # shapes so the wrapper composes regardless of inner provider.
            if isinstance(inner_result, tuple) and len(inner_result) == 2:
                response, usage = inner_result
            else:
                response = inner_result
                usage = None
            await self._emit_post_and_confirm(
                outcome=outcome,
                identity=identity,
                response=response,
                usage=usage,
                ctx=ctx,
                mutation_applied=mutation_applied,
            )
            confirmed = True
            return (response, usage) if usage is not None else response
        except asyncio.CancelledError:
            # Drain / shutdown: don't try to anchor — the sidecar is
            # also draining and the audit chain will reconcile via TTL.
            raise
        except BaseException as e:
            if not confirmed:
                await self._client.safe_confirm_apply_failed(
                    decision_id=outcome.decision_id,
                    effect_hash=outcome.effect_hash,
                    adapter_error=f"{type(e).__name__}: {e}",
                )
            raise

    @asynccontextmanager
    async def request_stream(
        self,
        messages: "Sequence[ModelMessage]",
        model_settings: "ModelSettings | None",
        model_request_parameters: "ModelRequestParameters",
        run_context: Any | None = None,
    ) -> AsyncIterator[Any]:
        """Streaming request: sidecar gates the call boundary; chunks pass through.

        The sidecar runs RequestDecision before the stream begins.
        Chunks flow through unchanged — per-token rate-shaping is out
        of scope for L3. When the stream completes (or fails), the
        wrapper emits LLM_CALL_POST with the final usage and confirms
        publish_outcome.

        Pydantic-AI's `request_stream` is an async context manager that
        yields a `StreamedResponse`; current Pydantic-AI versions also
        accept an optional `run_context` for inner provider state. The
        wrapper forwards it untouched.
        """
        ctx = current_run_context()
        identity = self._derive_call_identity(ctx, messages, model_settings)
        outcome = await self._call_request_decision(
            ctx, identity, messages, model_settings
        )

        confirmed = False
        applied_messages, applied_settings = (messages, model_settings)
        mutation_applied = False
        try:
            applied_messages, applied_settings, mutation_applied = (
                self._apply_mutation_if_any(outcome, messages, model_settings)
            )
            inner_cm = self._open_inner_stream(
                applied_messages,
                applied_settings,
                model_request_parameters,
                run_context,
            )
            async with inner_cm as stream:
                yield stream
            # Stream path: usage is exposed on the StreamedResponse via
            # `.usage()` after the stream completes. _extract_provider_amount
            # falls back to that when `usage` arg is None.
            await self._emit_post_and_confirm(
                outcome=outcome,
                identity=identity,
                response=stream,
                usage=None,
                ctx=ctx,
                mutation_applied=mutation_applied,
            )
            confirmed = True
        except asyncio.CancelledError:
            raise
        except BaseException as e:
            if not confirmed:
                await self._client.safe_confirm_apply_failed(
                    decision_id=outcome.decision_id,
                    effect_hash=outcome.effect_hash,
                    adapter_error=f"{type(e).__name__}: {e}",
                )
            raise

    # -- Internals --------------------------------------------------------

    def _derive_call_identity(
        self,
        ctx: RunContext,
        messages: "Sequence[ModelMessage]",
        model_settings: "ModelSettings | None",
    ) -> _CallIdentity:
        signature = self._call_signature_fn(messages, model_settings)
        step_id = f"{ctx.run_id}:call:{signature[:16]}"
        llm_call_uuid = derive_uuid_from_signature(signature, scope="llm_call_id")
        trace_decision_uuid = derive_uuid_from_signature(
            signature, scope="trace_decision_id"
        )
        idempotency_key = derive_idempotency_key(
            tenant_id=self._client.tenant_id,
            session_id=self._client.session_id,
            run_id=ctx.run_id,
            step_id=step_id,
            llm_call_id=str(llm_call_uuid),
            trigger="LLM_CALL_PRE",
        )
        return _CallIdentity(
            signature=signature,
            step_id=step_id,
            llm_call_id=str(llm_call_uuid),
            trace_decision_id=str(trace_decision_uuid),
            idempotency_key=idempotency_key,
        )

    async def _call_inner_request(
        self,
        messages: "Sequence[ModelMessage]",
        model_settings: "ModelSettings | None",
        model_request_parameters: "ModelRequestParameters",
        run_context: Any | None,
    ) -> "ModelResponse":
        """Call inner.request, forwarding run_context if the inner accepts it.

        Newer Pydantic-AI versions added an optional `run_context` kwarg
        on `Model.request`; older ones don't have it. We try the
        kwargful form first and fall back to the legacy 3-arg signature
        on TypeError — that way the wrapper composes with any base-Model
        implementation without forcing a hard version pin.
        """
        if run_context is None:
            return await self._inner.request(
                messages, model_settings, model_request_parameters
            )
        try:
            return await self._inner.request(
                messages,
                model_settings,
                model_request_parameters,
                run_context=run_context,
            )
        except TypeError:
            return await self._inner.request(
                messages, model_settings, model_request_parameters
            )

    def _open_inner_stream(
        self,
        messages: "Sequence[ModelMessage]",
        model_settings: "ModelSettings | None",
        model_request_parameters: "ModelRequestParameters",
        run_context: Any | None,
    ) -> Any:
        """Open inner.request_stream, forwarding run_context if accepted.

        Same compatibility shape as `_call_inner_request` — newer
        Pydantic-AI accepts `run_context`, older does not. Returns the
        inner async context manager unchanged for the caller to
        `async with`.
        """
        if run_context is None:
            return self._inner.request_stream(
                messages, model_settings, model_request_parameters
            )
        try:
            return self._inner.request_stream(
                messages,
                model_settings,
                model_request_parameters,
                run_context=run_context,
            )
        except TypeError:
            return self._inner.request_stream(
                messages, model_settings, model_request_parameters
            )

    async def _call_request_decision(
        self,
        ctx: RunContext,
        identity: _CallIdentity,
        messages: "Sequence[ModelMessage]",
        model_settings: "ModelSettings | None",
    ) -> DecisionOutcome:
        projected_claims = self._claim_estimator(messages, model_settings)
        # Cost Advisor P0.5: flatten messages into canonical prompt text
        # so the sidecar can emit prompt_hash on the audit.decision
        # CloudEvent. Used for run-scope dedup in cost_advisor rules
        # (failed_retry_burn_v1, runaway_loop_v1 per spec §5.1).
        prompt_text = _flatten_messages_to_prompt(messages)
        return await self._client.request_decision(
            trigger="LLM_CALL_PRE",
            run_id=ctx.run_id,
            step_id=identity.step_id,
            llm_call_id=identity.llm_call_id,
            tool_call_id="",
            decision_id=identity.trace_decision_id,
            route=ctx.route,
            projected_claims=projected_claims,
            idempotency_key=identity.idempotency_key,
            traceparent=ctx.traceparent,
            tracestate=ctx.tracestate,
            parent_run_id=ctx.parent_run_id,
            budget_grant_jti=ctx.budget_grant_jti,
            projected_unit=self._unit,
            prompt_text=prompt_text,
        )

    def _apply_mutation_if_any(
        self,
        outcome: DecisionOutcome,
        messages: "Sequence[ModelMessage]",
        model_settings: "ModelSettings | None",
    ) -> tuple["Sequence[ModelMessage]", "ModelSettings | None", bool]:
        """Returns (messages, settings, mutation_actually_applied).

        The trailing bool drives PublishOutcomeRequest.outcome on the
        confirm step:
          - True  → APPLIED       (a runtime mutation was applied)
          - False → APPLIED_NOOP  (CONTINUE, or DEGRADE with empty patch)
        DEGRADE with a non-empty patch raises MutationApplyFailed in
        POC (the outer try/except then confirms APPLY_FAILED).
        """
        if outcome.decision != "DEGRADE":
            return messages, model_settings, False
        if not outcome.mutation_patch_json:
            # DEGRADE with no patch — sidecar wants the audit chain to
            # record a degrade event but no runtime change happened.
            # Pass through; confirm will be APPLIED_NOOP.
            return messages, model_settings, False
        # POC: do NOT attempt to apply RFC 6902 patches against
        # pydantic-ai message structures (paths reference provider-
        # specific fields). Raise MutationApplyFailed; the outer
        # try/except in request()/request_stream() then confirms
        # publish_outcome=APPLY_FAILED so the audit chain still has a
        # terminal anchor.
        raise MutationApplyFailed(
            f"DEGRADE mutation_patch_json received but adapter does not yet "
            f"apply RFC 6902 patches; decision_id={outcome.decision_id}"
        )

    async def _emit_post_and_confirm(
        self,
        *,
        outcome: DecisionOutcome,
        identity: _CallIdentity,
        response: Any,
        usage: Any = None,
        ctx: RunContext,
        mutation_applied: bool,
    ) -> None:
        if not outcome.reservation_ids:
            # Sidecar approved with no reservation — e.g., trigger
            # boundary that the contract chose not to charge. Skip the
            # post-event payload and just confirm publish.
            await self._client.confirm_publish_outcome(
                decision_id=outcome.decision_id,
                effect_hash=outcome.effect_hash,
                outcome="APPLIED" if mutation_applied else "APPLIED_NOOP",
            )
            return

        reservation_id = outcome.reservation_ids[0]
        provider_amount = self._extract_provider_amount(response, usage)
        provider_event_id = self._provider_event_id_extractor(response)
        post_outcome = "SUCCESS"

        # Phase 2B Step 7: route MockLLM/SDK-provided usage through the
        # CommitEstimated path (estimated_amount_atomic). ProviderReport
        # routing is deferred to Step 8; the sidecar will return a typed
        # UNIMPLEMENTED Error if `provider_reported_amount_atomic` is sent.
        await self._client.emit_llm_call_post(
            run_id=ctx.run_id,
            step_id=identity.step_id,
            llm_call_id=identity.llm_call_id,
            decision_id=outcome.decision_id,
            reservation_id=reservation_id,
            provider_reported_amount_atomic="",
            estimated_amount_atomic=provider_amount,
            unit=self._unit,
            pricing=self._pricing,
            provider_event_id=provider_event_id,
            outcome=post_outcome,
            traceparent=ctx.traceparent,
            tracestate=ctx.tracestate,
        )
        await self._client.confirm_publish_outcome(
            decision_id=outcome.decision_id,
            effect_hash=outcome.effect_hash,
            outcome="APPLIED" if mutation_applied else "APPLIED_NOOP",
        )

    @staticmethod
    def _extract_provider_amount(response: Any, usage: Any = None) -> str:
        """Extract atomic spend from response or explicit usage arg.

        Modern Pydantic-AI's `Model.request` returns `(ModelResponse, Usage)`
        as a tuple; `Usage` is NOT exposed on `ModelResponse`. The wrapper
        passes `usage` directly via the `usage` argument so we don't have
        to introspect the response.

        Streaming path passes `usage=None` and we fall back to
        `response.usage()` (StreamedResponse exposes a callable usage).

        Returns "0" only as a defensive fallback when neither path yields
        an integer total.
        """
        if usage is None:
            usage_fn = getattr(response, "usage", None)
            if usage_fn is None:
                return "0"
            try:
                usage = usage_fn() if callable(usage_fn) else usage_fn
            except Exception:  # noqa: BLE001 — mid-stream usage may not be available
                return "0"
        if usage is None:
            return "0"
        total = getattr(usage, "total_tokens", None)
        if total is None:
            return "0"
        return str(int(total))


# -------------------------------------------------------------------------
# Convenience: capture exceptions raised by sidecar Decision back to caller.
# -------------------------------------------------------------------------


def is_spendguard_terminal(exc: BaseException) -> bool:
    """True if exc was raised by sidecar Decision and should terminate the run.

    Useful for higher-level frameworks that want to distinguish a budget
    enforcement decision from a model-provider transient error.

    `DecisionStopped` and `ApprovalRequired` are subclasses of
    `DecisionDenied` so the single base check covers all denied
    outcomes; `DecisionSkipped` is intentionally excluded because SKIP
    is non-fatal (caller may catch and continue).
    """
    return isinstance(exc, DecisionDenied) and not isinstance(exc, DecisionSkipped)


def is_spendguard_skip(exc: BaseException) -> bool:
    """True if the sidecar SKIP'd this trigger boundary (non-fatal)."""
    return isinstance(exc, DecisionSkipped)
