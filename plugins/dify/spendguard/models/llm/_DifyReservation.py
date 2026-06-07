"""Reservation/commit/release delegate for the Dify Model Provider plugin.

Mirrors the contract of
``sdk/python/src/spendguard/integrations/litellm.py::SpendGuardLiteLLMCallback``
(reserve in ``async_pre_call_hook`` -> commit in ``async_log_success_event``
-> release in ``async_log_failure_event``), translated to the Dify SDK's
synchronous ``_invoke`` signature.

Composition over inheritance: the Dify SDK base class
(``LargeLanguageModel``) and the SpendGuard reservation lifecycle are
orthogonal state machines. ``_DifyReservation`` owns the lifecycle;
``SpendGuardLLM`` only adapts the Dify SDK signature.

Slice 3 acceptance gates (review-standards.md §3):
- 3.1 composition-only (no ``LargeLanguageModel`` inheritance here).
- 3.2 ``__init__`` reads ``SPENDGUARD_SIDECAR_UDS`` + ``SPENDGUARD_TENANT_ID``;
  missing -> ``SpendGuardConfigError`` naming the var.
- 3.3 ``_ensure_client`` follows the LiteLLM ``_LoopBoundCallback`` pattern
  (5s deadline, 1s per-attempt cap, deadline-bounded backoff).
- 3.4 ``reserve`` builds + validates ``BudgetBinding`` via
  ``_validate_claim_against_binding`` (mirrors litellm.py:149-191).
- 3.5 ``commit_success`` passes ``estimated_amount_atomic=str(real_amount)``
  + ``provider_reported_amount_atomic=""``.
- 3.6 ``release_failure`` swallows release-RPC errors (TTL sweep is the
  durable backstop) but logs WARN.
- 3.7 ``release_failure`` classifies ``asyncio.CancelledError`` ->
  ``"CANCELLED"``.
"""

from __future__ import annotations

import asyncio
import json
import logging
import os
import re
from collections.abc import Mapping
from dataclasses import dataclass
from types import SimpleNamespace
from typing import Any

from spendguard.client import SpendGuardClient
from spendguard.errors import (
    DecisionDenied,
    SidecarUnavailable,
    SpendGuardConfigError,
    SpendGuardError,
)
from spendguard.ids import derive_idempotency_key, derive_uuid_from_signature
from spendguard.prompt_hash import compute as compute_prompt_hash

log = logging.getLogger("spendguard.dify_plugin.reservation")


# ---------------------------------------------------------------------------
# Public data carriers
# ---------------------------------------------------------------------------

@dataclass(frozen=True, slots=True)
class BudgetBinding:
    """Per-call budget binding: which budget/window/unit/pricing to use.

    Derived from Dify ``credentials`` (operator-supplied via the provider
    form). Mirrors the LiteLLM integration's ``BudgetBinding`` shape so
    estimator/reconciler claims validate against the same identity
    fields (budget_id + window_instance_id + unit.unit_id).
    """
    budget_id: str
    window_instance_id: str
    unit: Any        # common_pb2.UnitRef (duck-typed in tests)
    pricing: Any     # common_pb2.PricingFreeze (duck-typed in tests)


@dataclass(frozen=True, slots=True)
class DifyCallContext:
    """Inputs the reservation sees per Dify ``_invoke`` call."""
    workspace_id: str
    app_id: str | None
    model: str
    prompt_messages: list[Any]
    stream: bool
    credentials: Mapping[str, Any]
    user: str | None = None


@dataclass(frozen=True, slots=True)
class ReservationHandle:
    """State carried from ``reserve`` -> commit/release for one call.

    Frozen + slotted so it can travel safely across the sync/async bridge
    that ``SpendGuardLLM._invoke`` straddles.
    """
    decision_id: str
    reservation_id: str
    llm_call_id: str
    run_id: str
    step_id: str
    binding: BudgetBinding
    estimator_snapshot: Any
    stream: bool


# ---------------------------------------------------------------------------
# Internal helpers
# ---------------------------------------------------------------------------

