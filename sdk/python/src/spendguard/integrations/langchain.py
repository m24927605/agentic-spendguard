"""LangChain (and LangGraph) integration — gates BaseChatModel via the sidecar.

Wrap any LangChain `BaseChatModel` (ChatOpenAI, ChatAnthropic, etc.) in
`SpendGuardChatModel` to route every `_agenerate` / `_generate` call
through the SpendGuard sidecar's RequestDecision → CommitEstimated
lifecycle. Works transparently with LangGraph because LangGraph
operates on LangChain's `BaseChatModel` interface.

Integration shape::

    from langchain_openai import ChatOpenAI
    from spendguard import SpendGuardClient, new_uuid7
    from spendguard.integrations.langchain import SpendGuardChatModel
    from spendguard._proto.spendguard.common.v1 import common_pb2

    client = SpendGuardClient(socket_path=..., tenant_id=...)
    await client.connect()
    await client.handshake()

    guarded = SpendGuardChatModel(
        inner=ChatOpenAI(model="gpt-4o-mini"),
        client=client,
        budget_id="...",
        window_instance_id="...",
        unit=common_pb2.UnitRef(unit_id="...", token_kind="output_token", model_family="gpt-4"),
        pricing=common_pb2.PricingFreeze(...),
        claim_estimator=lambda messages: [common_pb2.BudgetClaim(...)],
    )

    # Use anywhere a ChatOpenAI / BaseChatModel works:
    response = await guarded.ainvoke([HumanMessage(content="Hello")])

    # Or build a LangGraph state machine on top:
    from langgraph.prebuilt import create_react_agent
    agent = create_react_agent(guarded, tools=[...])
    await agent.ainvoke({"messages": [...]})

POC scope:
  - Streaming (`_astream`) is gated at PRE only; POST emits after
    final chunk. Tool-call mid-stream isn't separately gated yet.
  - DEGRADE mutation patch is surfaced as APPLY_FAILED rather than
    applied (parity with pydantic-ai integration).
  - Idempotency uses (run_id, message_count) as the call signature;
    callers wanting deterministic key derivation across retries
    should pass `call_signature_fn=...`.
"""

from __future__ import annotations

import contextvars
from collections.abc import Callable, Sequence
from contextlib import asynccontextmanager
from dataclasses import dataclass
from typing import Any, AsyncIterator

from ..client import DecisionOutcome, SpendGuardClient
from ..errors import DecisionDenied, DecisionSkipped
import hashlib

from ..ids import (
    derive_idempotency_key,
    derive_uuid_from_signature,
    new_uuid7,
)

try:
    from langchain_core.language_models import BaseChatModel
    from langchain_core.messages import AIMessage, BaseMessage
    from langchain_core.outputs import ChatGeneration, ChatResult
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.langchain requires the [langchain] extra. "
        "Install with: pip install 'spendguard-sdk[langchain]'"
    ) from exc

try:
    from spendguard._proto.spendguard.common.v1 import common_pb2
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard proto stubs missing. Run `make proto` first."
    ) from exc


# Run-scoped context (mirrors pydantic_ai's contextvar so users can
# bind once and have both adapters pick up the same run_id).
_RUN_CONTEXT: contextvars.ContextVar["RunContext | None"] = contextvars.ContextVar(
    "spendguard_run_context", default=None
)


@dataclass(frozen=True, slots=True)
class RunContext:
    """Per-LangChain-invocation identifiers."""

    run_id: str


@asynccontextmanager
async def run_context(ctx: RunContext) -> AsyncIterator[RunContext]:
    token = _RUN_CONTEXT.set(ctx)
    try:
        yield ctx
    finally:
        _RUN_CONTEXT.reset(token)


def current_run_context() -> RunContext:
    ctx = _RUN_CONTEXT.get()
    if ctx is None:
        raise RuntimeError(
            "spendguard.integrations.langchain.SpendGuardChatModel called "
            "outside an active run_context(). Wrap your invocation:\n\n"
            "    async with run_context(RunContext(run_id=...)):\n"
            "        await guarded.ainvoke(messages)\n"
        )
    return ctx


