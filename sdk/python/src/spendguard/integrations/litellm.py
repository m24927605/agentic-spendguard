# ruff: noqa: ANN401  # LiteLLM's CustomLogger interface uses untyped Any
"""LiteLLM CustomLogger integration. See DESIGN.md §3.4 (Shape B).

Slice 1: skeleton + dataclasses + sync-fail-closed override. Async
hook bodies land in Slices 2-5. Proxy mode: `_LoopBoundCallback` +
DESIGN.md §7.2 PROXY_RECIPE.
"""

from __future__ import annotations

import contextvars
from collections.abc import AsyncIterator, Callable, Mapping
from contextlib import asynccontextmanager
from dataclasses import dataclass
from typing import Any

from ..client import SpendGuardClient
from ..errors import SpendGuardConfigError

try:
    from litellm.integrations.custom_logger import CustomLogger
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.litellm requires LiteLLM. "
        "Install with: pip install 'spendguard-sdk[litellm]'"
    ) from exc


_RUN_CONTEXT: contextvars.ContextVar[LiteLLMRunContext | None] = (
    contextvars.ContextVar("spendguard_litellm_run_context", default=None)
)


@dataclass(frozen=True, slots=True)
class LiteLLMRunContext:
    """Per-call identifiers. `step_id` is optional; callback derives a
    per-call step from `litellm_call_id` when None."""
    run_id: str
    step_id: str | None = None


@asynccontextmanager
async def run_context(
    ctx: LiteLLMRunContext,
) -> AsyncIterator[LiteLLMRunContext]:
    token = _RUN_CONTEXT.set(ctx)
    try:
        yield ctx
    finally:
        _RUN_CONTEXT.reset(token)


def current_run_context() -> LiteLLMRunContext | None:
    return _RUN_CONTEXT.get()


@dataclass(frozen=True, slots=True)
class ResolverContext:
    """Inputs the BudgetResolver sees on every call. Hook constructs
    this explicitly from `async_pre_call_hook` arguments — resolver
    MUST NOT scrape `data["user_api_key_dict"]` (not guaranteed
    present in LiteLLM kwargs)."""
    data: Mapping[str, Any]
    user_api_key_dict: Any | None
    call_type: str


@dataclass(frozen=True, slots=True)
class BudgetBinding:
    """Per-call binding: which budget/window/unit/pricing to use.
    Operator-supplied via the BudgetResolver."""
    budget_id: str
    window_instance_id: str
    unit: Any       # common_pb2.UnitRef
    pricing: Any    # common_pb2.PricingFreeze


BudgetResolver = Callable[[ResolverContext], "BudgetBinding | None"]
"""Map ResolverContext → BudgetBinding. Returning None raises
SpendGuardConfigError (no global default fallback — ADR-001)."""

ClaimEstimator = Callable[[ResolverContext], list[Any]]
"""Project BudgetClaims pre-call. v1 contract: exactly one claim."""

ClaimReconciler = Callable[[ResolverContext, Any], list[Any]]
"""Compute real claims at commit from ResolverContext + response_obj.
v1 contract: exactly one claim."""


class SpendGuardLiteLLMCallback(CustomLogger):
    """LiteLLM CustomLogger that reserves/commits via the SpendGuard sidecar.

    Slice 1 skeleton: async hooks raise NotImplementedError; sync
    log_pre_api_call raises SpendGuardConfigError so sync callers
    fail-closed BEFORE the wire (DESIGN.md ADR-005)."""

    def __init__(
        self,
        *,
        client: SpendGuardClient | None,
        budget_resolver: BudgetResolver,
        claim_estimator: ClaimEstimator,
        claim_reconciler: ClaimReconciler,
        fail_closed: bool = True,
    ) -> None:
        self._client = client
        self._budget_resolver = budget_resolver
        self._claim_estimator = claim_estimator
        self._claim_reconciler = claim_reconciler
        self._fail_closed = fail_closed
        # Per-call stash; lives on the callback (P1.5), keyed by
        # litellm_call_id. Slice 2 populates; Slices 3/4/5 consume.
        self._stash: dict[str, dict[str, Any]] = {}

    async def async_pre_call_hook(
        self,
        user_api_key_dict: Any,
        cache: Any,
        data: dict[str, Any],
        call_type: str,
    ) -> dict[str, Any] | None:
        raise NotImplementedError("Slice 2")

    async def async_log_success_event(
        self,
        kwargs: dict[str, Any],
        response_obj: Any,
        start_time: Any,
        end_time: Any,
    ) -> None:
        raise NotImplementedError("Slice 3 / Slice 4 (streaming)")

    async def async_log_failure_event(
        self,
        kwargs: dict[str, Any],
        response_obj: Any,
        start_time: Any,
        end_time: Any,
    ) -> None:
        raise NotImplementedError("Slice 5")

    def log_pre_api_call(
        self,
        model: str,
        messages: list[dict[str, Any]],
        kwargs: dict[str, Any],
    ) -> None:
        # Sync pre-wire hook (Round 2 P0.7). LiteLLM dispatches this
        # for sync litellm.completion() — fail-closed before the wire.
        raise SpendGuardConfigError(
            "Sync litellm.completion() is not supported by the "
            "SpendGuard callback. Use litellm.acompletion() or Shape A. "
            "See DESIGN.md ADR-005."
        )


class _LoopBoundCallback(SpendGuardLiteLLMCallback):
    """Lazy-init wrapper that binds SpendGuardClient to LiteLLM's
    serving event loop (Round 3 P0.3). gRPC/UDS channels are loop-
    affine; the LiteLLM proxy imports modules sync at boot then runs
    its own ASGI loop. Slice 1 skeleton; Slices 2-5 fill the hooks."""

    def __init__(
        self,
        *,
        socket_path: str,
        tenant_id: str,
        budget_resolver: BudgetResolver,
        claim_estimator: ClaimEstimator,
        claim_reconciler: ClaimReconciler,
        fail_closed: bool = True,
    ) -> None:
        super().__init__(
            client=None,
            budget_resolver=budget_resolver,
            claim_estimator=claim_estimator,
            claim_reconciler=claim_reconciler,
            fail_closed=fail_closed,
        )
        self._socket_path = socket_path
        self._tenant_id = tenant_id
        self._init_lock: Any = None  # asyncio.Lock — created on first hook

    async def _ensure_client(self) -> SpendGuardClient:
        raise NotImplementedError("Slice 2 wires _LoopBoundCallback handshake")


def install(
    *,
    client: SpendGuardClient,
    budget_resolver: BudgetResolver,
    claim_estimator: ClaimEstimator,
    claim_reconciler: ClaimReconciler,
    fail_closed: bool = True,
) -> SpendGuardLiteLLMCallback:
    """Build the callback and append to litellm.callbacks. Slice 2."""
    raise NotImplementedError("Slice 2")


__all__ = [
    "BudgetBinding",
    "BudgetResolver",
    "ClaimEstimator",
    "ClaimReconciler",
    "LiteLLMRunContext",
    "ResolverContext",
    "SpendGuardLiteLLMCallback",
    "_LoopBoundCallback",
    "current_run_context",
    "install",
    "run_context",
]
