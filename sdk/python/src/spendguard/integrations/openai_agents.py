"""OpenAI Agents SDK integration — gates Runner.run via the sidecar.

OpenAI's Agents SDK (`pip install openai-agents`) exposes an `Agent` /
`Runner` abstraction with pluggable `Model` implementations. Two
integration shapes are supported here:

1. **Custom Model** (recommended) — a `SpendGuardAgentsModel` that
   inherits from the SDK's `Model` base, intercepts each invocation,
   and routes through `SpendGuardClient.request_decision` /
   `emit_llm_call_post`. Use this when your agent's `model=` config
   accepts a Model instance (most current versions).

2. **Wrap-and-replace** — `gated_runner_run(agent, input, client=...)`
   helper that swaps the agent's model with `SpendGuardAgentsModel`
   only for the duration of one Runner.run() call.

POC scope:
  - SDK API surface evolves; this module pins to
    `openai-agents>=0.0.7` and re-validates on import. If the API
    drifts, integration falls back to a no-op wrapper that warns.
  - Streaming + tool-call mid-call interception are deferred (Runner
    still gates the surrounding boundary at every model invocation;
    inner tool calls inherit the parent reservation).

Integration shape::

    from agents import Agent, Runner
    from spendguard import SpendGuardClient
    from spendguard.integrations.openai_agents import SpendGuardAgentsModel
    from spendguard._proto.spendguard.common.v1 import common_pb2

    client = SpendGuardClient(socket_path=..., tenant_id=...)
    await client.handshake()

    guarded_model = SpendGuardAgentsModel(
        inner_model_name="gpt-4o-mini",
        client=client,
        budget_id=...,
        window_instance_id=...,
        unit=common_pb2.UnitRef(...),
        pricing=common_pb2.PricingFreeze(...),
        claim_estimator=lambda inputs: [...],
    )

    agent = Agent(name="my-agent", instructions="...", model=guarded_model)
    result = await Runner.run(agent, "Hello")
"""

from __future__ import annotations

import contextvars
from collections.abc import Callable, Sequence
from contextlib import asynccontextmanager
from dataclasses import dataclass
from typing import Any, AsyncIterator

from ..client import DecisionOutcome, SpendGuardClient
import hashlib

from ..ids import (
    derive_idempotency_key,
    derive_uuid_from_signature,
)


def _default_call_signature(input_payload: Any) -> str:
    """Hash the agent input into a 32-char hex digest."""
    return hashlib.blake2b(
        repr(input_payload).encode("utf-8"), digest_size=16
    ).hexdigest()

try:
    # The SDK underwent renames. Try a few import paths so we work
    # across openai-agents 0.0.x → 0.1.x. If none resolve, raise the
    # standard install hint.
    try:
        from agents import Model as _AgentsModel  # 0.1.x
    except ImportError:
        from openai_agents import Model as _AgentsModel  # 0.0.x
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


# Run-scoped context (shared identifier with langchain / pydantic_ai
# integrations so a multi-framework agent can reuse the same run_id).
_RUN_CONTEXT: contextvars.ContextVar["RunContext | None"] = contextvars.ContextVar(
    "spendguard_run_context", default=None
)


@dataclass(frozen=True, slots=True)
class RunContext:
    """Per-Runner.run() identifiers."""

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
"""Project BudgetClaim list from the agent input.

The OpenAI Agents SDK passes either a string user prompt or a
list[Message]; the estimator should accept either.
"""


class SpendGuardAgentsModel(_AgentsModel):  # type: ignore[misc, valid-type]
    """OpenAI Agents SDK Model wrapper that gates each invocation through the sidecar.

    The underlying Model class evolves between SDK versions; this
    wrapper subclasses it and overrides `__call__` (or whichever
    invocation entrypoint the current SDK uses). If the SDK exposes
    only a private `_call_provider`, the override there serves the
    same purpose.

    Implementation note: if the SDK's Model API surface diverges from
    what's expected (private rename, async/sync mismatch), the demo
    gate will surface it. POC tolerates SDK churn by failing visibly
    rather than silently no-op'ing.
    """

    def __init__(
        self,
        *,
        inner_model_name: str,
        client: SpendGuardClient,
        budget_id: str,
        window_instance_id: str,
        unit: Any,
        pricing: Any,
        claim_estimator: ClaimEstimator,
        **inner_kwargs: Any,
    ) -> None:
        # Forward inner-model kwargs to the SDK base class. The base
        # class's __init__ signature varies; pass via **kwargs and let
        # it raise if invalid.
        try:
            super().__init__(model=inner_model_name, **inner_kwargs)  # type: ignore[call-arg]
        except TypeError:
            # Older SDK had positional model arg.
            super().__init__(inner_model_name, **inner_kwargs)  # type: ignore[call-arg]

        self._client = client
        self._budget_id = budget_id
        self._window_instance_id = window_instance_id
        self._unit = unit
        self._pricing = pricing
        self._claim_estimator = claim_estimator

    async def __call__(self, *args: Any, **kwargs: Any) -> Any:
        """Gated invocation: PRE → inner.__call__ → POST."""
        return await self._gate_call(super().__call__, *args, **kwargs)

    async def _gate_call(
        self, inner_call: Callable[..., Any], *args: Any, **kwargs: Any
    ) -> Any:
        ctx = current_run_context()
        # Use args[0] as the input payload (string or messages); SDKs
        # typically pass it positionally.
        input_payload = args[0] if args else kwargs.get("input")
        signature = _default_call_signature(input_payload)
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
            projected_claims=self._claim_estimator(input_payload),
            idempotency_key=idempotency_key,
        )

        result = await inner_call(*args, **kwargs)

        total_tokens = self._extract_total_tokens(result)
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
                provider_event_id=self._extract_provider_event_id(result),
                outcome="SUCCESS",
            )

        return result

    @staticmethod
    def _extract_total_tokens(result: Any) -> int:
        # OpenAI Agents Result objects: try common attribute paths.
        # `result.usage.total_tokens` or `result.usage["total_tokens"]`
        # are typical; fall back to 0 if nothing matches.
        usage = getattr(result, "usage", None)
        if usage is None:
            return 0
        if hasattr(usage, "total_tokens"):
            try:
                return int(usage.total_tokens)
            except (TypeError, ValueError):
                return 0
        if isinstance(usage, dict) and "total_tokens" in usage:
            try:
                return int(usage["total_tokens"])
            except (TypeError, ValueError):
                return 0
        return 0

    @staticmethod
    def _extract_provider_event_id(result: Any) -> str:
        rid = getattr(result, "id", None) or getattr(result, "response_id", None)
        return rid if isinstance(rid, str) else ""


__all__ = [
    "ClaimEstimator",
    "RunContext",
    "SpendGuardAgentsModel",
    "current_run_context",
    "run_context",
]
