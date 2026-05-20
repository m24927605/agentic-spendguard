# ruff: noqa: ANN401  # LiteLLM's CustomLogger interface uses untyped Any
"""LiteLLM proxy CustomLogger integration. See DESIGN.md §3.4 v1 Path B.

Slice 1: skeleton + dataclasses.
Slice 2: pre-call hook + reservation lifecycle.
Slices 3-5: success/streaming/failure hook bodies.

The callback only fires in LiteLLM **proxy** mode (verified against
litellm source 2026-05-20); direct `litellm.acompletion()` callers
use Shape A egress proxy (DESIGN §3.4 v1 Path A) — no SDK code here.
"""

from __future__ import annotations

import asyncio
import contextvars
import json
import logging
import os
import re as _re
import time
from collections.abc import AsyncIterator, Callable, Mapping
from contextlib import asynccontextmanager
from dataclasses import dataclass
from types import SimpleNamespace
from typing import Any

from ..client import SpendGuardClient
from ..errors import (
    DecisionDenied,
    SidecarUnavailable,
    SpendGuardConfigError,
    SpendGuardError,
)
from ..ids import (
    derive_idempotency_key,
    derive_uuid_from_signature,
)
from ..prompt_hash import compute as compute_prompt_hash

try:
    from litellm.integrations.custom_logger import CustomLogger
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.litellm requires LiteLLM. "
        "Install with: pip install 'spendguard-sdk[litellm]'"
    ) from exc


log = logging.getLogger("spendguard.integrations.litellm")


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
ClaimEstimator = Callable[[ResolverContext], list[Any]]
ClaimReconciler = Callable[[ResolverContext, Any], list[Any]]


def _build_resolver_ctx(
    *,
    user_api_key_dict: Any,
    data: Mapping[str, Any],
    call_type: str,
) -> ResolverContext:
    return ResolverContext(
        data=data,
        user_api_key_dict=user_api_key_dict,
        call_type=call_type,
    )


def _build_decision_context(
    *,
    ctx: ResolverContext,
    binding: BudgetBinding,
    litellm_call_id: str,
    prompt_hash: str,
) -> dict[str, Any]:
    """Returns the 12-field dict the sidecar persists into
    canonical_events (DESIGN.md §8.2a). Until GH #77 lands sidecar
    enrichment, fields land in runtime_metadata Struct but only
    prompt_hash is currently extracted by the sidecar."""
    p = binding.pricing
    uak = ctx.user_api_key_dict
    return {
        "integration": "litellm",
        "litellm_call_id": litellm_call_id,
        "model": ctx.data.get("model"),
        "pricing_version": getattr(p, "pricing_version", ""),
        "price_snapshot_hash_hex": getattr(p, "price_snapshot_hash_hex", ""),
        "fx_rate_version": getattr(p, "fx_rate_version", ""),
        "unit_conversion_version": getattr(p, "unit_conversion_version", ""),
        "prompt_hash": prompt_hash,
        "call_type": ctx.call_type,
        "stream": bool(ctx.data.get("stream", False)),
        "mode": "proxy",  # v1 always proxy (DESIGN §3.4 v1 Path B)
        "team_id": getattr(uak, "team_id", None) if uak else None,
    }


def _validate_claim_against_binding(
    claim: Any, binding: BudgetBinding, *, source: str,
) -> None:
    """Validate a BudgetClaim's identity fields match the binding.
    Used by both pre-call (estimator) and commit (reconciler) paths
    to fail-closed on mis-attribution.

    Raises SpendGuardConfigError on any mismatch. `source` is the
    caller name (e.g. "claim_estimator", "claim_reconciler") for
    error messages.
    """
    claim_budget_id = getattr(claim, "budget_id", None) or ""
    claim_window = getattr(claim, "window_instance_id", None) or ""
    if claim_budget_id != binding.budget_id:
        raise SpendGuardConfigError(
            f"{source} returned budget_id={claim_budget_id!r} but "
            f"binding has budget_id={binding.budget_id!r}. Audit "
            "context would mis-charge."
        )
    if claim_window != binding.window_instance_id:
        raise SpendGuardConfigError(
            f"{source} returned window_instance_id={claim_window!r} "
            f"but binding has window_instance_id="
            f"{binding.window_instance_id!r}."
        )
    # Slice 3 R2 + R3 P1 fix: validate unit.unit_id MANDATORY and
    # EXACT. A claim with no unit / empty unit_id would otherwise
    # silently commit amount under binding.unit semantics (mis-charge).
    binding_unit = getattr(binding, "unit", None)
    binding_unit_id = getattr(binding_unit, "unit_id", None) or ""
    if not binding_unit_id:
        raise SpendGuardConfigError(
            "BudgetBinding.unit.unit_id is empty; resolver MUST yield a "
            "non-empty unit (DESIGN.md §6)."
        )
    claim_unit = getattr(claim, "unit", None)
    claim_unit_id = getattr(claim_unit, "unit_id", None) or ""
    if claim_unit_id != binding_unit_id:
        raise SpendGuardConfigError(
            f"{source} returned unit.unit_id={claim_unit_id!r} but "
            f"binding has unit.unit_id={binding_unit_id!r}. Amount "
            "would be committed under wrong unit semantics."
        )


