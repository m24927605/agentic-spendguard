# ruff: noqa: ANN401, S106
# ANN401: LiteLLM kwargs are intentionally ``Any``; the wrapper just
#         forwards them.
# S106:   ``token_kind="output_token"`` is a proto field value, not a
#         credential.
"""Reserve / commit / release lifecycle for the LiteLLM SDK shim.

``_DirectCore`` is the workhorse: every patched LiteLLM entry point
(``acompletion``, ``atext_completion``, ``Router.acompletion``, sync
``completion`` via the async bridge) delegates here with the saved
*original* function as ``_original_acompletion``.

Why pass the original in instead of re-importing ``litellm.acompletion``?
Two reasons:

  1. After ``install_shim()`` runs, ``litellm.acompletion`` *is* the
     patched wrapper. Calling it from inside the core would re-enter
     the wrapper, hit the ``_IN_FLIGHT`` guard, and bypass the reserve
     for the inner call — correct but obscures the call graph. Explicit
     injection makes the dispatch path observable in stack traces.
  2. ``Router.acompletion`` is an instance method bound to a specific
     ``self`` (the router instance carrying its own model_list). The
     Router patch synthesises a closure that binds ``self`` and hands
     the resulting ``Callable[..., Awaitable]`` to the core. No router
     re-discovery needed.

The lifecycle mirrors ``SpendGuardDirectAcompletion.__call__`` but with
the simpler ``SpendGuardShimOptions`` surface — operator supplies
``tenant_id`` + ``budget_id`` (or ``$SPENDGUARD_BUDGET_ID``) and the
core builds a sensible default ``BudgetBinding`` + estimator +
reconciler internally.
"""

from __future__ import annotations

import asyncio
import logging
import os
import time
from collections.abc import Awaitable, Callable
from types import SimpleNamespace
from typing import Any

from ...errors import (
    DecisionDenied,
    SidecarUnavailable,
    SpendGuardConfigError,
    SpendGuardError,
)
from ...ids import derive_idempotency_key, derive_uuid_from_signature
from ...prompt_hash import compute as compute_prompt_hash
from .._default_estimator import litellm_default_claim_estimator
from ._options import SpendGuardShimOptions

log = logging.getLogger("spendguard.integrations.litellm_sdk_shim")


# Default identifiers used when the operator does not supply a budget /
# window. Kept as module-level constants so tests can patch them and
# operators can audit them in source. These mirror the
# single-tenant defaults from ``litellm_guardrail`` (``b1`` / ``w1`` /
# ``u1``) so a sidecar configured for D11 also works for D12.
_DEFAULT_BUDGET_ID = "shim-default-budget"
_DEFAULT_WINDOW_ID = "shim-default-window"
_DEFAULT_UNIT_ID = "shim-default-unit"


def _build_default_binding(options: SpendGuardShimOptions) -> SimpleNamespace:
    """Construct a sensible ``BudgetBinding``-shaped object from options.

    Returns a ``SimpleNamespace`` instead of the real ``BudgetBinding``
    dataclass to avoid a hard import of ``..litellm`` (which would drag
    in ``litellm.integrations.custom_logger``). The shim's ``_core`` is
    independently importable so tests can run without LiteLLM. The
    field surface matches ``BudgetBinding`` exactly so downstream
    consumers (the SpendGuardClient wire format) see no difference.
    """
    budget_id = options.budget_id or os.environ.get(
        "SPENDGUARD_BUDGET_ID", "",
    ).strip() or _DEFAULT_BUDGET_ID
    window_id = os.environ.get(
        "SPENDGUARD_WINDOW_INSTANCE_ID", "",
    ).strip() or _DEFAULT_WINDOW_ID
    unit_id = os.environ.get(
        "SPENDGUARD_UNIT_ID", "",
    ).strip() or _DEFAULT_UNIT_ID

    # Lazy proto import so a missing protobuf at unit-test time still
    # lets the rest of the module load (the actual sidecar call obviously
    # requires the proto, but the import lives in _core_invoke).
    from spendguard._proto.spendguard.common.v1 import common_pb2

    unit = common_pb2.UnitRef(
        unit_id=unit_id,
        token_kind="output_token",
        model_family="gpt-4",
    )
    pricing = common_pb2.PricingFreeze(
        pricing_version="shim-v1",
        price_snapshot_hash=b"\x00" * 32,
        fx_rate_version="shim-v1",
        unit_conversion_version="shim-v1",
    )
    return SimpleNamespace(
        budget_id=budget_id,
        window_instance_id=window_id,
        unit=unit,
        pricing=pricing,
    )


