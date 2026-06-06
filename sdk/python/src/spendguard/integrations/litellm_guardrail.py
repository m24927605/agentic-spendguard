# ruff: noqa: ANN401  # LiteLLM's CustomGuardrail interface uses untyped Any
"""LiteLLM proxy ``CustomGuardrail`` plugin — SLICE 1 skeleton.

Discoverability + zero-Python install path for SpendGuard on the
LiteLLM proxy. Wraps the existing ``_LoopBoundCallback`` from
``spendguard.integrations.litellm`` (composition, NOT inheritance) so
both the legacy ``litellm_settings.callbacks`` path AND the new
``guardrails:`` registry path drive the same reserve / commit / release
flow (DESIGN.md §3.4 v1 Path B).

Slice 1 ships:
    * Module + class ``SpendGuardGuardrail`` registered against the
      LiteLLM 1.50+ ``CustomGuardrail`` ABC.
    * Composition with ``_LoopBoundCallback`` — the delegate instance
      lives on ``self._delegate``. Lazy loop binding stays in the
      delegate; this wrapper never touches an event loop at import.
    * Stub hook bodies that raise ``NotImplementedError`` pointing at
      the slice that will wire them. Tests verify they are
      coroutines + present without invoking the wired flow.

Anti-scope for SLICE 1 (per ``docs/slices/COV_D11_S1_guardrail_class.md``):
    * No real pre-call wiring — SLICE 2.
    * No commit / release wiring — SLICE 3.
    * No env-driven default factory — SLICE 4.
    * No ``proxy_config.yaml`` snippet — SLICE 5.

Backwards compatibility (per ``implementation.md`` §3):
    * ``spendguard.integrations.litellm.SpendGuardLiteLLMCallback`` is
      UNTOUCHED — the legacy ``litellm_settings.callbacks: [...]`` path
      keeps working. All ``test_litellm_*`` units stay green.
    * No new runtime dependency beyond what ``[litellm]`` already pulls.
"""

from __future__ import annotations

import logging
from typing import Any

# Composition source — these names are re-used unchanged so the
# reserve / commit / release flow remains single-sourced in
# ``litellm.py`` (1141 LOC, 5 hardened rounds of review).
from .litellm import (
    BudgetResolver,
    ClaimEstimator,
    ClaimReconciler,
    _LoopBoundCallback,
)

# Lazy CustomGuardrail import: surface a SpendGuard-shaped install hint
# when the operator forgot ``pip install 'spendguard-sdk[litellm-guardrail]'``
# (or is on a litellm release older than the guardrail surface). The
# ImportError shape is asserted by U01.
try:
    from litellm.integrations.custom_guardrail import CustomGuardrail
except ImportError as exc:  # pragma: no cover  - exercised via monkeypatch in U01
    raise ImportError(
        "spendguard.integrations.litellm_guardrail requires LiteLLM "
        "with guardrail support (>= 1.55). Install with: "
        "pip install 'spendguard-sdk[litellm-guardrail]'"
    ) from exc


log = logging.getLogger("spendguard.integrations.litellm_guardrail")


# Sentinel resolver / estimator / reconciler used ONLY when the
# operator constructs ``SpendGuardGuardrail()`` without explicit
# wiring kwargs. Slice 1 hooks never reach them (the hooks raise
# ``NotImplementedError``). Slice 4 replaces these defaults with
# the env-driven factory described in ``implementation.md`` §2.4.
def _skeleton_budget_resolver(_ctx: Any) -> None:
    return None


def _skeleton_claim_estimator(_ctx: Any) -> list[Any]:
    return []


def _skeleton_claim_reconciler(_ctx: Any, _response: Any) -> list[Any]:
    return []


