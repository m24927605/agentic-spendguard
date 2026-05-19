# ruff: noqa: ANN401  # LiteLLM's CustomLogger interface uses untyped Any
"""LiteLLM proxy CustomLogger integration. See DESIGN.md §3.4 v1 Path B.

Slice 1: skeleton + dataclasses. Async hook bodies land in Slices 2-5.
The callback only fires in LiteLLM **proxy** mode (verified against
litellm source 2026-05-20); direct `litellm.acompletion()` callers
use Shape A egress proxy (DESIGN §3.4 v1 Path A) — no SDK code here.
"""

from __future__ import annotations

import contextvars
from collections.abc import AsyncIterator, Callable, Mapping
from contextlib import asynccontextmanager
from dataclasses import dataclass
from typing import Any

from ..client import SpendGuardClient
from ..errors import SpendGuardConfigError  # noqa: F401  — used by Slice 2

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
    """LiteLLM proxy CustomLogger that reserves/commits via the
    SpendGuard sidecar. Only fires in LiteLLM **proxy** mode (per
    DESIGN.md §3.4 v1 Path B). Slice 1 skeleton: async hooks raise
    NotImplementedError; Slices 2-5 fill them in."""

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

    # NO log_pre_api_call override (Slice 1 R2 verified ineffective:
    # litellm_logging.py:45887 wraps callback invocations in
    # try/except Exception, exceptions are swallowed via
    # verbose_logger.exception). Sync direct callers route to Shape A
    # egress proxy per DESIGN.md §3.4 v1 Path A (no SDK code needed).


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

    # Round 1 P1 fix: the three async hooks MUST be overridden so
    # _ensure_client() runs before super() — locks the lazy-bind
    # contract regardless of which Slice fills the super body.
    async def async_pre_call_hook(self, *a: Any, **kw: Any) -> dict[str, Any] | None:
        await self._ensure_client()
        return await super().async_pre_call_hook(*a, **kw)

    async def async_log_success_event(self, *a: Any, **kw: Any) -> None:
        await self._ensure_client()
        await super().async_log_success_event(*a, **kw)

    async def async_log_failure_event(self, *a: Any, **kw: Any) -> None:
        await self._ensure_client()
        await super().async_log_failure_event(*a, **kw)


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