def _serialize_messages_for_hash(messages: Any) -> str:
    """Stable canonical-JSON of LiteLLM messages for prompt_hash input."""
    import json

    if messages is None:
        return ""
    try:
        return json.dumps(messages, sort_keys=True, separators=(",", ":"))
    except (TypeError, ValueError):
        return repr(messages)


class _DirectCore:
    """Reserve / call-original / commit-or-release lifecycle.

    One instance per ``install_shim()`` call; reused across every
    LiteLLM entry-point dispatch for the lifetime of the installation.

    Constructor binds the operator's options + a pre-built default
    binding so per-call dispatch is cheap (no env re-reads, no
    repeated proto construction).
    """

    def __init__(self, options: SpendGuardShimOptions) -> None:
        self._options = options
        self._client = options.client
        self._tenant_id = options.tenant_id
        self._fail_open = options.fail_open or (
            os.environ.get("SPENDGUARD_LITELLM_FAIL_OPEN", "").strip() == "1"
        )
        if self._fail_open:
            log.warning(
                "spendguard_litellm_shim: fail_open=True — sidecar errors "
                "will allow LLM calls (DEV ONLY).",
            )
        self._binding = _build_default_binding(options)
        # Per-binding default estimator. Closure captures the binding so
        # per-call dispatch only needs the request data dict.
        self._claim_estimator = litellm_default_claim_estimator(
            budget_id=self._binding.budget_id,
            window_instance_id=self._binding.window_instance_id,
            unit=self._binding.unit,
            model="gpt-4o-mini",  # overridden per-call from data["model"]
        )

    async def __call__(
        self,
        *,
        _original_acompletion: Callable[..., Awaitable[Any]],
        **litellm_kwargs: Any,
    ) -> Any:
        """Run one LLM call through the reserve→commit→release pipeline.

        ``_original_acompletion`` MUST be the pre-patch callable. The
        ``_patches/*`` modules supply it; callers in production never
        call ``_DirectCore`` directly.
        """
        # Build per-call IDs upfront. We mix urandom into the signature
        # so a tight ``asyncio.gather`` of bit-identical kwargs dicts
        # can't collide (matches ``SpendGuardDirectAcompletion`` Slice
        # A1 R1 F1 fix).
        litellm_call_id = str(
            litellm_kwargs.get("litellm_call_id")
            or derive_uuid_from_signature(
                f"shim:{id(litellm_kwargs)}:{time.time_ns()}:"
                f"{os.urandom(8).hex()}",
                scope="litellm_call_id",
            ),
        )
        # Inject the id back into kwargs so the original sees a stable
        # call id (matches LiteLLM's own call-id stamping behavior).
        litellm_kwargs["litellm_call_id"] = litellm_call_id

        data: dict[str, Any] = dict(litellm_kwargs)
        resolver_ctx = SimpleNamespace(
            data=data,
            user_api_key_dict=None,
            call_type=str(data.get("call_type", "acompletion")),
        )

        # Build claim via the per-binding default estimator. Single-claim
        # contract per DESIGN §6.
        estimator_claims = self._claim_estimator(resolver_ctx)
        if len(estimator_claims) != 1:
            raise SpendGuardConfigError(
                f"shim default estimator returned {len(estimator_claims)} "
                "claims; v1 contract requires exactly 1.",
            )

        # Derive deterministic per-call IDs.
        llm_call_id = str(derive_uuid_from_signature(
            f"shim:{litellm_call_id}", scope="llm_call_id",
        ))
        decision_id = str(derive_uuid_from_signature(
            f"shim:{litellm_call_id}", scope="decision_id",
        ))
        run_id = str(derive_uuid_from_signature(
            f"shim:{litellm_call_id}", scope="run_id",
        ))
        step_id = f"shim:{litellm_call_id[:16]}"

        idempotency_key = derive_idempotency_key(
            tenant_id=self._tenant_id,
            session_id=getattr(
                self._client, "session_id", self._tenant_id,
            ),
            run_id=run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )

        prompt_hash = compute_prompt_hash(
            _serialize_messages_for_hash(data.get("messages")),
            self._tenant_id,
        )
        decision_context = {
            "integration": "litellm",
            "mode": "sdk",  # distinguishes the shim path from proxy / direct
            "litellm_call_id": litellm_call_id,
            "model": data.get("model"),
            "prompt_hash": prompt_hash,
            "call_type": resolver_ctx.call_type,
            "stream": bool(data.get("stream", False)),
        }

        # ─── Reserve ────────────────────────────────────────────────
        try:
            outcome = await self._client.request_decision(
                trigger="LLM_CALL_PRE",
                run_id=run_id,
                step_id=step_id,
                llm_call_id=llm_call_id,
                tool_call_id="",
                decision_id=decision_id,
                route="llm.call",
                projected_claims=estimator_claims,
                idempotency_key=idempotency_key,
                projected_unit=self._binding.unit,
                decision_context_json=decision_context,
            )
        except DecisionDenied:
            # Operator-visible budget gate; propagate untouched.
            raise
        except SpendGuardError as exc:
            if self._fail_open:
                log.warning(
                    "spendguard_litellm_shim: sidecar pre-call error %r — "
                    "fail_open=True allowing call (DEV ONLY).",
                    exc,
                )
                return await _original_acompletion(**litellm_kwargs)
            raise SidecarUnavailable(
                f"sidecar pre-call failed (sdk shim mode): {exc}",
            ) from exc

        if getattr(outcome, "decision", "") == "DEGRADE":
            if self._fail_open:
                log.warning(
                    "spendguard_litellm_shim: DEGRADE under fail_open — "
                    "allowing call (DEV ONLY).",
                )
                return await _original_acompletion(**litellm_kwargs)
            raise SidecarUnavailable(
                "sidecar returned DEGRADE; sdk shim mode fails closed.",
            )

        if len(outcome.reservation_ids) != 1:
            raise SpendGuardConfigError(
                f"sidecar returned {len(outcome.reservation_ids)} "
                "reservations; v1 expects exactly 1.",
            )
        reservation_id = outcome.reservation_ids[0]

        # ─── Provider call ──────────────────────────────────────────
        try:
            response = await _original_acompletion(**litellm_kwargs)
        except Exception as call_exc:
            # Best-effort release — swallow SpendGuardError (TTL sweep
            # is the durable backstop), bubble any other rare error.
            try:
                await self._client.emit_llm_call_post(
                    run_id=run_id,
                    step_id=step_id,
                    llm_call_id=llm_call_id,
                    decision_id=outcome.decision_id,
                    reservation_id=reservation_id,
                    provider_reported_amount_atomic="0",
                    estimated_amount_atomic="0",
                    unit=self._binding.unit,
                    pricing=self._binding.pricing,
                    provider_event_id="",
                    outcome=(
                        "CANCELLED"
                        if isinstance(call_exc, asyncio.CancelledError)
                        else "FAILURE"
                    ),
                )
            except SpendGuardError as rel_exc:
                log.warning(
                    "spendguard_litellm_shim: release RPC failed for "
                    "llm_call_id=%s err=%r; reservation will TTL-sweep.",
                    llm_call_id,
                    rel_exc,
                )
            raise

        # ─── Commit ──────────────────────────────────────────────────
        # Reconcile from response.usage.completion_tokens (OpenAI shape;
        # LiteLLM normalises Anthropic / Bedrock / Gemini). Falls back
        # to the estimator amount if usage is missing.
        usage = getattr(response, "usage", None)
        actual_output_tokens = (
            getattr(usage, "completion_tokens", None) if usage else None
        )
        actual_input_tokens = (
            getattr(usage, "prompt_tokens", None) if usage else None
        )
        amount_atomic = (
            str(actual_output_tokens)
            if isinstance(actual_output_tokens, int) and actual_output_tokens >= 0
            else str(estimator_claims[0].amount_atomic)
        )

        try:
            await self._client.emit_llm_call_post(
                run_id=run_id,
                step_id=step_id,
                llm_call_id=llm_call_id,
                decision_id=outcome.decision_id,
                reservation_id=reservation_id,
                provider_reported_amount_atomic="",
                estimated_amount_atomic=amount_atomic,
                unit=self._binding.unit,
                pricing=self._binding.pricing,
                provider_event_id=str(getattr(response, "id", "") or ""),
                outcome="SUCCESS",
                actual_input_tokens=(
                    actual_input_tokens
                    if isinstance(actual_input_tokens, int)
                    else None
                ),
                actual_output_tokens=(
                    actual_output_tokens
                    if isinstance(actual_output_tokens, int)
                    else None
                ),
            )
        except SpendGuardError as commit_exc:
            log.warning(
                "spendguard_litellm_shim: commit RPC failed for "
                "llm_call_id=%s err=%r; reservation will TTL-sweep. "
                "Returning provider response to caller.",
                llm_call_id,
                commit_exc,
            )
        return response


__all__ = ["_DirectCore"]