def _serialize_messages_for_hash(messages: Any) -> str:
    """Stable canonical-JSON of LiteLLM messages for prompt_hash input."""
    if messages is None:
        return ""
    try:
        return json.dumps(messages, sort_keys=True, separators=(",", ":"))
    except (TypeError, ValueError):
        return repr(messages)  # last-resort stable string


class SpendGuardLiteLLMCallback(CustomLogger):
    """LiteLLM proxy CustomLogger that reserves/commits via the
    SpendGuard sidecar. Only fires in LiteLLM **proxy** mode (per
    DESIGN.md §3.4 v1 Path B)."""

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
        # Read env once at construction (DESIGN §7.1 + S6 fail-open loud).
        self._fail_open_dev: bool = (
            os.environ.get("SPENDGUARD_LITELLM_FAIL_OPEN") == "1"
        )
        if self._fail_open_dev:
            log.warning(
                "spendguard: SPENDGUARD_LITELLM_FAIL_OPEN=1 — fail-open "
                "mode active; sidecar errors will allow LLM calls. "
                "DEV ONLY (DESIGN.md ADR-004)."
            )
        # Slice 4 R1 P1.3: TTL env var is INFORMATIONAL ONLY in v1.
        # SpendGuardClient.request_decision has no per-reservation TTL
        # parameter; reservation TTL is sidecar-side config. We
        # validate the value but it is NOT plumbed to the sidecar.
        # Future slice adds `ttl_seconds=` to request_decision.
        try:
            self._ttl_seconds: int = int(
                os.environ.get("SPENDGUARD_LITELLM_TTL_SECONDS", "300")
            )
            if self._ttl_seconds < 0:
                raise ValueError("must be ≥ 0")
        except (ValueError, TypeError) as exc:
            raise SpendGuardConfigError(
                f"SPENDGUARD_LITELLM_TTL_SECONDS must be a non-negative "
                f"integer: {exc}"
            ) from exc
        # Per-call stash, keyed by litellm_call_id (P1.5 — never on data).
        self._stash: dict[str, dict[str, Any]] = {}

    async def async_pre_call_hook(
        self,
        user_api_key_dict: Any,
        cache: Any,
        data: dict[str, Any],
        call_type: str,
    ) -> dict[str, Any] | None:
        if self._client is None:
            raise SpendGuardConfigError(
                "SpendGuardLiteLLMCallback has no client. "
                "Direct instantiation: pass client=. Proxy mode: use "
                "_LoopBoundCallback so the client binds to the serving loop."
            )

        rctx = _build_resolver_ctx(
            user_api_key_dict=user_api_key_dict, data=data, call_type=call_type,
        )
        binding = self._budget_resolver(rctx)
        if binding is None:
            raise SpendGuardConfigError(
                "budget_resolver returned None; resolver MUST yield a "
                "BudgetBinding (DESIGN.md ADR-001 — no global default)"
            )
        # Slice 2 R3 P1: binding fields MUST be non-empty. Empty
        # binding + empty claim would silently pass the later equality
        # check and reach the sidecar with an invalid reservation.
        if not binding.budget_id:
            raise SpendGuardConfigError(
                "BudgetBinding.budget_id is empty; resolver MUST yield a "
                "non-empty budget_id (DESIGN.md §6)."
            )
        if not binding.window_instance_id:
            raise SpendGuardConfigError(
                "BudgetBinding.window_instance_id is empty; resolver MUST "
                "yield a non-empty window_instance_id (DESIGN.md §6)."
            )

        litellm_call_id = data.get("litellm_call_id")
        if not litellm_call_id:
            # Pivot R0 P1.1: fail-closed when LiteLLM doesn't stamp an ID
            # (would break commit-lookup + LiteLLM_SpendLogs reconciliation).
            raise SpendGuardConfigError(
                "data['litellm_call_id'] missing — LiteLLM did not stamp a "
                "call id. Verify litellm>=1.50 and callback runs in proxy "
                "mode (the only path that gates via this hook)."
            )
        litellm_call_id = str(litellm_call_id)
        llm_call_id = str(derive_uuid_from_signature(
            f"litellm:{litellm_call_id}", scope="llm_call_id"))
        decision_id = str(derive_uuid_from_signature(
            f"litellm:{litellm_call_id}", scope="decision_id"))

        ctx_obj = current_run_context()
        run_id = ctx_obj.run_id if ctx_obj else str(
            derive_uuid_from_signature(
                f"litellm:{litellm_call_id}", scope="run_id"))
        step_id = (ctx_obj.step_id if ctx_obj and ctx_obj.step_id
                   else f"litellm:{litellm_call_id[:16]}")

        idempotency_key = derive_idempotency_key(
            tenant_id=self._client.tenant_id,
            session_id=self._client.session_id,
            run_id=run_id, step_id=step_id, llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )

        prompt_hash = compute_prompt_hash(
            _serialize_messages_for_hash(data.get("messages")),
            self._client.tenant_id,
        )
        decision_context = _build_decision_context(
            ctx=rctx, binding=binding, litellm_call_id=litellm_call_id,
            prompt_hash=prompt_hash,
        )

        # Estimator called ONCE; reused for request + stash (R2 P1.1).
        estimator_claims = self._claim_estimator(rctx)
        # R3 P1.2: enforce single-claim contract BEFORE sidecar wire.
        if len(estimator_claims) != 1:
            raise SpendGuardConfigError(
                f"claim_estimator returned {len(estimator_claims)} claims; "
                "v1 contract requires exactly 1 (DESIGN.md §6)."
            )
        # Slice 2 R1 P1.1 + R2 P1.1 + Slice 3 R2 P1 fix: validate
        # estimator claim EXACTLY matches the binding (budget_id +
        # window_instance_id + unit). Mismatch (including None/empty
        # when binding is non-empty) means audit context would
        # mis-charge.
        _validate_claim_against_binding(estimator_claims[0], binding, source="claim_estimator")

        # Slice 4 R2 P1 fix: build the primitive snapshot BEFORE the
        # sidecar await so an in-hook mutation of the operator's
        # shared mutable claim object (during the await window)
        # cannot change what the streaming fallback later commits.
        # Captured here = same primitive values the sidecar will see.
        _estimator_snapshot = SimpleNamespace(
            amount_atomic=str(getattr(estimator_claims[0], "amount_atomic", "")),
            budget_id=str(getattr(estimator_claims[0], "budget_id", "")),
            window_instance_id=str(
                getattr(estimator_claims[0], "window_instance_id", "")
            ),
            unit=SimpleNamespace(unit_id=str(
                getattr(
                    getattr(estimator_claims[0], "unit", None), "unit_id", "",
                )
            )),
        )

        try:
            outcome = await self._client.request_decision(
                trigger="LLM_CALL_PRE",
                run_id=run_id, step_id=step_id, llm_call_id=llm_call_id,
                tool_call_id="", decision_id=decision_id, route="llm.call",
                projected_claims=estimator_claims,
                idempotency_key=idempotency_key,
                projected_unit=binding.unit,
                # R4 P0.1: 12-field bundle. Folded into runtime_metadata
                # by client.py; sidecar passthrough is GH #77.
                decision_context_json=decision_context,
            )
        except DecisionDenied:
            raise  # proxy treats raised exception as block
        except SpendGuardError as exc:
            if self._fail_open_dev:
                log.warning(
                    "spendguard: SPENDGUARD_LITELLM_FAIL_OPEN=1 — allowing "
                    "call despite sidecar error %r (DEV ONLY).", exc,
                )
                return data
            raise SidecarUnavailable(
                f"sidecar pre-call failed: {exc}"
            ) from exc

        # Slice 2 R1 P0.1 fix: DEGRADE outcome must fail-closed for
        # LiteLLM (DESIGN §5 ledger-down row). DEGRADE means the
        # sidecar couldn't fully evaluate (e.g. Postgres down);
        # allowing a real-money LLM call under that condition breaks
        # F2 fail-closed + F4 audit coverage.
        if getattr(outcome, "decision", "") == "DEGRADE":
            if self._fail_open_dev:
                log.warning(
                    "spendguard: DEGRADE outcome under fail-open; "
                    "allowing call (DEV ONLY)."
                )
                return data
            raise SidecarUnavailable(
                "sidecar returned DEGRADE (ledger or dependent service "
                "unavailable); LiteLLM proxy fails closed on DEGRADE "
                "(DESIGN.md §5)."
            )

        # R4 P0.2: validate reservation cardinality BEFORE returning.
        # Slice 2 R1 P1.2 fix: when sidecar returned multi-reservation
        # outcome (shouldn't happen but defensive), proactively release
        # each one before raising so TTL-sweep is the backstop, not the
        # primary path. fire-and-forget; we already know we're failing.
        if len(outcome.reservation_ids) != 1:
            for rid in outcome.reservation_ids:
                try:
                    await self._client.emit_llm_call_post(
                        run_id=run_id, step_id=step_id, llm_call_id=llm_call_id,
                        decision_id=outcome.decision_id,
                        reservation_id=rid,
                        provider_reported_amount_atomic="0",
                        unit=binding.unit, pricing=binding.pricing,
                        provider_event_id="",
                        outcome="FAILURE",
                    )
                except Exception as rel_exc:  # noqa: BLE001
                    # best-effort; TTL sweep is durable backstop
                    log.warning(
                        "spendguard: best-effort release of reservation "
                        "%s failed: %r", rid, rel_exc,
                    )
            raise SpendGuardConfigError(
                f"sidecar returned {len(outcome.reservation_ids)} "
                "reservations; v1 expects exactly 1 (DESIGN.md §6). "
                "Failing closed before provider HTTP request "
                "(reservations released best-effort)."
            )

        # Stash on side-channel keyed by litellm_call_id (P1.5).
        self._stash[litellm_call_id] = {
            "decision_id": outcome.decision_id,
            "reservation_ids": tuple(outcome.reservation_ids),  # plural
            "llm_call_id": llm_call_id,
            "run_id": run_id, "step_id": step_id,
            "binding": binding,
            "audit_decision_event_id": outcome.audit_decision_event_id,
            "decision_context": decision_context,
            "stream": decision_context["stream"],
            # Frozen snapshot for streaming-fallback safety (R1 P1.2).
            "estimator_claims_snapshot": [_estimator_snapshot],
            "mode": decision_context["mode"],
        }
        # data returned unchanged — NO `spendguard` key on it (P1.5).
        return data

    def _get_stash(self, kwargs: Mapping[str, Any]) -> dict[str, Any] | None:
        """Lookup stash by litellm_call_id WITHOUT popping. Slices 3/5
        pop only AFTER sidecar ACK so retries can find the stash
        (Round 3 P0.6 — pop-on-extract lost retry state)."""
        call_id = kwargs.get("litellm_call_id")
        return self._stash.get(str(call_id)) if call_id else None

    def _pop_stash(self, kwargs: Mapping[str, Any]) -> None:
        """Remove stash entry after sidecar acks the commit/release."""
        call_id = kwargs.get("litellm_call_id")
        if call_id:
            self._stash.pop(str(call_id), None)

    @staticmethod
    def _provider_event_id(response_obj: Any) -> str:
        return str(getattr(response_obj, "id", "") or "")

    async def async_log_success_event(
        self,
        kwargs: dict[str, Any],
        response_obj: Any,
        start_time: Any,
        end_time: Any,
    ) -> None:
        """Commit path. Routes to streaming branch when stash['stream']."""
        stash = self._get_stash(kwargs)
        if stash is None:
            return  # pre-call didn't fire; silent no-op
        if stash["stream"]:
            return await self._async_log_success_streaming(
                stash, kwargs, response_obj,
            )
        if self._client is None:  # Slice 3 R1 P2: fail-closed, not silent
            raise SpendGuardConfigError(
                "stash present but self._client is None; reservation will "
                "TTL-sweep but audit row missing. Should be impossible "
                "after pre-call hook succeeded."
            )

        binding: BudgetBinding = stash["binding"]
        rctx = _build_resolver_ctx(
            user_api_key_dict=kwargs.get("user_api_key_dict"),
            data=kwargs, call_type=kwargs.get("call_type", ""),
        )
        real_claims = self._claim_reconciler(rctx, response_obj)
        if len(real_claims) != 1:
            raise SpendGuardConfigError(
                f"claim_reconciler returned {len(real_claims)} claims; "
                "v1 contract requires exactly 1 (DESIGN.md §6)."
            )
        # Slice 3 R1 P1 + R2 P1 fix: mirror pre-call identity check
        # (budget_id + window_instance_id + unit) at commit time.
        # A bad reconciler returning different budget/window/unit
        # would otherwise commit semantically wrong usage.
        real_claim = real_claims[0]
        _validate_claim_against_binding(real_claim, binding, source="claim_reconciler")

        reservation_ids = stash["reservation_ids"]
        if len(reservation_ids) != 1:
            raise SpendGuardConfigError(
                f"stash has {len(reservation_ids)} reservation_ids; "
                "v1 expects exactly 1 (DESIGN.md §6)."
            )

        real_amount = real_claim.amount_atomic
        try:
            # Slice 3 R1 P0: emit estimated_amount_atomic (the v1
            # CommitEstimated path) — provider_reported_amount_atomic
            # targets the deferred ProviderReport path (Phase 2B Step
            # 8). Existing integrations (langchain, pydantic_ai,
            # openai_agents) all set provider_reported="" + estimated=
            # str(amount). Match that contract.
            await self._client.emit_llm_call_post(
                run_id=stash["run_id"], step_id=stash["step_id"],
                llm_call_id=stash["llm_call_id"],
                decision_id=stash["decision_id"],
                reservation_id=reservation_ids[0],
                provider_reported_amount_atomic="",
                estimated_amount_atomic=str(real_amount),
                unit=binding.unit, pricing=binding.pricing,
                provider_event_id=self._provider_event_id(response_obj),
                outcome="SUCCESS",
            )
        except SpendGuardError:
            if self._fail_open_dev:
                log.warning(
                    "spendguard: commit failed under fail-open; "
                    "reservation will TTL-sweep llm_call_id=%s",
                    stash["llm_call_id"],
                )
                return
            raise
        self._pop_stash(kwargs)

    async def _async_log_success_streaming(
        self,
        stash: dict[str, Any],
        kwargs: dict[str, Any],
        response_obj: Any,
    ) -> None:
        """Slice 4 — end-of-stream commit path.

        Difference from non-streaming:
        - response_obj.usage may be None on some provider/version
          combos. Callback (NOT reconciler) falls back to stashed
          estimator_claims and logs WARNING (R3 P0.7).
        - Commit-boundary sidecar failure is wrapped as
          `SidecarUnavailable` so NF5 typed-exception contract holds
          at the commit boundary (R4 P0.6).
        """
        if self._client is None:  # defensive (same contract as non-stream)
            raise SpendGuardConfigError(
                "stash present but self._client is None; reservation will "
                "TTL-sweep but audit row missing."
            )
        binding: BudgetBinding = stash["binding"]
        rctx = _build_resolver_ctx(
            user_api_key_dict=kwargs.get("user_api_key_dict"),
            data=kwargs, call_type=kwargs.get("call_type", ""),
        )
        usage = getattr(response_obj, "usage", None)
        if usage is None:
            # R3 P0.7: estimator-fallback degraded path. Not F7
            # acceptance; logged loudly + commits estimator amount.
            log.warning(
                "spendguard: streaming response has no .usage frame; "
                "falling back to estimator value for llm_call_id=%s "
                "(degraded path; F7 acceptance requires reconciled usage).",
                stash["llm_call_id"],
            )
            real_claims = stash["estimator_claims_snapshot"]
        else:
            real_claims = self._claim_reconciler(rctx, response_obj)
        if len(real_claims) != 1:
            raise SpendGuardConfigError(
                f"streaming reconciler returned {len(real_claims)} "
                "claims; v1 contract requires exactly 1 (DESIGN.md §6)."
            )
        real_claim = real_claims[0]
        _validate_claim_against_binding(
            real_claim, binding, source="streaming claim_reconciler",
        )

        reservation_ids = stash["reservation_ids"]
        if len(reservation_ids) != 1:
            raise SpendGuardConfigError(
                f"stash has {len(reservation_ids)} reservation_ids; "
                "v1 expects exactly 1 (DESIGN.md §6)."
            )

        try:
            await self._client.emit_llm_call_post(
                run_id=stash["run_id"], step_id=stash["step_id"],
                llm_call_id=stash["llm_call_id"],
                decision_id=stash["decision_id"],
                reservation_id=reservation_ids[0],
                provider_reported_amount_atomic="",
                estimated_amount_atomic=str(real_claim.amount_atomic),
                unit=binding.unit, pricing=binding.pricing,
                provider_event_id=self._provider_event_id(response_obj),
                outcome="SUCCESS",
            )
        except SidecarUnavailable as exc:
            # Slice 4 R1 P1.1 fix: only ACTUAL availability failures
            # are wrapped (NF5 contract). Semantic errors (Denied,
            # ConfigError) propagate as-is so callers can distinguish
            # transport problems from invariant violations.
            if self._fail_open_dev:
                log.warning(
                    "spendguard: streaming commit unavailable under "
                    "fail-open; reservation will TTL-sweep "
                    "llm_call_id=%s err=%r", stash["llm_call_id"], exc,
                )
                return
            raise SidecarUnavailable(
                f"sidecar unavailable at streaming commit boundary: {exc}"
            ) from exc
        except SpendGuardError as exc:
            # Semantic error (e.g. invariant rejection, config). Don't
            # mask as availability — keep stash for retry visibility.
            if self._fail_open_dev:
                log.warning(
                    "spendguard: streaming commit semantic error under "
                    "fail-open; reservation will TTL-sweep "
                    "llm_call_id=%s err=%r", stash["llm_call_id"], exc,
                )
                return
            raise
        self._pop_stash(kwargs)

    async def async_log_failure_event(
        self,
        kwargs: dict[str, Any],
        response_obj: Any,
        start_time: Any,
        end_time: Any,
    ) -> None:
        """Slice 5 — release reservation on provider failure / cancel.

        LiteLLM fires this on provider exception, internal cancel, and
        between retry attempts (ADR-002: each attempt has a distinct
        `litellm_call_id` → distinct `decision_id` → distinct
        reservation; pre-call reserves, this hook releases). Release
        errors are SWALLOWED so we never mask the original LiteLLM
        exception that triggered the callback — TTL sweep is the
        durable backstop (FAILURE_MODES.md).
        """
        stash = self._get_stash(kwargs)
        if stash is None:
            return  # pre-call didn't fire — nothing to release
        if self._client is None:  # defensive, mirrors success branch
            log.warning(
                "spendguard: failure event with stash but no client; "
                "reservation will TTL-sweep llm_call_id=%s",
                stash["llm_call_id"],
            )
            return
        binding: BudgetBinding = stash["binding"]
        outcome = self._classify_failure(
            kwargs.get("exception") or response_obj,
        )
        reservation_ids = stash["reservation_ids"]
        if len(reservation_ids) != 1:
            log.warning(
                "spendguard: failure-event has %d reservations; "
                "v1 expects 1 — releasing first only "
                "(others will TTL-sweep).",
                len(reservation_ids),
            )
        if not reservation_ids:
            self._pop_stash(kwargs)
            return
        try:
            await self._client.emit_llm_call_post(
                run_id=stash["run_id"], step_id=stash["step_id"],
                llm_call_id=stash["llm_call_id"],
                decision_id=stash["decision_id"],
                reservation_id=reservation_ids[0],
                provider_reported_amount_atomic="0",
                estimated_amount_atomic="0",
                unit=binding.unit, pricing=binding.pricing,
                provider_event_id=self._provider_event_id(response_obj),
                outcome=outcome,
            )
        except SpendGuardError as exc:
            log.warning(
                "spendguard: release RPC failed for llm_call_id=%s "
                "outcome=%s err=%r; reservation will TTL-sweep "
                "(keeping stash for retry visibility).",
                stash["llm_call_id"], outcome, exc,
            )
            return  # keep stash; do NOT re-raise (mask original error)
        self._pop_stash(kwargs)

    # Boundary on either side must be start/end of string OR a
    # non-letter — `\b` alone is too loose because `_` is a word
    # character (so `operation_canceled` would NOT match with `\b`).
    _CANCELLED_TOKEN_RE = _re.compile(
        r"(?:^|[^A-Za-z])cancell?ed(?:$|[^A-Za-z])", _re.IGNORECASE,
    )

    @classmethod
    def _classify_failure(cls, exception: Any) -> str:
        """`CancelledError` → CANCELLED; everything else → FAILURE.

        Some LiteLLM versions deliver `kwargs["exception"]` as a string
        repr instead of the actual exception object; the string branch
        defends against that (spec §Slice-5 code skeleton).

        Slice 5 R1 P2 hardening: use a word-boundary regex
        (`\bcancell?ed\b`) instead of a naive substring `in` check so
        provider messages like "uncancelled" or "cancellation_not_allowed"
        don't get misclassified as CANCELLED. Accepts both "cancelled"
        (British) and "canceled" (American) spellings since both appear
        in real provider error messages.
        """
        if isinstance(exception, asyncio.CancelledError):
            return "CANCELLED"
        if isinstance(exception, str) and cls._CANCELLED_TOKEN_RE.search(
            exception,
        ):
            return "CANCELLED"
        return "FAILURE"

    # NO log_pre_api_call override — verified ineffective (Slice 1 R2).
    # Sync direct callers route to Shape A egress (DESIGN §3.4 v1 Path A).


