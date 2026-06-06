# ruff: noqa: ANN401  # LiteLLM's CustomGuardrail interface uses untyped Any
"""LiteLLM proxy ``CustomGuardrail`` plugin — SLICE 1 + 2 + 3 wired.

Discoverability + zero-Python install path for SpendGuard on the
LiteLLM proxy. Wraps the existing ``_LoopBoundCallback`` from
``spendguard.integrations.litellm`` (composition, NOT inheritance) so
both the legacy ``litellm_settings.callbacks`` path AND the new
``guardrails:`` registry path drive the same reserve / commit / release
flow (DESIGN.md §3.4 v1 Path B).

Slice 1 shipped:
    * Module + class ``SpendGuardGuardrail`` registered against the
      LiteLLM 1.50+ ``CustomGuardrail`` ABC.
    * Composition with ``_LoopBoundCallback`` — the delegate instance
      lives on ``self._delegate``. Lazy loop binding stays in the
      delegate; this wrapper never touches an event loop at import.

Slice 2 shipped:
    * ``async_pre_call_hook`` wired — pure delegation to
      ``_LoopBoundCallback.async_pre_call_hook``.

Slice 3 ships (this file):
    * ``async_post_call_success_hook`` wired — translates the
      ``CustomGuardrail`` ``(data, user_api_key_dict, response)``
      signature into the ``CustomLogger`` ``(kwargs, response_obj,
      start_time, end_time)`` shape that
      ``_LoopBoundCallback.async_log_success_event`` consumes.
    * ``async_post_call_failure_hook`` wired — translates the
      ``CustomGuardrail`` ``(request_data, original_exception,
      user_api_key_dict, traceback_str)`` signature into the
      ``CustomLogger`` shape that
      ``_LoopBoundCallback.async_log_failure_event`` consumes, then
      re-raises the original exception per LiteLLM's failure-hook
      propagation contract.

Anti-scope for SLICE 3 (per ``docs/slices/COV_D11_S3_commit_release.md``):
    * No env-driven default factory — SLICE 4.
    * No ``proxy_config.yaml`` snippet — SLICE 5.
    * No demo mode — SLICE 6.
    * No docs page — SLICE 7.

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
        """Pre-call gate — pure delegation to ``_LoopBoundCallback``.

        Per ``review-standards.md`` §Slice 2:
            * 2.1 (Blocker): pure delegation, fewer than 5 LOC of body
              excluding signature, no new error handling.
            * 2.2 (Blocker): ``DecisionDenied`` /
              ``SidecarUnavailable`` / ``SpendGuardConfigError`` raises
              propagate; no ``except`` swallowing.
            * 2.3 (Blocker): return value forwarded verbatim from
              delegate; no ``data`` mutation.

        The delegate (``_LoopBoundCallback``) is single-sourced in
        ``litellm.py`` and already implements:
            * reserve via ``request_decision`` (litellm.py L388-399)
            * DENY → ``DecisionDenied`` propagates (L400-401)
              → LiteLLM proxy maps ``status_code=403`` to HTTP 403
              automatically via ``getattr(exc, "status_code", 500)``
              (per ``errors.py`` L53; see DESIGN §3.4 Path B).
            * DEGRADE → ``SidecarUnavailable`` raised (L418-429)
              → ``status_code=503`` (``errors.py`` L35) → HTTP 503.
            * ALLOW → returns ``data`` unchanged (L476).
            * Lazy event-loop binding via
              ``_LoopBoundCallback._ensure_client`` (L804-863) — the
              guardrail wrapper never touches the loop.

        SLICE 1 sentinel resolvers (``_skeleton_budget_resolver`` etc)
        surface as a loud ``SpendGuardConfigError("budget_resolver
        returned None")`` from ``litellm.py`` L298-302 when this hook
        runs without an operator-supplied resolver. That's by design:
        SLICE 4 replaces the sentinels with the env-driven factory
        per ``implementation.md`` §2.4. Until then, the hook fails
        loudly (not silently) when wired against the default skeleton.
        """
        return await self._delegate.async_pre_call_hook(
            user_api_key_dict, cache, data, call_type,
        )

    async def async_post_call_success_hook(
        self,
        data: dict[str, Any],
        user_api_key_dict: Any,
        response: Any,
    ) -> None:
        """Commit path — pure delegation to ``_LoopBoundCallback``.

        Per ``review-standards.md`` §Slice 3:
            * 3.1 (Blocker): kwargs dict carries ``litellm_call_id``
              copied from ``data`` so the delegate's ``_get_stash``
              finds the SLICE 2 reserve stash. Missing
              ``litellm_call_id`` → ``_get_stash`` returns None →
              silent no-op (per ``litellm.py`` L482-483 contract).
            * 3.3 (Major): ``start_time`` / ``end_time`` are
              propagated from ``data`` when LiteLLM stamps them; the
              delegate's commit path reads ``response.usage`` (not
              timestamps) so ``None`` is safe — pinned by U05.
            * 3.4 (Blocker): when ``response.usage`` is None the
              delegate's streaming-fallback branch fires
              (``litellm.py`` L598-608) — commits the estimator
              snapshot + WARN log. Owned by the delegate; the wrapper
              forwards the (possibly usage-less) ``response`` verbatim.

        Translation contract (CustomGuardrail → CustomLogger):
            ``CustomGuardrail.async_post_call_success_hook`` signature::

                (data, user_api_key_dict, response)

            ``_LoopBoundCallback.async_log_success_event`` signature::

                (kwargs: dict, response_obj, start_time, end_time)

            * ``kwargs = dict(data)`` so ``kwargs["litellm_call_id"]``
              hits the same stash key the pre-call hook populated.
              ``dict(data)`` is a shallow copy — never mutates the
              caller's ``data`` dict (review-standards 2.3 sibling
              invariant carried over to SLICE 3).
            * ``kwargs["user_api_key_dict"]`` populated so the
              delegate's resolver context construction
              (``litellm.py`` L519) has the team/tenant scope.
            * ``response`` forwarded as ``response_obj`` verbatim;
              the delegate's reconciler reads ``response.usage``
              and ``response.id``.
            * ``start_time`` / ``end_time`` propagated from ``data``
              when LiteLLM stamps them (forward-compat); fall back to
              ``None`` (delegate ignores them — commit reads
              ``response.usage``).

        LiteLLM contract: success hook returns ``None``; the proxy
        does not expect a return value on this surface (verified
        against ``CustomGuardrail.async_post_call_success_hook``
        signature -> ``Any`` but the registry ignores returns).
        """
        kwargs: dict[str, Any] = dict(data)
        kwargs["litellm_call_id"] = data.get("litellm_call_id")
        kwargs["user_api_key_dict"] = user_api_key_dict
        await self._delegate.async_log_success_event(
            kwargs,
            response,
            data.get("start_time"),
            data.get("end_time"),
        )
        return None

    async def async_post_call_failure_hook(
        self,
        request_data: dict[str, Any],
        original_exception: Exception,
        user_api_key_dict: Any,
        traceback_str: str | None = None,
    ) -> None:
        """Release path — pure delegation to ``_LoopBoundCallback``.

        Per ``review-standards.md`` §Slice 3:
            * 3.1 (Blocker): kwargs dict carries ``litellm_call_id``
              copied from ``request_data`` so the delegate's
              ``_get_stash`` finds the SLICE 2 reserve stash.
            * 3.2 (Blocker): ``kwargs["exception"] =
              original_exception`` is populated BEFORE forwarding so
              the delegate's ``_classify_failure`` (``litellm.py``
              L739-760) can map ``asyncio.CancelledError`` → outcome
              CANCELLED vs every other exception → outcome FAILURE.
              Missing this populate would silently misclassify every
              failure as FAILURE.
            * 3.3 (Major): ``start_time`` / ``end_time`` propagated
              from ``request_data`` when LiteLLM stamps them; the
              delegate's release path does not consume them, so
              ``None`` is safe.
            * 3.5 (Minor): no new exception types introduced. The
              original exception is re-raised verbatim per LiteLLM's
              failure-hook propagation contract (HTTP error path).

        Translation contract (CustomGuardrail → CustomLogger):
            ``CustomGuardrail.async_post_call_failure_hook`` signature::

                (request_data, original_exception, user_api_key_dict,
                 traceback_str=None)

            ``_LoopBoundCallback.async_log_failure_event`` signature::

                (kwargs: dict, response_obj, start_time, end_time)

            * ``kwargs = dict(request_data)`` shallow copy (caller
              dict never mutated).
            * ``kwargs["exception"] = original_exception`` so
              ``_classify_failure`` reads the exception object.
            * ``kwargs["litellm_call_id"]`` copied from
              ``request_data`` so ``_get_stash`` finds the
              reservation.
            * ``response_obj`` passed as None (no successful response
              on the failure path); the delegate's release branch
              tolerates this via ``getattr(response_obj, "id", "")``
              (``litellm.py`` L491-493).
            * ``traceback_str`` is the LiteLLM 1.55+ optional arg;
              not forwarded into the delegate's kwargs by design
              (the delegate already has the exception object;
              traceback strings can leak PII into audit logs and the
              release path does not consume them).

        LiteLLM contract: failure hook is expected to propagate the
        original exception so the proxy returns the underlying HTTP
        error rather than swallowing it. The delegate's
        ``async_log_failure_event`` already swallows its own RPC
        errors (``litellm.py`` L722-729) to avoid masking the
        original LiteLLM exception — we therefore re-raise the
        original exception verbatim after the delegate's release
        call completes (or its own errors are logged).
        """
        kwargs: dict[str, Any] = dict(request_data)
        kwargs["litellm_call_id"] = request_data.get("litellm_call_id")
        kwargs["user_api_key_dict"] = user_api_key_dict
        kwargs["exception"] = original_exception
        await self._delegate.async_log_failure_event(
            kwargs,
            None,
            request_data.get("start_time"),
            request_data.get("end_time"),
        )
        raise original_exception


__all__ = ["SpendGuardGuardrail"]
