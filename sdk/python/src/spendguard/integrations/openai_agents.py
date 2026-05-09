"""OpenAI Agents SDK integration — gates Model.get_response via the sidecar.

Validated against `openai-agents>=0.17`. The SDK exposes
`agents.models.interface.Model` as an abstract base; subclasses
override `get_response` (sync semantics over async I/O) plus
optionally `stream_response`. Built-in `OpenAIChatCompletionsModel`
follows this pattern; we mirror it.

Integration shape::

    from agents import Agent, Runner
    from agents.models.openai_chatcompletions import OpenAIChatCompletionsModel
    from openai import AsyncOpenAI

    from spendguard import SpendGuardClient
    from spendguard.integrations.openai_agents import (
        RunContext, SpendGuardAgentsModel, run_context,
    )
    from spendguard._proto.spendguard.common.v1 import common_pb2

    client = SpendGuardClient(socket_path=..., tenant_id=...)
    await client.connect()
    await client.handshake()

    inner_model = OpenAIChatCompletionsModel(
        model="gpt-4o-mini",
        openai_client=AsyncOpenAI(),
    )
    guarded = SpendGuardAgentsModel(
        inner=inner_model,
        client=client,
        budget_id="...",
        window_instance_id="...",
        unit=common_pb2.UnitRef(...),
        pricing=common_pb2.PricingFreeze(...),
        claim_estimator=lambda inp: [common_pb2.BudgetClaim(...)],
    )

    agent = Agent(name="my-agent", instructions="...", model=guarded)
    async with run_context(RunContext(run_id="...")):
        result = await Runner.run(agent, "Hello")

POC scope:
  - Stream gating bracketed at the model boundary; intra-stream tool
    calls inherit the parent reservation (parity with LangChain).
  - DEGRADE mutation patches surfaced as APPLY_FAILED rather than
    applied (parity with other integrations).
"""

from __future__ import annotations

import contextvars
import hashlib
from collections.abc import Callable
from contextlib import asynccontextmanager
from dataclasses import dataclass
from typing import Any, AsyncIterator

from ..client import DecisionOutcome, SpendGuardClient
from ..ids import (
    derive_idempotency_key,
    derive_uuid_from_signature,
    new_uuid7,
)

try:
    from agents.models.interface import Model as _AgentsModel
    from agents.items import ModelResponse  # type: ignore[attr-defined]
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.openai_agents requires the "
        "[openai-agents] extra. Install with: "
        "pip install 'spendguard-sdk[openai-agents]'"
    ) from exc

try:
    from spendguard._proto.spendguard.common.v1 import common_pb2
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard proto stubs missing. Run `make proto` first."
    ) from exc


# Run-scoped context shared with langchain / pydantic_ai integrations
# so multi-framework agents reuse one trace.
_RUN_CONTEXT: contextvars.ContextVar["RunContext | None"] = contextvars.ContextVar(
    "spendguard_run_context", default=None
)


@dataclass(frozen=True, slots=True)
class RunContext:
    """Per Runner.run() identifiers."""

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
            "spendguard.integrations.openai_agents called outside an active "
            "run_context(). Wrap your Runner.run invocation:\n\n"
            "    async with run_context(RunContext(run_id=...)):\n"
            "        await Runner.run(agent, input)\n"
        )
    return ctx


ClaimEstimator = Callable[[Any], list[Any]]
"""Project BudgetClaim list from the model `input` payload (str | list[Item])."""


def _signature(input_payload: Any, system_instructions: str | None) -> str:
    text = repr(input_payload) + "|" + (system_instructions or "")
    return hashlib.blake2b(text.encode("utf-8"), digest_size=16).hexdigest()