class _LoopBoundCallback(SpendGuardLiteLLMCallback):
    """Lazy-init wrapper that binds SpendGuardClient to LiteLLM's
    serving event loop (Round 3 P0.3). gRPC/UDS channels are loop-
    affine; the LiteLLM proxy imports modules sync at boot then runs
    its own ASGI loop."""

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

    # Slice 2 R1 P0.2 fix: absolute deadline + no sleep after final
    # attempt. Earlier loop slept post-final and didn't bound the
    # per-attempt handshake duration, blowing well past 3.1s in the
    # worst case. ENSURE_CLIENT_DEADLINE_S is the hard upper bound on
    # the lazy init; if exceeded, surface SidecarUnavailable.
    _ENSURE_CLIENT_DEADLINE_S = 5.0  # generous; covers 5 attempts + retries
    _ENSURE_CLIENT_ATTEMPT_TIMEOUT_S = 1.0  # per-attempt handshake cap
    _ENSURE_CLIENT_MAX_ATTEMPTS = 5  # cap retries even if deadline allows more

    async def _ensure_client(self) -> SpendGuardClient:
        if self._client is not None:
            return self._client
        if self._init_lock is None:
            self._init_lock = asyncio.Lock()
        async with self._init_lock:
            if self._client is not None:
                return self._client
            loop = asyncio.get_running_loop()
            deadline = loop.time() + self._ENSURE_CLIENT_DEADLINE_S
            c = SpendGuardClient(
                socket_path=self._socket_path,
                tenant_id=self._tenant_id,
            )
            last_exc: Exception | None = None
            attempt = 0
            while attempt < self._ENSURE_CLIENT_MAX_ATTEMPTS:
                remaining = deadline - loop.time()
                if remaining <= 0:
                    break  # hard deadline already breached
                attempt += 1
                # Slice 2 R2 P1.2 fix: recompute remaining BEFORE each
                # awaited op; bound timeout by min(attempt_timeout,
                # remaining) so a final attempt with <1s budget cannot
                # blow past the deadline.
                try:
                    connect_timeout = min(
                        self._ENSURE_CLIENT_ATTEMPT_TIMEOUT_S, remaining,
                    )
                    await asyncio.wait_for(c.connect(), timeout=connect_timeout)
                    # Re-check deadline before second await.
                    remaining = deadline - loop.time()
                    if remaining <= 0:
                        last_exc = SidecarUnavailable(
                            "deadline expired between connect and handshake"
                        )
                        break
                    handshake_timeout = min(
                        self._ENSURE_CLIENT_ATTEMPT_TIMEOUT_S, remaining,
                    )
                    await asyncio.wait_for(
                        c.handshake(), timeout=handshake_timeout,
                    )
                    self._client = c
                    return c
                except Exception as exc:  # noqa: BLE001
                    last_exc = exc
                # No sleep AFTER the final attempt or near-deadline.
                if attempt >= self._ENSURE_CLIENT_MAX_ATTEMPTS:
                    break
                backoff = min(0.1 * (2 ** (attempt - 1)), 1.0)
                remaining = deadline - loop.time()
                if remaining <= backoff:
                    break
                await asyncio.sleep(backoff)
            raise SidecarUnavailable(
                f"sidecar handshake failed within "
                f"{self._ENSURE_CLIENT_DEADLINE_S}s deadline "
                f"({attempt} attempts): {last_exc}"
            ) from last_exc

    async def async_pre_call_hook(self, *a: Any, **kw: Any) -> dict[str, Any] | None:
        await self._ensure_client()
        return await super().async_pre_call_hook(*a, **kw)

    async def async_log_success_event(self, *a: Any, **kw: Any) -> None:
        await self._ensure_client()
        await super().async_log_success_event(*a, **kw)

    async def async_log_failure_event(self, *a: Any, **kw: Any) -> None:
        await self._ensure_client()
        await super().async_log_failure_event(*a, **kw)


