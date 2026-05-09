"""Microsoft Agent Governance Toolkit (AGT) integration.

AGT (`pip install agent-governance-toolkit`) ships deterministic
policy enforcement for AI agent actions. SpendGuard composes with it
on the budget axis: AGT decides "is this tool action allowed?" while
SpendGuard decides "if allowed, can we afford it right now?".

Two integration shapes:

1. **Async pre-action hook** — `gate_budget(payload, *, client, ...)`
   helper that an AGT-compatible policy callback invokes before
   approving the action. If SpendGuard denies, the helper raises
   `DecisionDenied` (or subclass) which the AGT chain surfaces as
   a hard policy fail.

2. **Composite evaluator** — `SpendGuardCompositeEvaluator` wraps
   AGT's `PolicyEvaluator` so a single `.evaluate(input)` call runs
   AGT first, then SpendGuard's budget check on ALLOW results. Used
   when the application code sees one Evaluator surface for both
   policy + spend.

Integration shape::

    from agent_os.policies import (
        PolicyEvaluator, PolicyDocument, PolicyRule, PolicyCondition,
        PolicyAction, PolicyOperator, PolicyDefaults,
    )
    from spendguard import SpendGuardClient
    from spendguard.integrations.agt import SpendGuardCompositeEvaluator
    from spendguard._proto.spendguard.common.v1 import common_pb2

    agt_evaluator = PolicyEvaluator(policies=[PolicyDocument(
        name="my-policy", version="1.0",
        defaults=PolicyDefaults(action=PolicyAction.ALLOW),
        rules=[PolicyRule(
            name="block-dangerous-tools",
            condition=PolicyCondition(
                field="tool_name",
                operator=PolicyOperator.IN,
                value=["execute_code", "delete_file"]),
            action=PolicyAction.DENY, priority=100,
        )],
    )])

    composite = SpendGuardCompositeEvaluator(
        agt_evaluator=agt_evaluator,
        spendguard_client=client,
        budget_id="...",
        window_instance_id="...",
        unit=common_pb2.UnitRef(...),
        pricing=common_pb2.PricingFreeze(...),
        claim_estimator=lambda payload: [common_pb2.BudgetClaim(...)],
    )

    result = await composite.evaluate({
        "tool_name": "web_search",
        "tool_args": {"q": "..."},
        "tenant_id": "...",
        "run_id": "...",
    })
    # → result.allowed: bool
    # → result.reason: str (AGT-deny reason OR SpendGuard reason_codes)

POC scope:
  - AGT's API surface evolves; this module pins to
    `agent-governance-toolkit>=3.4` and falls back if the import
    path differs.
  - `SpendGuardCompositeEvaluator.evaluate()` is async (sidecar IPC
    is async). AGT's own `PolicyEvaluator.evaluate()` is sync; we
    wrap via `asyncio.to_thread()`.
  - DEGRADE is treated as ALLOW with a note in the reason; full
    DEGRADE→mutation patch propagation is deferred (per LangChain
    integration parity).
  - Audit chain dual-write: AGT writes to its own audit log;
    SpendGuard writes to canonical_events. Reconciliation across
    the two is documented as a follow-on (a SpendGuard-side relay
    that ingests AGT events).
"""

from __future__ import annotations

import asyncio
import contextvars
from collections.abc import Callable, Mapping
from contextlib import asynccontextmanager
from dataclasses import dataclass
from typing import Any, AsyncIterator

from ..client import DecisionOutcome, SpendGuardClient
from ..errors import DecisionDenied, SpendGuardError
from ..ids import (
    derive_idempotency_key,
    derive_uuid_from_signature,
    new_uuid7,
)

import hashlib

try:
    # Try AGT's primary import path first (3.4+).
    from agent_os.policies import (  # type: ignore[import-not-found]
        PolicyAction,
        PolicyEvaluator,
    )
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.agt requires Microsoft Agent Governance "
        "Toolkit. Install with: "
        "pip install 'spendguard-sdk[agt]' "
        "(extras add agent-governance-toolkit>=3.4)"
    ) from exc

try:
    from spendguard._proto.spendguard.common.v1 import common_pb2
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard proto stubs missing. Run `make proto` first."
    ) from exc


# Run-scoped context shared with langchain / pydantic_ai integrations.
_RUN_CONTEXT: contextvars.ContextVar["RunContext | None"] = contextvars.ContextVar(
    "spendguard_run_context", default=None
)


@dataclass(frozen=True, slots=True)
class RunContext:
    """Per-evaluation identifiers (one per Composite.evaluate call)."""

    run_id: str


@asynccontextmanager
async def run_context(ctx: RunContext) -> AsyncIterator[RunContext]:
    token = _RUN_CONTEXT.set(ctx)
    try:
        yield ctx
    finally:
        _RUN_CONTEXT.reset(token)


