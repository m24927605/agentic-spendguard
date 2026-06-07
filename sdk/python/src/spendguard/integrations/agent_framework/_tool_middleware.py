"""SpendGuardToolMiddleware — opt-in tool-scope (TOOL_CALL_PRE) gating.

Sibling to ``SpendGuardMiddleware``: that one gates LLM calls; this one
gates function/tool invocations. They are deliberately separate classes
per ADR-002 (review-standards.md §2.1 D4):

  > LLM middleware ≠ tool middleware; they are distinct classes /
  > registrations.

This lets customers run token-budget enforcement at the LLM layer while
running a *separate* tool-cost budget on the function layer (e.g.
external API spend per tool call).

Usage::

    from agent_framework import ChatAgent

    chat_mw = SpendGuardMiddleware(client=..., options=..., unit=..., pricing=...)
    tool_mw = SpendGuardToolMiddleware(
        client=..., options=..., unit=..., pricing=...,
        claim_estimator=lambda fn, args: [BudgetClaim(...)],
    )
    agent = ChatAgent(chat_client=..., middleware=[chat_mw, tool_mw])
"""

from __future__ import annotations

import hashlib
import logging
from collections.abc import Awaitable, Callable
from typing import Any

from ...client import DecisionOutcome, SpendGuardClient
from ...ids import (
    derive_idempotency_key,
    derive_uuid_from_signature,
)
from ._errors import SidecarUnavailable, SpendGuardConfigError
from ._options import SpendGuardAgentFrameworkOptions
from ._run_context import current_run_context

try:
    from agent_framework import FunctionInvocationContext, FunctionMiddleware
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.agent_framework requires the "
        "[agent-framework] extra. Install with: "
        "pip install 'spendguard-sdk[agent-framework]'"
    ) from exc


logger = logging.getLogger(__name__)


ToolClaimEstimator = Callable[[Any, Any], list[Any]]
"""``(function, arguments) -> [BudgetClaim]`` — projects the cost of a tool call.

The two args mirror ``FunctionInvocationContext.function`` and
``FunctionInvocationContext.arguments``.
"""


def _tool_signature(function: Any, arguments: Any) -> str:
    """Stable content hash over function-name + arg payload for ID derivation."""
    name = getattr(function, "name", None) or repr(function)
    # arguments may be a pydantic BaseModel or a plain Mapping; coerce
    # both to a stable string form.
    if hasattr(arguments, "model_dump_json"):
        try:
            arg_str = arguments.model_dump_json()
        except Exception:  # noqa: BLE001
            arg_str = repr(arguments)
    else:
        arg_str = repr(arguments)
    payload = f"fn|{name}|args|{arg_str}"
    return hashlib.blake2b(payload.encode("utf-8"), digest_size=16).hexdigest()


class SpendGuardToolMiddleware(FunctionMiddleware):  # type: ignore[misc, valid-type]
    """MAF FunctionMiddleware that gates each tool/function call through SpendGuard.

    Lifecycle:
      1. PRE — ``RequestDecision(TOOL_CALL_PRE)`` keyed by ``tool_call_id``.
      2. CALL — ``await call_next()`` runs the tool.
      3. POST — caller-side observable via the audit chain; this
         middleware does not emit a separate POST event in v1 because
         tool-cost emission shape is provider-specific.

    Sidecar-unavailable handling is governed by the same
    ``SpendGuardAgentFrameworkOptions.on_sidecar_unavailable`` knob as
    the chat middleware.
    """

    def __init__(
        self,
        *,
        client: SpendGuardClient,
        options: SpendGuardAgentFrameworkOptions,
        unit: Any,
        pricing: Any,
        claim_estimator: ToolClaimEstimator,
        route: str = "tool.call",
    ) -> None:
        if client is None:
            raise SpendGuardConfigError(
                "SpendGuardToolMiddleware(client=...) is required; got None."
            )
        if not isinstance(options, SpendGuardAgentFrameworkOptions):
            raise SpendGuardConfigError(
                "SpendGuardToolMiddleware(options=...) must be a "
                "SpendGuardAgentFrameworkOptions instance."
            )
        if claim_estimator is None:
            raise SpendGuardConfigError(
                "SpendGuardToolMiddleware(claim_estimator=...) is required; "
                "tool cost shape is provider-specific and there is no "
                "default."
            )
        self._client = client
        self._options = options
        self._unit = unit
        self._pricing = pricing
        self._claim_estimator = claim_estimator
        self._route = route

    async def process(
        self,
        context: FunctionInvocationContext,
        call_next: Callable[[], Awaitable[None]],
    ) -> None:
        """Tool-scope gating — see class docstring for the lifecycle."""
        run_ctx = current_run_context()
        signature = _tool_signature(context.function, context.arguments)
        tool_call_id = str(
            derive_uuid_from_signature(signature, scope="tool_call_id")
        )
        decision_id = str(
            derive_uuid_from_signature(signature, scope="decision_id")
        )
        step_id = f"{run_ctx.run_id}:maf-tool:{signature[:16]}"
        idempotency_key = derive_idempotency_key(
            tenant_id=self._client.tenant_id,
            session_id=self._client.session_id,
            run_id=run_ctx.run_id,
            step_id=step_id,
            llm_call_id="",
            trigger="TOOL_CALL_PRE",
        )

        # ---- PRE: reserve ---------------------------------------------------
        try:
            _outcome: DecisionOutcome = await self._client.request_decision(
                trigger="TOOL_CALL_PRE",
                run_id=run_ctx.run_id,
                step_id=step_id,
                llm_call_id="",
                tool_call_id=tool_call_id,
                decision_id=decision_id,
                route=self._route,
                projected_claims=self._claim_estimator(
                    context.function, context.arguments
                ),
                idempotency_key=idempotency_key,
            )
        except SidecarUnavailable:
            if self._options.on_sidecar_unavailable == "allow":
                logger.warning(
                    "SpendGuard sidecar unavailable; on_sidecar_unavailable="
                    "'allow' — proceeding without a tool reservation. "
                    "run_id=%s",
                    run_ctx.run_id,
                )
                await call_next()
                return
            raise

        # ---- CALL: invoke inner function -----------------------------------
        await call_next()


__all__ = [
    "ToolClaimEstimator",
    "SpendGuardToolMiddleware",
]