# install() factory REMOVED at pivot — direct litellm.callbacks=[...] was
# verified ineffective (Slice 1 R2). Proxy users instantiate
# _LoopBoundCallback in proxy_config.yaml; direct callers use Shape A
# (set litellm.api_base = "http://localhost:9000/v1").


# ---------------------------------------------------------------------------
# Direct async wrapper (Slice A1) — `acompletion()` for non-proxy callers.
#
# Operators who run `await litellm.acompletion(...)` directly (no LiteLLM
# proxy in the loop) use this wrapper to get reserve→commit gating with the
# same failure-mode contract as the proxy callback path. ADR-005 stands:
# sync `litellm.completion()` is NOT supported here because LiteLLM's logging
# dispatcher swallows exceptions from sync hooks — sync callers must route
# via Shape A egress proxy (DESIGN §3.4 v1 Path A).
# ---------------------------------------------------------------------------


class SpendGuardDirectAcompletion:
    """Drop-in async wrapper for `litellm.acompletion()` with budget gating.

    Configure once at module init; call `await wrapper(...)` per request.
    Internally:
      1. Stamps a `litellm_call_id` if the caller did not.
      2. Resolves binding + estimates the worst-case claim.
      3. Reserves via `client.request_decision(...)`.
      4. Delegates to `litellm.acompletion(**kwargs)`.
      5. On success → `emit_llm_call_post(outcome=SUCCESS)` with the
         reconciler's real amount.
      6. On exception → best-effort `emit_llm_call_post(outcome=FAILURE)`
         release, then re-raises the original LiteLLM exception. SpendGuard
         release errors are swallowed (TTL sweep is the durable backstop).

    Fail-closed by default: `DENY`/`DEGRADE` from sidecar → `DecisionDenied`/
    `SidecarUnavailable` raised; provider call is NOT attempted. Operators
    who set `SPENDGUARD_LITELLM_FAIL_OPEN=1` for dev get a warning + the
    provider call still runs (no audit row).
    """

    def __init__(
        self,
        *,
        client: SpendGuardClient,
        budget_resolver: BudgetResolver,
        claim_estimator: ClaimEstimator,
        claim_reconciler: ClaimReconciler,
    ) -> None:
        self._client = client
        self._budget_resolver = budget_resolver
        self._claim_estimator = claim_estimator
        self._claim_reconciler = claim_reconciler
        self._fail_open_dev = (
            os.environ.get("SPENDGUARD_LITELLM_FAIL_OPEN", "").strip() == "1"
        )

    async def __call__(self, **litellm_kwargs: Any) -> Any:
        import litellm  # local import — keeps the SDK lightweight at import
        litellm_call_id = str(
            litellm_kwargs.get("litellm_call_id")
            or derive_uuid_from_signature(
                f"direct:{id(litellm_kwargs)}:{time.time_ns()}",
                scope="litellm_call_id",
            ),
        )
        litellm_kwargs["litellm_call_id"] = litellm_call_id
        data: dict[str, Any] = dict(litellm_kwargs)
        rctx = _build_resolver_ctx(
            user_api_key_dict=None, data=data,
            call_type=str(data.get("call_type", "acompletion")),
        )
        binding = self._budget_resolver(rctx)
        if binding is None or not binding.budget_id or not binding.window_instance_id:
            raise SpendGuardConfigError(
                "budget_resolver returned None/empty binding (DESIGN ADR-001)"
            )
        estimator_claims = self._claim_estimator(rctx)
        if len(estimator_claims) != 1:
            raise SpendGuardConfigError(
                f"claim_estimator returned {len(estimator_claims)} claims; "
                "v1 contract requires exactly 1",
            )
        _validate_claim_against_binding(
            estimator_claims[0], binding, source="claim_estimator",
        )
        llm_call_id = str(derive_uuid_from_signature(
            f"litellm-direct:{litellm_call_id}", scope="llm_call_id"))
        decision_id = str(derive_uuid_from_signature(
            f"litellm-direct:{litellm_call_id}", scope="decision_id"))
        ctx_obj = current_run_context()
        run_id = (ctx_obj.run_id if ctx_obj else str(
            derive_uuid_from_signature(
                f"litellm-direct:{litellm_call_id}", scope="run_id")))
        step_id = (ctx_obj.step_id if ctx_obj and ctx_obj.step_id
                   else f"litellm-direct:{litellm_call_id[:16]}")
        prompt_hash = compute_prompt_hash(
            _serialize_messages_for_hash(data.get("messages")),
            self._client.tenant_id,
        )
        decision_context = _build_decision_context(
            ctx=rctx, binding=binding, litellm_call_id=litellm_call_id,
            prompt_hash=prompt_hash,
        )
        decision_context["mode"] = "direct"  # vs the proxy-callback path
        idempotency_key = derive_idempotency_key(
            tenant_id=self._client.tenant_id,
            session_id=self._client.session_id,
            run_id=run_id, step_id=step_id, llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )
        try:
            outcome = await self._client.request_decision(
                trigger="LLM_CALL_PRE",
                run_id=run_id, step_id=step_id, llm_call_id=llm_call_id,
                tool_call_id="", decision_id=decision_id, route="llm.call",
                projected_claims=estimator_claims,
                idempotency_key=idempotency_key,
                projected_unit=binding.unit,
                decision_context_json=decision_context,
            )
        except DecisionDenied:
            raise
        except SpendGuardError as exc:
            if self._fail_open_dev:
                log.warning(
                    "spendguard: SPENDGUARD_LITELLM_FAIL_OPEN=1 — direct "
                    "acompletion bypassing sidecar error %r (DEV ONLY).", exc,
                )
                return await litellm.acompletion(**litellm_kwargs)
            raise SidecarUnavailable(
                f"sidecar pre-call failed (direct mode): {exc}",
            ) from exc
        if getattr(outcome, "decision", "") == "DEGRADE":
            if self._fail_open_dev:
                log.warning(
                    "spendguard: DEGRADE under fail-open (direct mode); "
                    "allowing call (DEV ONLY).",
                )
                return await litellm.acompletion(**litellm_kwargs)
            raise SidecarUnavailable(
                "sidecar returned DEGRADE; direct mode fails closed.",
            )
        if len(outcome.reservation_ids) != 1:
            raise SpendGuardConfigError(
                f"sidecar returned {len(outcome.reservation_ids)} reservations; "
                "v1 expects exactly 1",
            )
        reservation_id = outcome.reservation_ids[0]
        # Delegate to LiteLLM. Failures bubble through finally → release.
        try:
            response = await litellm.acompletion(**litellm_kwargs)
        except Exception as call_exc:
            # Best-effort release; SpendGuardError swallowed, others bubble
            # alongside the original exception via chained traceback.
            try:
                await self._client.emit_llm_call_post(
                    run_id=run_id, step_id=step_id, llm_call_id=llm_call_id,
                    decision_id=outcome.decision_id,
                    reservation_id=reservation_id,
                    provider_reported_amount_atomic="0",
                    estimated_amount_atomic="0",
                    unit=binding.unit, pricing=binding.pricing,
                    provider_event_id="",
                    outcome=(
                        "CANCELLED"
                        if isinstance(call_exc, asyncio.CancelledError)
                        else "FAILURE"
                    ),
                )
            except SpendGuardError as rel_exc:
                log.warning(
                    "spendguard: direct release RPC failed for llm_call_id=%s "
                    "err=%r; reservation will TTL-sweep.",
                    llm_call_id, rel_exc,
                )
            raise
        real_claims = self._claim_reconciler(rctx, response)
        if len(real_claims) != 1:
            raise SpendGuardConfigError(
                f"claim_reconciler returned {len(real_claims)} claims; "
                "v1 contract requires exactly 1",
            )
        _validate_claim_against_binding(
            real_claims[0], binding, source="claim_reconciler",
        )
        await self._client.emit_llm_call_post(
            run_id=run_id, step_id=step_id, llm_call_id=llm_call_id,
            decision_id=outcome.decision_id,
            reservation_id=reservation_id,
            provider_reported_amount_atomic="",
            estimated_amount_atomic=str(real_claims[0].amount_atomic),
            unit=binding.unit, pricing=binding.pricing,
            provider_event_id=str(getattr(response, "id", "") or ""),
            outcome="SUCCESS",
        )
        return response


__all__ = [
    "BudgetBinding",
    "BudgetResolver",
    "ClaimEstimator",
    "ClaimReconciler",
    "LiteLLMRunContext",
    "ResolverContext",
    "SpendGuardDirectAcompletion",
    "SpendGuardLiteLLMCallback",
    "_LoopBoundCallback",
    "current_run_context",
    "run_context",
]