# Claim estimator type: receives the messages list, returns claims.
ClaimEstimator = Callable[[Sequence[BaseMessage]], list[Any]]
"""Project BudgetClaim list from incoming messages.

A simple chars/4 heuristic estimator looks like::

    def estimate(messages):
        chars = sum(len(getattr(m, "content", "")) for m in messages)
        projected_tokens = max(50, chars // 4)
        return [common_pb2.BudgetClaim(
            budget_id=BUDGET_ID,
            unit=UNIT_REF,
            amount_atomic=str(projected_tokens),
            direction=common_pb2.BudgetClaim.DEBIT,
            window_instance_id=WINDOW_ID,
        )]
"""

CallSignatureFn = Callable[[Sequence[BaseMessage]], str]
"""Custom call-content signature; defaults to a hash over message content."""


def _default_call_signature(messages: Sequence[BaseMessage]) -> str:
    """Hash the visible message content into a 32-char hex digest.

    LangChain's `BaseMessage.content` can be str or list[dict]; we
    coerce via __str__. blake2b matches pydantic_ai signature width
    (32 hex chars) so ID derivation downstream is symmetric.
    """
    payload = "\n".join(f"{type(m).__name__}:{m.content!s}" for m in messages)
    return hashlib.blake2b(payload.encode("utf-8"), digest_size=16).hexdigest()