def _validate_claim_against_binding(
    claim: Any, binding: BudgetBinding, *, source: str,
) -> None:
    """Validate a BudgetClaim's identity matches the binding.

    Mirrors ``sdk/python/src/spendguard/integrations/litellm.py``
    ``_validate_claim_against_binding`` (lines 149-191). Empty fields
    are rejected: an empty unit_id would silently commit amount under
    the binding's unit semantics (mis-charge).
    """
    claim_budget_id = getattr(claim, "budget_id", None) or ""
    claim_window = getattr(claim, "window_instance_id", None) or ""
    if claim_budget_id != binding.budget_id:
        raise SpendGuardConfigError(
            f"{source} returned budget_id={claim_budget_id!r} but binding "
            f"has budget_id={binding.budget_id!r}. Audit context would "
            "mis-charge."
        )
    if claim_window != binding.window_instance_id:
        raise SpendGuardConfigError(
            f"{source} returned window_instance_id={claim_window!r} but "
            f"binding has window_instance_id="
            f"{binding.window_instance_id!r}."
        )
    binding_unit = getattr(binding, "unit", None)
    binding_unit_id = getattr(binding_unit, "unit_id", None) or ""
    if not binding_unit_id:
        raise SpendGuardConfigError(
            "BudgetBinding.unit.unit_id is empty; resolver MUST yield a "
            "non-empty unit."
        )
    claim_unit = getattr(claim, "unit", None)
    claim_unit_id = getattr(claim_unit, "unit_id", None) or ""
    if claim_unit_id != binding_unit_id:
        raise SpendGuardConfigError(
            f"{source} returned unit.unit_id={claim_unit_id!r} but binding "
            f"has unit.unit_id={binding_unit_id!r}. Amount would be "
            "committed under wrong unit semantics."
        )


def _serialize_messages_for_hash(prompt_messages: Any) -> str:
    """Stable canonical-JSON of Dify prompt_messages for prompt_hash input.

    Dify prompt_messages are pydantic ``PromptMessage`` instances; fall
    back to ``str()`` when ``model_dump_json`` is not available (test
    duck-typed inputs).
    """
    if prompt_messages is None:
        return ""
    try:
        rows: list[Any] = []
        for msg in prompt_messages:
            if hasattr(msg, "model_dump"):
                rows.append(msg.model_dump(exclude_none=True))
            elif isinstance(msg, dict):
                rows.append(msg)
            else:
                rows.append(repr(msg))
        return json.dumps(rows, sort_keys=True, separators=(",", ":"))
    except (TypeError, ValueError):
        return repr(prompt_messages)


def _build_binding_from_credentials(
    credentials: Mapping[str, Any],
) -> BudgetBinding:
    """Translate Dify ``credentials`` dict into a BudgetBinding.

    The fields are operator-supplied via the provider form
    (``provider/spendguard.yaml``). Missing fields raise
    ``SpendGuardConfigError`` with the offending key named (review-
    standards.md cross-cutting "error messages" row).
    """
    required_keys = (
        "spendguard_budget_id",
        "spendguard_window_instance_id",
    )
    for key in required_keys:
        if not str(credentials.get(key) or "").strip():
            raise SpendGuardConfigError(
                f"credentials.{key} is missing or empty; configure it on "
                "the Dify provider form."
            )
    # The token-kind defaults to output_token; the unit is the v1 atomic
    # unit (DESIGN.md §6). Operators can override via the resolver but
    # the v1 plugin takes a sensible default to keep the form short.
    unit = SimpleNamespace(
        unit_id="atomic.usd.micro",
        token_kind="output_token",
    )
    pricing = SimpleNamespace(
        pricing_version="v1",
        price_snapshot_hash_hex="",
        fx_rate_version="v1",
        unit_conversion_version="v1",
    )
    return BudgetBinding(
        budget_id=str(credentials["spendguard_budget_id"]),
        window_instance_id=str(credentials["spendguard_window_instance_id"]),
        unit=unit,
        pricing=pricing,
    )


def _build_default_claim(
    *, binding: BudgetBinding, estimated_amount_atomic: str,
) -> SimpleNamespace:
    """Build a single BudgetClaim duck-typed for the SDK + tests.

    SDK expects ``common_pb2.BudgetClaim`` proto messages. For the v1
    plugin the structural fields (budget_id, window_instance_id,
    amount_atomic, unit) are what matter — the sidecar's binding
    validator only reads those. Duck-typing here keeps the v1 plugin
    SDK-version-independent.
    """
    return SimpleNamespace(
        budget_id=binding.budget_id,
        window_instance_id=binding.window_instance_id,
        amount_atomic=str(estimated_amount_atomic),
        unit=SimpleNamespace(unit_id=binding.unit.unit_id),
    )


# ---------------------------------------------------------------------------
# CANCELLED classification (mirrors litellm.py:735-760)
# ---------------------------------------------------------------------------