class SpendGuardGuardrail(CustomGuardrail):
    """SpendGuard ``CustomGuardrail`` for the LiteLLM proxy (skeleton).

    Composition wires this wrapper to a ``_LoopBoundCallback`` instance
    on ``self._delegate``. SLICE 2/3 will route the three hook methods
    through that delegate; SLICE 1 only ships the class shape.

    Registration (target ``proxy_config.yaml``, wired in SLICE 5)::

        guardrails:
          - guardrail_name: spendguard
            litellm_params:
              guardrail: spendguard.integrations.litellm_guardrail.SpendGuardGuardrail
              mode: pre_call
              default_on: true

    Args:
        guardrail_name: Operator-facing name used by the LiteLLM
            registry. Defaults to ``"spendguard"`` to match the
            registry snippet shipped in SLICE 5.
        budget_resolver: Optional explicit resolver. SLICE 4 default
            factory reads this from env when omitted.
        claim_estimator: Optional explicit estimator.
        claim_reconciler: Optional explicit reconciler.
        socket_path: Optional sidecar UDS path (``SPENDGUARD_SIDECAR_UDS``
            target in SLICE 4).
        tenant_id: Optional SpendGuard tenant id (``SPENDGUARD_TENANT_ID``
            target in SLICE 4).
        **kwargs: Forwarded to ``CustomGuardrail.__init__`` so the
            LiteLLM registry can pass ``mode`` / ``default_on`` /
            ``supported_event_hooks`` etc. without this wrapper having
            to enumerate the upstream knobs.
    """

    def __init__(
        self,
        *,
        guardrail_name: str = "spendguard",
        budget_resolver: BudgetResolver | None = None,
        claim_estimator: ClaimEstimator | None = None,
        claim_reconciler: ClaimReconciler | None = None,
        socket_path: str | None = None,
        tenant_id: str | None = None,
        **kwargs: Any,
    ) -> None:
        # ``CustomGuardrail`` accepts ``guardrail_name`` as its first
        # positional and forwards the rest into ``CustomLogger``. We
        # MUST call ``super().__init__`` (review-standards 1.2 Blocker)
        # so LiteLLM's registry can introspect the instance later.
        super().__init__(guardrail_name=guardrail_name, **kwargs)

        # Composition (review-standards 1.3 Blocker): hold a
        # ``_LoopBoundCallback`` instance — never subclass it,
        # never multiply-inherit ``CustomGuardrail`` + ``CustomLogger``.
        # Defaults are no-op sentinels so the class is constructable
        # without an operator-side resolver in skeleton mode. SLICE 2
        # replaces the hook bodies; SLICE 4 swaps the defaults for the
        # env-driven factory.
        self._delegate: _LoopBoundCallback = _LoopBoundCallback(
            socket_path=socket_path or "",
            tenant_id=tenant_id or "",
            budget_resolver=budget_resolver or _skeleton_budget_resolver,
            claim_estimator=claim_estimator or _skeleton_claim_estimator,
            claim_reconciler=claim_reconciler or _skeleton_claim_reconciler,
        )

    async def async_pre_call_hook(
        self,
        user_api_key_dict: Any,
        cache: Any,
        data: dict[str, Any],
        call_type: str,
    ) -> dict[str, Any] | None:
        """Pre-call gate — wired in SLICE 2.

        SLICE 2 will pure-delegate to
        ``self._delegate.async_pre_call_hook(...)`` so reserve /
        DEGRADE / DENY semantics stay single-sourced in ``litellm.py``.
        """
        raise NotImplementedError(
            "SpendGuardGuardrail.async_pre_call_hook is wired in "
            "COV_D11_S2 (reserve via _LoopBoundCallback delegate)."
        )

    async def async_post_call_success_hook(
        self,
        data: dict[str, Any],
        user_api_key_dict: Any,
        response: Any,
    ) -> None:
        """Commit path — wired in SLICE 3.

        SLICE 3 will translate the ``CustomGuardrail`` post-call
        signature into the ``CustomLogger`` ``kwargs / response_obj``
        shape that ``_LoopBoundCallback.async_log_success_event``
        expects, including the streaming-fallback branch.
        """
        raise NotImplementedError(
            "SpendGuardGuardrail.async_post_call_success_hook is wired "
            "in COV_D11_S3 (commit via delegate.async_log_success_event)."
        )

    async def async_post_call_failure_hook(
        self,
        request_data: dict[str, Any],
        original_exception: Exception,
        user_api_key_dict: Any,
        traceback_str: str | None = None,
    ) -> None:
        """Release path — wired in SLICE 3.

        SLICE 3 will translate the ``CustomGuardrail`` failure-hook
        signature into the ``CustomLogger`` ``kwargs / exception``
        shape that ``_LoopBoundCallback.async_log_failure_event``
        consumes. ``traceback_str`` is the LiteLLM 1.55+ optional
        argument; signature accepts it for forward-compat without
        forcing a floor-bump in SLICE 1.
        """
        raise NotImplementedError(
            "SpendGuardGuardrail.async_post_call_failure_hook is wired "
            "in COV_D11_S3 (release via delegate.async_log_failure_event)."
        )


__all__ = ["SpendGuardGuardrail"]