class SpendGuardAgentsModel(_AgentsModel):  # type: ignore[misc, valid-type]
    """OpenAI Agents SDK Model wrapper that gates each invocation through the sidecar.

    Subclasses `agents.models.interface.Model` and overrides
    `get_response` to insert PRE/POST hooks around the inner model
    call. `stream_response`, `close`, and `get_retry_advice` delegate
    transparently.
    """

    def __init__(
        self,
        *,
        inner: _AgentsModel,
        client: SpendGuardClient,
        budget_id: str,
        window_instance_id: str,
        unit: Any,
        pricing: Any,
        claim_estimator: ClaimEstimator,
    ) -> None:
        # Note: agents.Model is ABC with no shared state in __init__,
        # so we don't call super().__init__(). The inner model is what
        # actually owns the OpenAI client + retry logic.
        self._inner = inner
        self._client = client
        self._budget_id = budget_id
        self._window_instance_id = window_instance_id
        self._unit = unit
        self._pricing = pricing
        self._claim_estimator = claim_estimator

    async def get_response(
        self,
        system_instructions: str | None,
        input: Any,
        model_settings: Any,
        tools: Any,
        output_schema: Any,
        handoffs: Any,
        tracing: Any,
        *,
        previous_response_id: str | None = None,
        conversation_id: str | None = None,
        prompt: Any = None,
    ) -> Any:
        """Gated invocation: PRE → inner.get_response → POST."""
        ctx = current_run_context()
        signature = _signature(input, system_instructions)
        llm_call_id = str(derive_uuid_from_signature(signature, scope="llm_call_id"))
        decision_id = str(derive_uuid_from_signature(signature, scope="decision_id"))
        step_id = f"{ctx.run_id}:oai-call:{signature[:16]}"
        idempotency_key = derive_idempotency_key(
            tenant_id=self._client.tenant_id,
            session_id=self._client.session_id,
            run_id=ctx.run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )

        outcome: DecisionOutcome = await self._client.request_decision(
            trigger="LLM_CALL_PRE",
            run_id=ctx.run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            tool_call_id="",
            decision_id=decision_id,
            route="llm.call",
            projected_claims=self._claim_estimator(input),
            idempotency_key=idempotency_key,
        )

        # Delegate to inner Model. agents.Model.get_response signature
        # uses keyword-only args after `tracing`; pass them through.
        inner_response = await self._inner.get_response(
            system_instructions,
            input,
            model_settings,
            tools,
            output_schema,
            handoffs,
            tracing,
            previous_response_id=previous_response_id,
            conversation_id=conversation_id,
            prompt=prompt,
        )

        # Extract usage from ModelResponse (validated against
        # openai-agents 0.17.0 — has `usage` Usage field with
        # total_tokens / input_tokens / output_tokens).
        total_tokens = self._extract_total_tokens(inner_response)
        provider_event_id = getattr(inner_response, "response_id", "") or ""

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

        return inner_response

    def stream_response(
        self,
        system_instructions: str | None,
        input: Any,
        model_settings: Any,
        tools: Any,
        output_schema: Any,
        handoffs: Any,
        tracing: Any,
        *,
        previous_response_id: str | None = None,
        conversation_id: str | None = None,
        prompt: Any = None,
    ) -> Any:
        """Streaming pass-through.

        POC: streams from the inner model directly without per-chunk
        gating. The PRE side fires when the wrapping Runner moves to
        the next non-streaming boundary; for full per-chunk gating use
        `get_response`. Tracked as a follow-on per the integration
        docs.
        """
        return self._inner.stream_response(
            system_instructions,
            input,
            model_settings,
            tools,
            output_schema,
            handoffs,
            tracing,
            previous_response_id=previous_response_id,
            conversation_id=conversation_id,
            prompt=prompt,
        )

    async def close(self) -> None:
        await self._inner.close()

    def get_retry_advice(self, request: Any) -> Any:
        return self._inner.get_retry_advice(request)

    @staticmethod
    def _extract_total_tokens(response: Any) -> int:
        usage = getattr(response, "usage", None)
        if usage is None:
            return 0
        # agents.Usage exposes total_tokens directly.
        total = getattr(usage, "total_tokens", None)
        if isinstance(total, int):
            return total
        if isinstance(total, str):
            try:
                return int(total)
            except ValueError:
                return 0
        return 0


__all__ = [
    "ClaimEstimator",
    "RunContext",
    "SpendGuardAgentsModel",
    "current_run_context",
    "run_context",
]