# Word-boundary regex; ``\b`` alone is too loose because ``_`` is a word
# character. Accepts both "cancelled" (British) and "canceled" (American).
_CANCELLED_TOKEN_RE = re.compile(
    r"(?:^|[^A-Za-z])cancell?ed(?:$|[^A-Za-z])", re.IGNORECASE,
)


def _classify_failure(exc: Any) -> str:
    """``CancelledError`` -> CANCELLED; everything else -> FAILURE.

    Some adapter layers may deliver the exception as a string repr; the
    string branch defends against that (mirrors LiteLLM contract).
    """
    if isinstance(exc, asyncio.CancelledError):
        return "CANCELLED"
    if isinstance(exc, str) and _CANCELLED_TOKEN_RE.search(exc):
        return "CANCELLED"
    return "FAILURE"


# ---------------------------------------------------------------------------
# Reservation delegate
# ---------------------------------------------------------------------------

class _DifyReservation:
    """Reservation lifecycle delegate. Composition surface only — does NOT
    inherit from LargeLanguageModel (review-standards.md 3.1)."""

    # Mirrors _LoopBoundCallback constants in
    # sdk/python/src/spendguard/integrations/litellm.py:800-802.
    _ENSURE_CLIENT_DEADLINE_S = 5.0
    _ENSURE_CLIENT_ATTEMPT_TIMEOUT_S = 1.0
    _ENSURE_CLIENT_MAX_ATTEMPTS = 5

    def __init__(
        self,
        *,
        socket_path: str | None = None,
        tenant_id: str | None = None,
    ) -> None:
        # review-standards.md 3.2: missing env vars -> SpendGuardConfigError
        # that names the offending var.
        resolved_socket = socket_path or os.environ.get(
            "SPENDGUARD_SIDECAR_UDS", "",
        ).strip()
        resolved_tenant = tenant_id or os.environ.get(
            "SPENDGUARD_TENANT_ID", "",
        ).strip()
        if not resolved_socket:
            raise SpendGuardConfigError(
                "SPENDGUARD_SIDECAR_UDS is missing; configure it on the "
                "plugin daemon container env."
            )
        if not resolved_tenant:
            raise SpendGuardConfigError(
                "SPENDGUARD_TENANT_ID is missing; configure it on the "
                "plugin daemon container env."
            )
        self._socket_path = resolved_socket
        self._tenant_id = resolved_tenant
        self._client: SpendGuardClient | None = None
        self._init_lock: asyncio.Lock | None = None
        # NO module-level mutable state (review-standards.md 3.8). The
        # fail-open env var is captured once at construction so log spam
        # at every call doesn't fire.
        self._fail_open_dev: bool = (
            os.environ.get("SPENDGUARD_DIFY_FAIL_OPEN", "").strip() == "1"
        )
        if self._fail_open_dev:
            log.warning(
                "spendguard: SPENDGUARD_DIFY_FAIL_OPEN=1 — fail-open mode "
                "active; sidecar errors will allow LLM calls. DEV ONLY.",
            )

    # ------------------------------------------------------------------
    # Lazy client init (review-standards.md 3.3 — mirrors litellm.py:804-863)
    # ------------------------------------------------------------------

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
            client = SpendGuardClient(
                socket_path=self._socket_path,
                tenant_id=self._tenant_id,
                runtime_kind="dify-plugin",
                runtime_version="0.1.0",
                sdk_version="0.5.1",
            )
            last_exc: Exception | None = None
            attempt = 0
            while attempt < self._ENSURE_CLIENT_MAX_ATTEMPTS:
                remaining = deadline - loop.time()
                if remaining <= 0:
                    break
                attempt += 1
                try:
                    connect_timeout = min(
                        self._ENSURE_CLIENT_ATTEMPT_TIMEOUT_S, remaining,
                    )
                    await asyncio.wait_for(
                        client.connect(), timeout=connect_timeout,
                    )
                    remaining = deadline - loop.time()
                    if remaining <= 0:
                        last_exc = SidecarUnavailable(
                            "deadline expired between connect and handshake",
                        )
                        break
                    handshake_timeout = min(
                        self._ENSURE_CLIENT_ATTEMPT_TIMEOUT_S, remaining,
                    )
                    await asyncio.wait_for(
                        client.handshake(), timeout=handshake_timeout,
                    )
                    self._client = client
                    return client
                except Exception as exc:
                    last_exc = exc
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

    # ------------------------------------------------------------------
    # Reserve (review-standards.md 3.4)
    # ------------------------------------------------------------------

    async def reserve(
        self,
        ctx: DifyCallContext,
        *,
        estimated_amount_atomic: str = "1000",
    ) -> ReservationHandle:
        """Build binding + claim, request decision, return a handle.

        Raises:
          DecisionDenied   on DENY.
          SidecarUnavailable on DEGRADE (unless fail-open env set).
          SpendGuardConfigError on resolver/binding/claim validation failures.
        """
        binding = _build_binding_from_credentials(ctx.credentials)

        # Idempotency key derivation mirrors litellm.py:339-344.
        # The Dify SDK doesn't stamp a stable call ID across retries, so
        # we synthesise one from the workspace+app+model+prompt-hash
        # tuple — same logical call -> same key -> sidecar cache hit.
        prompt_hash = compute_prompt_hash(
            _serialize_messages_for_hash(ctx.prompt_messages),
            self._tenant_id,
        )
        signature = (
            f"dify:{ctx.workspace_id}:{ctx.app_id or ''}:{ctx.model}:"
            f"{prompt_hash}"
        )
        llm_call_id = str(derive_uuid_from_signature(
            signature, scope="llm_call_id",
        ))
        decision_id = str(derive_uuid_from_signature(
            signature, scope="decision_id",
        ))
        run_id = str(derive_uuid_from_signature(
            signature, scope="run_id",
        ))
        step_id = f"dify:{llm_call_id[:16]}"

        client = await self._ensure_client()
        idempotency_key = derive_idempotency_key(
            tenant_id=client.tenant_id,
            session_id=client.session_id,
            run_id=run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )

        claim = _build_default_claim(
            binding=binding, estimated_amount_atomic=estimated_amount_atomic,
        )
        _validate_claim_against_binding(claim, binding, source="claim_estimator")

        # Build decision_context for canonical_events enrichment
        # (same shape as litellm.py:120-146).
        decision_context = {
            "integration": "dify-plugin",
            "dify_workspace_id": ctx.workspace_id,
            "dify_app_id": ctx.app_id or "",
            "model": ctx.model,
            "prompt_hash": prompt_hash,
            "stream": bool(ctx.stream),
            "mode": "plugin",
        }

        try:
            outcome = await client.request_decision(
                trigger="LLM_CALL_PRE",
                run_id=run_id,
                step_id=step_id,
                llm_call_id=llm_call_id,
                tool_call_id="",
                decision_id=decision_id,
                route="llm.call",
                projected_claims=[claim],
                idempotency_key=idempotency_key,
                projected_unit=binding.unit,
                decision_context_json=decision_context,
            )
        except DecisionDenied:
            raise
        except SpendGuardError as exc:
            if self._fail_open_dev:
                log.warning(
                    "spendguard: fail-open allowing call despite sidecar "
                    "error %r (DEV ONLY).", exc,
                )
                # Return a sentinel handle so the caller can still proceed
                # under fail-open. Commit/release are no-ops in this path.
                return ReservationHandle(
                    decision_id=decision_id,
                    reservation_id="",
                    llm_call_id=llm_call_id,
                    run_id=run_id,
                    step_id=step_id,
                    binding=binding,
                    estimator_snapshot=claim,
                    stream=ctx.stream,
                )
            raise SidecarUnavailable(
                f"sidecar pre-call failed: {exc}",
            ) from exc

        # DEGRADE -> fail-closed unless fail-open env set
        # (review-standards.md cross-cutting; mirrors litellm.py:418-429).
        decision_str = getattr(outcome, "decision", "")
        if decision_str == "DEGRADE":
            if self._fail_open_dev:
                log.warning(
                    "spendguard: DEGRADE under fail-open; allowing call "
                    "(DEV ONLY).",
                )
                return ReservationHandle(
                    decision_id=getattr(outcome, "decision_id", decision_id),
                    reservation_id="",
                    llm_call_id=llm_call_id,
                    run_id=run_id,
                    step_id=step_id,
                    binding=binding,
                    estimator_snapshot=claim,
                    stream=ctx.stream,
                )
            raise SidecarUnavailable(
                "sidecar returned DEGRADE (ledger or dependent service "
                "unavailable); Dify plugin fails closed on DEGRADE.",
            )

        reservation_ids = tuple(outcome.reservation_ids)
        if len(reservation_ids) != 1:
            # Defensive release of unexpected multi-reservation outcomes
            # (mirrors litellm.py:436-460).
            for rid in reservation_ids:
                try:
                    await client.emit_llm_call_post(
                        run_id=run_id,
                        step_id=step_id,
                        llm_call_id=llm_call_id,
                        decision_id=outcome.decision_id,
                        reservation_id=rid,
                        provider_reported_amount_atomic="0",
                        unit=binding.unit,
                        pricing=binding.pricing,
                        provider_event_id="",
                        outcome="FAILURE",
                    )
                except Exception as rel_exc:
                    log.warning(
                        "spendguard: best-effort release of reservation %s "
                        "failed: %r", rid, rel_exc,
                    )
            raise SpendGuardConfigError(
                f"sidecar returned {len(reservation_ids)} reservations; "
                "v1 expects exactly 1. Failing closed before upstream HTTP.",
            )

        return ReservationHandle(
            decision_id=outcome.decision_id,
            reservation_id=reservation_ids[0],
            llm_call_id=llm_call_id,
            run_id=run_id,
            step_id=step_id,
            binding=binding,
            estimator_snapshot=claim,
            stream=ctx.stream,
        )

    # ------------------------------------------------------------------
    # Commit (review-standards.md 3.5)
    # ------------------------------------------------------------------

    async def commit_success(
        self,
        handle: ReservationHandle,
        *,
        real_amount_atomic: str,
        provider_event_id: str = "",
        actual_input_tokens: int | None = None,
        actual_output_tokens: int | None = None,
    ) -> None:
        """Commit reservation with the real usage amount.

        Mirrors litellm.py:550-560 — emits ``estimated_amount_atomic`` +
        empty ``provider_reported_amount_atomic`` (the v1 CommitEstimated
        path). Actual token counts are passed through so the audit row
        carries delta_b / delta_c ratios.
        """
        if not handle.reservation_id:
            # Fail-open sentinel handle; nothing to commit.
            return
        client = await self._ensure_client()
        try:
            await client.emit_llm_call_post(
                run_id=handle.run_id,
                step_id=handle.step_id,
                llm_call_id=handle.llm_call_id,
                decision_id=handle.decision_id,
                reservation_id=handle.reservation_id,
                provider_reported_amount_atomic="",
                estimated_amount_atomic=str(real_amount_atomic),
                unit=handle.binding.unit,
                pricing=handle.binding.pricing,
                provider_event_id=provider_event_id,
                outcome="SUCCESS",
                actual_input_tokens=actual_input_tokens,
                actual_output_tokens=actual_output_tokens,
            )
        except SpendGuardError as exc:
            if self._fail_open_dev:
                log.warning(
                    "spendguard: commit failed under fail-open; reservation "
                    "will TTL-sweep llm_call_id=%s err=%r",
                    handle.llm_call_id, exc,
                )
                return
            raise

    # ------------------------------------------------------------------
    # Release (review-standards.md 3.6 + 3.7)
    # ------------------------------------------------------------------

    async def release_failure(
        self,
        handle: ReservationHandle,
        exc: BaseException | str,
        *,
        provider_event_id: str = "",
    ) -> None:
        """Release reservation on failure / cancellation.

        Release-RPC errors are SWALLOWED so we never mask the original
        upstream exception; TTL sweep is the durable backstop
        (review-standards.md 3.6).
        """
        if not handle.reservation_id:
            return
        outcome = _classify_failure(exc)
        try:
            client = await self._ensure_client()
        except SpendGuardError as ensure_exc:
            log.warning(
                "spendguard: release skipped (sidecar unavailable) for "
                "llm_call_id=%s outcome=%s err=%r; reservation will "
                "TTL-sweep.",
                handle.llm_call_id, outcome, ensure_exc,
            )
            return
        try:
            await client.emit_llm_call_post(
                run_id=handle.run_id,
                step_id=handle.step_id,
                llm_call_id=handle.llm_call_id,
                decision_id=handle.decision_id,
                reservation_id=handle.reservation_id,
                provider_reported_amount_atomic="0",
                estimated_amount_atomic="0",
                unit=handle.binding.unit,
                pricing=handle.binding.pricing,
                provider_event_id=provider_event_id,
                outcome=outcome,
            )
        except SpendGuardError as rel_exc:
            log.warning(
                "spendguard: release RPC failed for llm_call_id=%s "
                "outcome=%s err=%r; reservation will TTL-sweep.",
                handle.llm_call_id, outcome, rel_exc,
            )
            return