class SpendGuardChatModel(BaseChatModel):
    """LangChain BaseChatModel wrapper that gates each call through the sidecar.

    The class inherits directly from BaseChatModel rather than wrapping
    via composition; LangChain's runnable protocol checks isinstance,
    not duck typing.

    All Pydantic-validation arbitrary-types are allowed because we hold
    a `SpendGuardClient` (gRPC channel) and proto messages — neither is
    Pydantic-friendly.
    """

    # Pydantic v2 config: allow non-Pydantic types in fields.
    model_config = {"arbitrary_types_allowed": True}

    # `inner` accepts BaseChatModel OR a RunnableBinding produced by
    # bind_tools(). Type as Any here because LangChain's Runnable
    # tree isn't Pydantic-friendly; runtime checks below validate.
    inner: Any
    client: SpendGuardClient
    budget_id: str
    window_instance_id: str
    unit: Any  # common_pb2.UnitRef
    pricing: Any  # common_pb2.PricingFreeze
    claim_estimator: ClaimEstimator
    call_signature_fn: CallSignatureFn | None = None

    @property
    def _llm_type(self) -> str:
        return f"spendguard:{self.inner._llm_type}"

    # -- LangChain integrations (LangGraph create_react_agent uses these) ----

    def bind_tools(self, tools: Any, **kwargs: Any) -> "SpendGuardChatModel":
        """Forward tool binding to the inner model + re-wrap.

        LangGraph's `create_react_agent` calls bind_tools on the model
        to attach the tool schema. The base BaseChatModel raises
        NotImplementedError; we delegate to the inner model and wrap
        the result back in SpendGuardChatModel so gating is preserved.
        """
        bound_inner = self.inner.bind_tools(tools, **kwargs)  # type: ignore[attr-defined]
        return SpendGuardChatModel(
            inner=bound_inner,
            client=self.client,
            budget_id=self.budget_id,
            window_instance_id=self.window_instance_id,
            unit=self.unit,
            pricing=self.pricing,
            claim_estimator=self.claim_estimator,
            call_signature_fn=self.call_signature_fn,
        )

    def with_structured_output(self, *args: Any, **kwargs: Any) -> Any:
        """Forward structured-output binding to the inner model."""
        return self.inner.with_structured_output(*args, **kwargs)  # type: ignore[attr-defined]

    # -- BaseChatModel surface ------------------------------------------------

    def _generate(
        self,
        messages: list[BaseMessage],
        stop: list[str] | None = None,
        run_manager: Any = None,
        **kwargs: Any,
    ) -> ChatResult:
        # LangChain's sync path is rare in agent code (most agents call
        # _agenerate). Block on async via the running loop if available;
        # otherwise raise a clear error pointing the caller at ainvoke.
        import asyncio

        try:
            loop = asyncio.get_event_loop()
            if loop.is_running():
                raise RuntimeError(
                    "SpendGuardChatModel._generate (sync) called inside a running "
                    "event loop. Use `await guarded.ainvoke(...)` instead."
                )
        except RuntimeError:
            loop = asyncio.new_event_loop()
        return loop.run_until_complete(
            self._agenerate(messages, stop, run_manager, **kwargs)
        )

    async def _agenerate(
        self,
        messages: list[BaseMessage],
        stop: list[str] | None = None,
        run_manager: Any = None,
        **kwargs: Any,
    ) -> ChatResult:
        ctx = current_run_context()
        sig_fn = self.call_signature_fn or _default_call_signature
        signature = sig_fn(messages)

        # Stable identifiers per LLM call within this run.
        llm_call_id = str(derive_uuid_from_signature(signature, scope="llm_call_id"))
        decision_id = str(derive_uuid_from_signature(signature, scope="decision_id"))
        step_id = f"{ctx.run_id}:lc-call:{signature[:16]}"
        idempotency_key = derive_idempotency_key(
            tenant_id=self.client.tenant_id,
            session_id=self.client.session_id,
            run_id=ctx.run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )

        # 1) PRE — request_decision (raises DecisionStopped on STOP, etc.)
        outcome: DecisionOutcome = await self.client.request_decision(
            trigger="LLM_CALL_PRE",
            run_id=ctx.run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            tool_call_id="",
            decision_id=decision_id,
            route="llm.call",
            projected_claims=self.claim_estimator(messages),
            idempotency_key=idempotency_key,
        )

        # 2) Inner LangChain model call.
        inner_result: ChatResult = await self.inner._agenerate(
            messages, stop=stop, run_manager=run_manager, **kwargs
        )

        # 3) Extract usage from response. LangChain conventions:
        #    AIMessage.usage_metadata has total_tokens (LangChain ≥0.3),
        #    or response_metadata['token_usage']['total_tokens'] (older).
        total_tokens = self._extract_total_tokens(inner_result)
        provider_event_id = self._extract_provider_event_id(inner_result)

        # 4) POST — emit_llm_call_post with real usage (drives ledger commit).
        if outcome.reservation_ids:
            reservation_id = outcome.reservation_ids[0]
            await self.client.emit_llm_call_post(
                run_id=ctx.run_id,
                step_id=step_id,
                llm_call_id=llm_call_id,
                decision_id=outcome.decision_id,
                reservation_id=reservation_id,
                provider_reported_amount_atomic="",
                estimated_amount_atomic=str(total_tokens),
                unit=self.unit,
                pricing=self.pricing,
                provider_event_id=provider_event_id,
                outcome="SUCCESS",
            )

        return inner_result

    @staticmethod
    def _extract_total_tokens(result: ChatResult) -> int:
        if not result.generations:
            return 0
        gen = result.generations[0]
        msg = gen.message if isinstance(gen, ChatGeneration) else None
        if isinstance(msg, AIMessage):
            usage = getattr(msg, "usage_metadata", None) or {}
            if isinstance(usage, dict) and "total_tokens" in usage:
                return int(usage["total_tokens"])
            # Fallback: response_metadata.token_usage (older convention)
            md = getattr(msg, "response_metadata", None) or {}
            tu = md.get("token_usage") if isinstance(md, dict) else None
            if isinstance(tu, dict) and "total_tokens" in tu:
                return int(tu["total_tokens"])
        return 0

    @staticmethod
    def _extract_provider_event_id(result: ChatResult) -> str:
        if not result.generations:
            return ""
        gen = result.generations[0]
        msg = gen.message if isinstance(gen, ChatGeneration) else None
        if isinstance(msg, AIMessage):
            md = getattr(msg, "response_metadata", None) or {}
            if isinstance(md, dict):
                rid = md.get("id") or md.get("response_id")
                if isinstance(rid, str):
                    return rid
        return ""


__all__ = [
    "ClaimEstimator",
    "CallSignatureFn",
    "RunContext",
    "SpendGuardChatModel",
    "current_run_context",
    "run_context",
]