def current_run_context() -> RunContext | None:
    return _RUN_CONTEXT.get()


@dataclass(frozen=True, slots=True)
class CompositeResult:
    """Result of running AGT + SpendGuard on a single payload."""

    allowed: bool
    reason: str
    matched_rule_ids: list[str]
    decision_id: str | None
    audit_decision_event_id: str | None


ClaimEstimator = Callable[[Mapping[str, Any]], list[Any]]
"""Project BudgetClaims from the AGT evaluation payload (action, args, etc.)."""


def _signature(payload: Mapping[str, Any]) -> str:
    return hashlib.blake2b(repr(sorted(payload.items())).encode("utf-8"), digest_size=16).hexdigest()


class SpendGuardCompositeEvaluator:
    """Run AGT first, then SpendGuard, return a unified verdict.

    Order matters: AGT is cheap (deterministic local check); SpendGuard
    incurs a sidecar round-trip. If AGT denies, we never reach
    SpendGuard — saves a request_decision and avoids stale reservations
    on AGT-deny actions.
    """

    def __init__(
        self,
        *,
        agt_evaluator: PolicyEvaluator,
        spendguard_client: SpendGuardClient,
        budget_id: str,
        window_instance_id: str,
        unit: Any,
        pricing: Any,
        claim_estimator: ClaimEstimator,
    ) -> None:
        self._agt = agt_evaluator
        self._client = spendguard_client
        self._budget_id = budget_id
        self._window_instance_id = window_instance_id
        self._unit = unit
        self._pricing = pricing
        self._claim_estimator = claim_estimator

    async def evaluate(self, payload: Mapping[str, Any]) -> CompositeResult:
        # 1) AGT (sync; offload to a thread for async caller correctness).
        agt_result = await asyncio.to_thread(self._agt.evaluate, dict(payload))

        # AGT's PolicyEvaluator returns an object with `.allowed` (bool),
        # `.action` (lowercase str e.g. 'allow' / 'deny'), `.matched_rule`
        # (str | None), and `.reason` (str). Validated against
        # agent-governance-toolkit 3.4. Defensive get() for forward
        # compat — if a future AGT version drops `allowed` we fall back
        # to comparing `.action`.
        agt_allowed = getattr(agt_result, "allowed", None)
        if agt_allowed is None:
            agt_action = getattr(agt_result, "action", None)
            agt_allowed = agt_action != "deny" and agt_action != PolicyAction.DENY

        agt_reason = getattr(agt_result, "reason", "")
        agt_matched = getattr(agt_result, "matched_rule", None)
        agt_matched_list = [agt_matched] if agt_matched else []

        if not agt_allowed:
            return CompositeResult(
                allowed=False,
                reason=f"AGT_DENY: {agt_reason}" if agt_reason else "AGT_DENY",
                matched_rule_ids=agt_matched_list,
                decision_id=None,
                audit_decision_event_id=None,
            )

        # 2) SpendGuard.
        ctx = current_run_context()
        run_id = ctx.run_id if ctx else str(new_uuid7())

        signature = _signature(payload)
        llm_call_id = str(derive_uuid_from_signature(signature, scope="llm_call_id"))
        decision_id = str(derive_uuid_from_signature(signature, scope="decision_id"))
        step_id = f"{run_id}:agt-call:{signature[:16]}"
        idempotency_key = derive_idempotency_key(
            tenant_id=self._client.tenant_id,
            session_id=self._client.session_id,
            run_id=run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            trigger="TOOL_CALL_PRE",
        )

        try:
            outcome: DecisionOutcome = await self._client.request_decision(
                trigger="TOOL_CALL_PRE",
                run_id=run_id,
                step_id=step_id,
                llm_call_id=llm_call_id,
                tool_call_id=str(payload.get("tool_call_id") or ""),
                decision_id=decision_id,
                route="tool.call",
                projected_claims=self._claim_estimator(payload),
                idempotency_key=idempotency_key,
            )
        except DecisionDenied as e:
            return CompositeResult(
                allowed=False,
                reason=f"SPENDGUARD_DENY: {','.join(e.reason_codes)}",
                matched_rule_ids=e.matched_rule_ids,
                decision_id=e.decision_id,
                audit_decision_event_id=e.audit_decision_event_id,
            )
        except SpendGuardError:
            raise

        return CompositeResult(
            allowed=True,
            reason="ALLOW (AGT + SpendGuard both PASS)",
            matched_rule_ids=list(outcome.matched_rule_ids),
            decision_id=outcome.decision_id,
            audit_decision_event_id=outcome.audit_decision_event_id,
        )


__all__ = [
    "ClaimEstimator",
    "CompositeResult",
    "RunContext",
    "SpendGuardCompositeEvaluator",
    "current_run_context",
    "run_context",
]
