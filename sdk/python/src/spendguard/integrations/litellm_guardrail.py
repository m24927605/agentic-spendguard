# ruff: noqa: ANN401  # LiteLLM's CustomGuardrail interface uses untyped Any
"""LiteLLM proxy ``CustomGuardrail`` plugin — SLICE 1 + 2 + 3 + 4 + 4b wired.

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

Slice 3 shipped:
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

Slice 4 ships (this file):
    * Env-driven default factory classmethods:
        - ``SpendGuardGuardrail.from_env()`` reads
          ``SPENDGUARD_TENANT_ID`` / ``SPENDGUARD_SIDECAR_ADDRESS``
          (with ``SPENDGUARD_SIDECAR_UDS`` legacy fallback) /
          optional ``SPENDGUARD_API_KEY`` / ``SPENDGUARD_DISABLED``
          / ``SPENDGUARD_PROXY_TIMEOUT_MS`` and constructs a fully
          wired guardrail — adapter authors no longer need to thread
          kwargs through proxy yaml.
        - ``SpendGuardGuardrail.from_kwargs(**kwargs)`` is the
          explicit-kwargs constructor (kwargs win over env;
          delegates straight to ``__init__``).
        - ``SpendGuardGuardrail.from_config(config: dict)`` accepts
          the parsed-yaml dict shape SLICE 5's ``proxy_config.yaml``
          entry will produce.
    * Missing required env var → ``SpendGuardConfigError`` naming the
      var (review-standards 4.1 Blocker).
    * ``SPENDGUARD_DISABLED=true`` / ``1`` / ``yes`` (case-insensitive)
      → no-op delegate: hooks short-circuit without touching the
      sidecar (mirrors the TS SDK disabled-mode pattern).

Slice 4b ships (this file):
    * Resolver-module wiring: ``SPENDGUARD_RESOLVER_MODULE`` —
      ``pkg.mod:fn_name`` triple-factory escape hatch returning
      ``(BudgetResolver, ClaimEstimator | None, ClaimReconciler)``.
      Dispatched via ``importlib.import_module`` + ``getattr``.
    * Single-tenant default path (when ``SPENDGUARD_RESOLVER_MODULE``
      is unset): build a closure resolver + reconciler from the 3
      budget-binding env vars (``SPENDGUARD_BUDGET_ID`` /
      ``SPENDGUARD_WINDOW_INSTANCE_ID`` / ``SPENDGUARD_UNIT_ID``) +
      the 4 pricing-version env vars (``SPENDGUARD_PRICING_VERSION`` /
      ``SPENDGUARD_FX_RATE_VERSION`` /
      ``SPENDGUARD_UNIT_CONVERSION_VERSION`` /
      ``SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX``). Mirrors the field-by-
      field shape used by
      ``examples/litellm-proxy-composite/spendguard_litellm_proxy_callback.py``.
    * ``BudgetBinding`` validation: empty
      ``budget_id`` / ``window_instance_id`` / ``unit_id`` rejected
      at factory time, naming the offending field — fail-closed
      before the first hook invocation rather than surfacing
      ``SpendGuardConfigError("budget_resolver returned None")``
      from ``litellm.py:298-302`` at the first request.
    * Same wiring lands in ``from_config`` so SLICE 5's
      ``proxy_config.yaml`` loader inherits it verbatim.

Anti-scope for SLICE 4b (per ``docs/slices/COV_D11_S4B_resolver_module.md``):
    * No ``proxy_config.yaml`` snippet — SLICE 5.
    * No demo mode — SLICE 6.
    * No docs page — SLICE 7.
    * No re-touch of the 5-var SLICE 4 subset — SLICE 4 tests stay
      unchanged.

Anti-scope for SLICE 4 (per ``docs/slices/COV_D11_S4_env_defaults.md``):
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

import importlib
import logging
import os
from typing import Any

from spendguard.errors import SpendGuardConfigError

# Composition source — these names are re-used unchanged so the
# reserve / commit / release flow remains single-sourced in
# ``litellm.py`` (1141 LOC, 5 hardened rounds of review).
from .litellm import (
    BudgetBinding,
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

    # -------------------------------------------------------------------
    # SLICE 4 — env-driven default factory classmethods.
    #
    # `from_env`:    operator config from environment variables.
    # `from_kwargs`: explicit kwargs (mirrors `__init__` surface but
    #                callable from a dict via `**`).
    # `from_config`: parsed-yaml dict (SLICE 5 `proxy_config.yaml`
    #                parser calls this).
    #
    # Each factory:
    #   * Returns a fresh `SpendGuardGuardrail` (no module-level
    #     singleton — review-standards 1.4 carryover).
    #   * Raises `SpendGuardConfigError` when a required value is
    #     missing, naming the offending env var or config key
    #     (review-standards 4.1 Blocker).
    #   * Honours `disabled=true` by installing a no-op delegate so
    #     hooks short-circuit (mirrors the TS SDK disabled-mode).
    #
    # No `from_env` on `SpendGuardClient` exists today (verified) and
    # this SLICE 4 must NOT refactor the existing client surface; we
    # read env vars directly here and forward parsed values into the
    # `_LoopBoundCallback` constructor instead.
    # -------------------------------------------------------------------

    @classmethod
    def from_env(cls) -> SpendGuardGuardrail:
        """Construct a guardrail from environment variables.

        Required env vars:
            * ``SPENDGUARD_TENANT_ID``
            * ``SPENDGUARD_SIDECAR_ADDRESS`` (or legacy
              ``SPENDGUARD_SIDECAR_UDS``; either is accepted so
              existing deployments — examples/litellm-proxy-composite,
              the legacy callback path — continue to work)

        Optional env vars:
            * ``SPENDGUARD_API_KEY`` — sidecar auth token; default None.
            * ``SPENDGUARD_DISABLED`` — case-insensitive truthy values
              (``true`` / ``1`` / ``yes`` / ``on``) install a no-op
              delegate so hooks short-circuit without touching the
              sidecar. Default False.
            * ``SPENDGUARD_PROXY_TIMEOUT_MS`` — integer milliseconds;
              defaults to 5000. Parsed via ``int()``; non-integer
              raises ``SpendGuardConfigError`` naming the var.

        Returns:
            A fully constructed ``SpendGuardGuardrail`` ready to
            register with the LiteLLM proxy. The underlying
            ``_LoopBoundCallback`` defers gRPC channel creation to
            the first hook invocation on the serving event loop
            (Round 3 P0.3 — loop-affinity invariant).

        Raises:
            SpendGuardConfigError: a required env var is missing or a
                typed value (e.g. ``SPENDGUARD_PROXY_TIMEOUT_MS``)
                cannot be parsed. The message names the offending
                var so operators can fix the deployment.
        """
        config = cls._read_env_config()
        return cls._from_parsed_config(config)

    @classmethod
    def from_kwargs(cls, **kwargs: Any) -> SpendGuardGuardrail:
        """Construct a guardrail from explicit kwargs.

        Delegates straight to ``__init__`` — useful for callers that
        hold the config as a dict and want to splat it without
        running the env-var resolution pipeline (kwargs win over env;
        env is NOT consulted on this path).

        The kwargs surface mirrors ``__init__``:
        ``guardrail_name`` / ``budget_resolver`` / ``claim_estimator``
        / ``claim_reconciler`` / ``socket_path`` / ``tenant_id`` plus
        any extra kwargs LiteLLM's registry forwards into
        ``CustomGuardrail`` (e.g. ``mode``, ``default_on``,
        ``supported_event_hooks``).
        """
        return cls(**kwargs)

    @classmethod
    def from_config(cls, config: dict) -> SpendGuardGuardrail:
        """Construct a guardrail from a parsed config dict.

        Accepts the dict shape SLICE 5's ``proxy_config.yaml`` parser
        will emit. Same construction pipeline as ``from_env`` but
        reads from ``config`` instead of ``os.environ``.

        Expected keys:
            * ``tenant_id`` (required)
            * ``sidecar_address`` or legacy ``socket_path`` /
              ``sidecar_uds`` (required; first non-empty wins)
            * ``api_key`` (optional)
            * ``disabled`` (optional bool; accepts True / strings)
            * ``proxy_timeout_ms`` (optional int)

        Raises:
            SpendGuardConfigError: required key missing or invalid.
        """
        parsed = cls._coerce_config_dict(config)
        return cls._from_parsed_config(parsed)

    # -------------------------------------------------------------------
    # Internal config-resolution helpers (SLICE 4).
    # -------------------------------------------------------------------

    @staticmethod
    def _parse_disabled(raw: str | None) -> bool:
        """Parse a disabled-flag string into a bool.

        Truthy values (case-insensitive): ``true`` / ``1`` / ``yes`` /
        ``on``. Falsy values (default): everything else, including
        empty string and ``false`` / ``0`` / ``no`` / ``off``.

        Centralised so ``from_env`` and ``from_config`` agree on
        truthiness semantics — no operator surprises from a yaml
        boolean vs an env-var string mismatch.
        """
        if raw is None:
            return False
        return raw.strip().lower() in {"true", "1", "yes", "on"}

    @classmethod
    def _read_env_config(cls) -> dict[str, Any]:
        """Resolve env vars into the parsed-config dict shape.

        Centralised so both ``from_env`` and any future config
        loaders share the same env-var spelling table. Required vars
        missing → ``SpendGuardConfigError`` named at the call site.
        """
        tenant_id = os.environ.get("SPENDGUARD_TENANT_ID", "").strip()
        if not tenant_id:
            raise SpendGuardConfigError(
                "missing env var SPENDGUARD_TENANT_ID — required for "
                "SpendGuardGuardrail.from_env(). Set the SpendGuard "
                "tenant id at LiteLLM proxy boot."
            )

        sidecar_address = (
            os.environ.get("SPENDGUARD_SIDECAR_ADDRESS", "").strip()
            or os.environ.get("SPENDGUARD_SIDECAR_UDS", "").strip()
        )
        if not sidecar_address:
            raise SpendGuardConfigError(
                "missing env var SPENDGUARD_SIDECAR_ADDRESS — required "
                "for SpendGuardGuardrail.from_env(). Set the sidecar "
                "UDS path (e.g. unix:///run/spendguard.sock) at "
                "LiteLLM proxy boot. SPENDGUARD_SIDECAR_UDS is also "
                "accepted as a legacy alias."
            )

        api_key = os.environ.get("SPENDGUARD_API_KEY") or None
        disabled = cls._parse_disabled(os.environ.get("SPENDGUARD_DISABLED"))

        timeout_raw = os.environ.get("SPENDGUARD_PROXY_TIMEOUT_MS", "").strip()
        if timeout_raw:
            try:
                proxy_timeout_ms: int = int(timeout_raw)
            except ValueError as exc:
                raise SpendGuardConfigError(
                    "invalid env var SPENDGUARD_PROXY_TIMEOUT_MS="
                    f"{timeout_raw!r} — must be an integer millisecond "
                    "value (e.g. 5000)."
                ) from exc
        else:
            proxy_timeout_ms = 5000

        # ---------------------------------------------------------------
        # SLICE 4b: resolver-module + single-tenant binding env vars.
        # All optional from the SLICE 4 5-var subset's perspective —
        # when none are set the SLICE 1 skeleton resolver stays put and
        # existing SLICE 4 baseline tests pass unchanged.
        # ---------------------------------------------------------------
        resolver_module = (
            os.environ.get("SPENDGUARD_RESOLVER_MODULE", "").strip() or None
        )
        budget_id = os.environ.get("SPENDGUARD_BUDGET_ID", "").strip() or None
        window_instance_id = (
            os.environ.get("SPENDGUARD_WINDOW_INSTANCE_ID", "").strip() or None
        )
        unit_id = os.environ.get("SPENDGUARD_UNIT_ID", "").strip() or None
        pricing_version = (
            os.environ.get("SPENDGUARD_PRICING_VERSION", "").strip() or None
        )
        fx_rate_version = (
            os.environ.get("SPENDGUARD_FX_RATE_VERSION", "").strip() or None
        )
        unit_conversion_version = (
            os.environ.get("SPENDGUARD_UNIT_CONVERSION_VERSION", "").strip() or None
        )
        price_snapshot_hash_hex = (
            os.environ.get("SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX", "").strip() or None
        )

        return {
            "tenant_id": tenant_id,
            "sidecar_address": sidecar_address,
            "api_key": api_key,
            "disabled": disabled,
            "proxy_timeout_ms": proxy_timeout_ms,
            "resolver_module": resolver_module,
            "budget_id": budget_id,
            "window_instance_id": window_instance_id,
            "unit_id": unit_id,
            "pricing_version": pricing_version,
            "fx_rate_version": fx_rate_version,
            "unit_conversion_version": unit_conversion_version,
            "price_snapshot_hash_hex": price_snapshot_hash_hex,
        }

    @classmethod
    def _coerce_config_dict(cls, config: dict) -> dict[str, Any]:
        """Coerce a caller-supplied dict into the parsed-config shape.

        Same semantics as ``_read_env_config`` but the source is the
        dict, not the environment. SLICE 5's ``proxy_config.yaml``
        parser will hand us a dict like::

            {
                "tenant_id": "...",
                "sidecar_address": "unix:///run/spendguard.sock",
                "api_key": "...",            # optional
                "disabled": false,           # optional
                "proxy_timeout_ms": 5000,    # optional
            }
        """
        if not isinstance(config, dict):
            raise SpendGuardConfigError(
                "SpendGuardGuardrail.from_config expects a dict; "
                f"got {type(config).__name__}."
            )

        tenant_id = str(config.get("tenant_id") or "").strip()
        if not tenant_id:
            raise SpendGuardConfigError(
                "missing config key 'tenant_id' — required for "
                "SpendGuardGuardrail.from_config(). Add it to your "
                "proxy_config.yaml guardrail entry."
            )

        sidecar_address = (
            str(config.get("sidecar_address") or "").strip()
            or str(config.get("socket_path") or "").strip()
            or str(config.get("sidecar_uds") or "").strip()
        )
        if not sidecar_address:
            raise SpendGuardConfigError(
                "missing config key 'sidecar_address' — required for "
                "SpendGuardGuardrail.from_config(). Add it to your "
                "proxy_config.yaml guardrail entry "
                "('socket_path' / 'sidecar_uds' are legacy aliases)."
            )

        api_key_raw = config.get("api_key")
        api_key = api_key_raw if api_key_raw else None

        disabled_raw = config.get("disabled")
        if isinstance(disabled_raw, bool):
            disabled: bool = disabled_raw
        else:
            disabled = cls._parse_disabled(
                None if disabled_raw is None else str(disabled_raw),
            )

        timeout_raw = config.get("proxy_timeout_ms")
        if timeout_raw is None:
            proxy_timeout_ms: int = 5000
        else:
            try:
                proxy_timeout_ms = int(timeout_raw)
            except (TypeError, ValueError) as exc:
                raise SpendGuardConfigError(
                    "invalid config key 'proxy_timeout_ms'="
                    f"{timeout_raw!r} — must be an integer millisecond "
                    "value (e.g. 5000)."
                ) from exc

        # SLICE 4b: same shape as `_read_env_config`. Dict keys may
        # omit any of these (None / missing) and the construction
        # pipeline behaves identically: with `resolver_module` set,
        # operator factory dispatches; otherwise the 3 + 4 vars build
        # the single-tenant closure.
        resolver_module = config.get("resolver_module") or None
        if isinstance(resolver_module, str):
            resolver_module = resolver_module.strip() or None

        def _opt_str(key: str) -> str | None:
            raw = config.get(key)
            if raw is None:
                return None
            stripped = str(raw).strip()
            return stripped or None

        return {
            "tenant_id": tenant_id,
            "sidecar_address": sidecar_address,
            "api_key": api_key,
            "disabled": disabled,
            "proxy_timeout_ms": proxy_timeout_ms,
            "resolver_module": resolver_module,
            "budget_id": _opt_str("budget_id"),
            "window_instance_id": _opt_str("window_instance_id"),
            "unit_id": _opt_str("unit_id"),
            "pricing_version": _opt_str("pricing_version"),
            "fx_rate_version": _opt_str("fx_rate_version"),
            "unit_conversion_version": _opt_str("unit_conversion_version"),
            "price_snapshot_hash_hex": _opt_str("price_snapshot_hash_hex"),
        }

    @classmethod
    def _from_parsed_config(
        cls, parsed: dict[str, Any],
    ) -> SpendGuardGuardrail:
        """Construct a guardrail from the parsed-config dict shape.

        Single construction pipeline both ``from_env`` and
        ``from_config`` route through, so the disabled-mode + lazy
        delegate + resolver-wiring semantics stay identical regardless
        of where the config came from.

        SLICE 4b dispatch:
            * ``parsed["resolver_module"]`` set → import +
              triple-factory dispatch via ``_load_resolver_triple``.
              The 3 budget-binding + 4 pricing-version vars are NOT
              consulted on this path (operator factory owns binding
              construction; the U08 invariant).
            * Otherwise, any of the 3 + 4 single-tenant vars set →
              build a closure resolver + reconciler from those vars.
              ``_validate_budget_binding`` runs at factory time so
              empty fields fail-closed before the first hook fires.
            * Otherwise (all the SLICE 4b vars unset) → the SLICE 1
              skeleton resolver stays put. Backward-compat for the
              SLICE 4 baseline tests + adapter authors who supply
              resolvers via ``from_kwargs``.
        """
        instance = cls(
            socket_path=parsed["sidecar_address"],
            tenant_id=parsed["tenant_id"],
        )

        # The default skeleton resolvers wired by __init__ would surface
        # a `SpendGuardConfigError("budget_resolver returned None")` on
        # first hook invocation. SLICE 4 left them in place; SLICE 4b
        # replaces them when any of the resolver / binding env vars are
        # supplied. The disabled path below short-circuits before any
        # resolver fires, matching the TS SDK contract.

        if parsed["disabled"]:
            instance._install_disabled_delegate()
        else:
            cls._wire_resolver_from_parsed(instance, parsed)

        # Stash parsed values on the instance so SLICE 5's bootstrap
        # validator can inspect what was applied without re-reading
        # the env. Underscored to keep the public surface stable.
        instance._config_api_key = parsed["api_key"]
        instance._config_disabled = parsed["disabled"]
        instance._config_proxy_timeout_ms = parsed["proxy_timeout_ms"]
        instance._config_resolver_module = parsed.get("resolver_module")
        instance._config_budget_id = parsed.get("budget_id")
        instance._config_window_instance_id = parsed.get("window_instance_id")
        instance._config_unit_id = parsed.get("unit_id")
        instance._config_pricing_version = parsed.get("pricing_version")
        instance._config_fx_rate_version = parsed.get("fx_rate_version")
        instance._config_unit_conversion_version = parsed.get(
            "unit_conversion_version",
        )
        instance._config_price_snapshot_hash_hex = parsed.get(
            "price_snapshot_hash_hex",
        )

        return instance

    # -------------------------------------------------------------------
    # SLICE 4b — resolver-module + single-tenant binding wiring.
    #
    # Two paths:
    #   1. `SPENDGUARD_RESOLVER_MODULE=pkg.mod:fn_name` →
    #      `_load_resolver_triple` imports the module, looks up the
    #      attribute, invokes it as a zero-arg factory, and expects a
    #      triple `(BudgetResolver, ClaimEstimator | None,
    #      ClaimReconciler)`. Empty / missing / non-callable → raise
    #      `SpendGuardConfigError` naming the env var.
    #   2. Single-tenant default: build a closure resolver from the
    #      3 budget-binding + 4 pricing-version env vars (mirrors
    #      `examples/litellm-proxy-composite/spendguard_litellm_proxy_callback.py`)
    #      and a closure reconciler that reads
    #      `response.usage.completion_tokens` (OpenAI shape).
    # -------------------------------------------------------------------

    @classmethod
    def _wire_resolver_from_parsed(
        cls, instance: SpendGuardGuardrail, parsed: dict[str, Any],
    ) -> None:
        """Replace the SLICE 1 skeleton delegate with one that has a
        real resolver / reconciler wired per the parsed config.

        Called by ``_from_parsed_config`` when not in disabled mode.
        No-op when neither the resolver-module nor any single-tenant
        var is set (preserves SLICE 4 baseline behaviour).
        """
        has_resolver_module = bool(parsed.get("resolver_module"))
        single_tenant_vars = [
            "budget_id",
            "window_instance_id",
            "unit_id",
            "pricing_version",
            "fx_rate_version",
            "unit_conversion_version",
            "price_snapshot_hash_hex",
        ]
        has_any_single_tenant = any(parsed.get(k) for k in single_tenant_vars)

        if not has_resolver_module and not has_any_single_tenant:
            # Adapter-author path: leave skeleton resolvers in place;
            # operator will supply via `from_kwargs` or a yaml entry
            # that ships an explicit resolver.
            return

        if has_resolver_module:
            # U08 invariant: when resolver-module is set, the
            # single-tenant vars are NOT consulted. The operator factory
            # is fully responsible for binding construction.
            resolver, estimator, reconciler = cls._load_resolver_triple(
                str(parsed["resolver_module"]),
            )
        else:
            cls._validate_budget_binding_fields(parsed)
            binding = cls._build_binding_from_parsed(parsed)
            resolver = cls._make_single_tenant_resolver(binding)
            estimator = None  # delegate falls back to _default_estimator
            reconciler = cls._make_default_reconciler(binding)

        # Swap the SLICE 1 skeleton delegate for one wired with the
        # resolved trio. The lazy loop-binding invariant is preserved
        # — `_LoopBoundCallback.__init__` does NOT create a gRPC
        # channel; the first hook call binds on the serving loop.
        instance._delegate = _LoopBoundCallback(
            socket_path=parsed["sidecar_address"],
            tenant_id=parsed["tenant_id"],
            budget_resolver=resolver,
            claim_estimator=estimator,
            claim_reconciler=reconciler,
        )

    @staticmethod
    def _load_resolver_triple(
        resolver_module: str,
    ) -> tuple[BudgetResolver, ClaimEstimator | None, ClaimReconciler]:
        """Resolve ``pkg.mod:fn_name`` (or legacy ``pkg.mod.fn_name``)
        into a triple of callables.

        The operator-supplied factory is called with zero args and is
        expected to return a 3-tuple
        ``(resolver, estimator | None, reconciler)``. ``estimator`` may
        be ``None`` so the delegate falls back to ``_default_estimator``.

        Raises:
            SpendGuardConfigError: module / attribute / shape problems
                are reported as config errors naming the env var so
                operators can fix the deployment quickly.
        """
        spec = resolver_module.strip()
        if not spec:
            raise SpendGuardConfigError(
                "SPENDGUARD_RESOLVER_MODULE is empty; expected "
                "'pkg.mod:fn_name' triple-factory spec."
            )

        if ":" in spec:
            module_path, _, attr_name = spec.partition(":")
        elif "." in spec:
            # Legacy dot-only syntax: `pkg.mod.fn_name`. Split on the
            # last dot. Documented as a fallback so the task prompt's
            # smoke-test spelling (`spendguard.budget.resolver.X`)
            # still works without a colon.
            module_path, _, attr_name = spec.rpartition(".")
        else:
            raise SpendGuardConfigError(
                f"invalid SPENDGUARD_RESOLVER_MODULE={spec!r} — expected "
                "'pkg.mod:fn_name' (colon separator). Got a single "
                "identifier with no module path."
            )

        module_path = module_path.strip()
        attr_name = attr_name.strip()
        if not module_path or not attr_name:
            raise SpendGuardConfigError(
                f"invalid SPENDGUARD_RESOLVER_MODULE={spec!r} — both "
                "module path and attribute name must be non-empty "
                "(e.g. 'myapp.spendguard:make_triple')."
            )

        try:
            module = importlib.import_module(module_path)
        except ImportError as exc:
            raise SpendGuardConfigError(
                f"SPENDGUARD_RESOLVER_MODULE={spec!r}: cannot import "
                f"module {module_path!r} — {exc}. Check PYTHONPATH and "
                "the module spelling."
            ) from exc

        try:
            factory = getattr(module, attr_name)
        except AttributeError as exc:
            raise SpendGuardConfigError(
                f"SPENDGUARD_RESOLVER_MODULE={spec!r}: module "
                f"{module_path!r} has no attribute {attr_name!r}. "
                "The triple-factory must be importable."
            ) from exc

        if not callable(factory):
            raise SpendGuardConfigError(
                f"SPENDGUARD_RESOLVER_MODULE={spec!r}: "
                f"{module_path}.{attr_name} is not callable. The "
                "triple-factory must be a zero-arg function returning "
                "(resolver, estimator | None, reconciler)."
            )

        try:
            triple = factory()
        except Exception as exc:
            raise SpendGuardConfigError(
                f"SPENDGUARD_RESOLVER_MODULE={spec!r}: triple-factory "
                f"raised at proxy boot — {exc!r}. Fix the factory or "
                "switch to the single-tenant env-var path."
            ) from exc

        if not (
            isinstance(triple, tuple)
            and len(triple) == 3
            and callable(triple[0])
            and (triple[1] is None or callable(triple[1]))
            and callable(triple[2])
        ):
            raise SpendGuardConfigError(
                f"SPENDGUARD_RESOLVER_MODULE={spec!r}: triple-factory "
                "must return a 3-tuple (resolver, estimator | None, "
                f"reconciler) of callables; got {triple!r}."
            )

        resolver, estimator, reconciler = triple
        return resolver, estimator, reconciler

    @staticmethod
    def _validate_budget_binding_fields(parsed: dict[str, Any]) -> None:
        """Reject partial / inconsistent single-tenant config.

        Mirror of ``litellm.py`` lines 306-315 + the unit-id check in
        ``_validate_claim_against_binding``: each of ``budget_id`` /
        ``window_instance_id`` / ``unit_id`` MUST be set when the
        single-tenant default-resolver path is taken. Empty pricing
        fields fail-closed too so commit-side audit context is never
        empty.
        """
        required = {
            "budget_id": "SPENDGUARD_BUDGET_ID",
            "window_instance_id": "SPENDGUARD_WINDOW_INSTANCE_ID",
            "unit_id": "SPENDGUARD_UNIT_ID",
            "pricing_version": "SPENDGUARD_PRICING_VERSION",
            "fx_rate_version": "SPENDGUARD_FX_RATE_VERSION",
            "unit_conversion_version": "SPENDGUARD_UNIT_CONVERSION_VERSION",
            "price_snapshot_hash_hex": "SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX",
        }
        missing = [env_name for key, env_name in required.items() if not parsed.get(key)]
        if missing:
            raise SpendGuardConfigError(
                "incomplete single-tenant SpendGuard binding: "
                f"missing {', '.join(missing)}. Set every variable "
                "in the SLICE 4b binding set, or set "
                "SPENDGUARD_RESOLVER_MODULE to dispatch through an "
                "operator-supplied triple-factory."
            )

    @staticmethod
    def _build_binding_from_parsed(parsed: dict[str, Any]) -> BudgetBinding:
        """Build a ``BudgetBinding`` from the 3 + 4 single-tenant vars.

        Mirrors field-by-field the binding shape used by
        ``examples/litellm-proxy-composite/spendguard_litellm_proxy_callback.py``::

            UnitRef(unit_id, token_kind="output_token", model_family="gpt-4")
            PricingFreeze(pricing_version, fx_rate_version,
                          unit_conversion_version, price_snapshot_hash)

        The ``token_kind`` / ``model_family`` defaults match the
        example callback's hard-coded choice; operators who need
        different unit semantics must switch to the resolver-module
        path (SLICE 4b future-compat for SLICE 5's yaml entry).
        """
        # Imported here (not at module top) to keep the SLICE 1 import
        # surface unchanged — the proto module is otherwise unused by
        # the guardrail wrapper.
        from spendguard._proto.spendguard.common.v1 import common_pb2

        try:
            price_snapshot_hash = bytes.fromhex(
                str(parsed["price_snapshot_hash_hex"]),
            )
        except ValueError as exc:
            raise SpendGuardConfigError(
                "invalid SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX="
                f"{parsed['price_snapshot_hash_hex']!r} — must be a "
                "hex-encoded snapshot digest (e.g. 'a1b2c3...'). "
                f"Decode error: {exc}."
            ) from exc

        unit_ref = common_pb2.UnitRef(
            unit_id=str(parsed["unit_id"]),
            token_kind="output_token",  # noqa: S106 — proto field, not a credential
            model_family="gpt-4",
        )
        pricing = common_pb2.PricingFreeze(
            pricing_version=str(parsed["pricing_version"]),
            price_snapshot_hash=price_snapshot_hash,
            fx_rate_version=str(parsed["fx_rate_version"]),
            unit_conversion_version=str(parsed["unit_conversion_version"]),
        )
        return BudgetBinding(
            budget_id=str(parsed["budget_id"]),
            window_instance_id=str(parsed["window_instance_id"]),
            unit=unit_ref,
            pricing=pricing,
        )

    @staticmethod
    def _make_single_tenant_resolver(
        binding: BudgetBinding,
    ) -> BudgetResolver:
        """Closure resolver: ignore the per-request ``ResolverContext``
        and return the env-bound ``BudgetBinding`` every call.

        Matches the shape of ``_resolve`` in
        ``examples/litellm-proxy-composite/spendguard_litellm_proxy_callback.py``:
        single-tenant production deployments freeze one binding at
        proxy boot. Multi-tenant operators must switch to
        ``SPENDGUARD_RESOLVER_MODULE`` so the resolver can inspect
        ``ctx.user_api_key_dict.team_id``.
        """

        def _resolver(_ctx: Any) -> BudgetBinding:
            return binding

        return _resolver

    @staticmethod
    def _make_default_reconciler(
        binding: BudgetBinding,
    ) -> ClaimReconciler:
        """Closure reconciler: read ``response.usage.completion_tokens``
        (OpenAI shape; LiteLLM normalises every provider into the same
        shape) and emit a single ``BudgetClaim`` under the env-bound
        binding's unit semantics.

        ``max(tokens, 1)`` keeps the commit row non-empty so the
        downstream stats aggregator never reads a zero-amount commit
        as a missing commit. Matches the example callback's
        ``_reconcile`` behaviour field-by-field.
        """
        from spendguard._proto.spendguard.common.v1 import common_pb2

        def _reconciler(_ctx: Any, response_obj: Any) -> list[Any]:
            usage = getattr(response_obj, "usage", None)
            tokens = int(getattr(usage, "completion_tokens", 0) or 0)
            return [
                common_pb2.BudgetClaim(
                    budget_id=binding.budget_id,
                    unit=binding.unit,
                    amount_atomic=str(max(tokens, 1)),
                    direction=common_pb2.BudgetClaim.DEBIT,
                    window_instance_id=binding.window_instance_id,
                ),
            ]

        return _reconciler

    def _install_disabled_delegate(self) -> None:
        """Install a no-op delegate so every hook short-circuits.

        Mirrors the TS SDK's disabled-mode pattern: the guardrail
        remains constructable + introspectable (so LiteLLM's registry
        + ops dashboards still see it) but the three hook methods do
        not call into the sidecar gRPC channel — they return None
        without touching any IO. Used by ``from_env`` /
        ``from_config`` when ``SPENDGUARD_DISABLED`` is truthy.
        """
        self._delegate = _NoopGuardrailDelegate()  # type: ignore[assignment]


# ---------------------------------------------------------------------------
# SLICE 5 — operator-facing ``guardrails:`` registry entry factory.
#
# `spendguard_guardrail_factory` is what an operator references from
# ``proxy_config.yaml``'s ``guardrails: [{litellm_params: {guardrail: ...}}]``
# entry. It MUST tolerate two calling conventions:
#
#   1. Direct programmatic call (tests, ad-hoc bootstrap):
#          spendguard_guardrail_factory({"tenant_id": "...", ...})
#      — the dict is the parsed litellm_params from yaml.
#
#   2. LiteLLM's `get_instance_fn` resolves the dotted path
#      `spendguard.integrations.litellm_guardrail.spendguard_guardrail_factory`
#      to THIS function and instantiates it like a class:
#          factory(guardrail_name="spendguard",
#                   event_hook="pre_call",
#                   default_on=True,
#                   **extra_litellm_params)
#      — LiteLLM splices `guardrail_name` / `event_hook` /
#      `default_on` plus every other key in `litellm_params` (yaml
#      block) as kwargs.
#
# Path 2 is verified against
# `litellm/proxy/guardrails/guardrail_registry.py:516-571`
# (`initialize_custom_guardrail` → `_guardrail_class(...)`).
#
# Bootstrap validator (§5.4 Major from review-standards): missing
# required keys (`tenant_id` / `sidecar_address`) OR a non-importable
# `resolver_module` surface a `SpendGuardConfigError` at boot time —
# BEFORE the first hook runs. `from_config` already does this for the
# required-key paths; the resolver-module path is exercised at
# construction (not lazily on first hook) so an operator typo never
# wedges first request.
# ---------------------------------------------------------------------------


# Module path used by LiteLLM's `get_instance_fn` to resolve the
# factory at proxy boot. Pinned here as a constant so
# `test_factory_function_importable_via_module_path` can introspect
# the exact spelling the README + yaml example reference.
SPENDGUARD_GUARDRAIL_MODULE_PATH = (
    "spendguard.integrations.litellm_guardrail.spendguard_guardrail_factory"
)

# Env var → config dict key mapping used by `_merge_inline_and_env`.
# The right-hand side mirrors the key spelling SLICE 4 / 4b's
# `_coerce_config_dict` consumes; the left-hand side is the env var
# spelling SLICE 4 / 4b's `_read_env_config` consumes. We keep both
# spellings consistent so an operator reading the yaml example can
# guess the env var without grepping the SDK.
_ENV_VAR_TO_CONFIG_KEY: dict[str, str] = {
    "SPENDGUARD_TENANT_ID": "tenant_id",
    "SPENDGUARD_SIDECAR_ADDRESS": "sidecar_address",
    "SPENDGUARD_API_KEY": "api_key",
    "SPENDGUARD_DISABLED": "disabled",
    "SPENDGUARD_PROXY_TIMEOUT_MS": "proxy_timeout_ms",
    "SPENDGUARD_RESOLVER_MODULE": "resolver_module",
    "SPENDGUARD_BUDGET_ID": "budget_id",
    "SPENDGUARD_WINDOW_INSTANCE_ID": "window_instance_id",
    "SPENDGUARD_UNIT_ID": "unit_id",
    "SPENDGUARD_PRICING_VERSION": "pricing_version",
    "SPENDGUARD_FX_RATE_VERSION": "fx_rate_version",
    "SPENDGUARD_UNIT_CONVERSION_VERSION": "unit_conversion_version",
    "SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX": "price_snapshot_hash_hex",
}

# Legacy env var alias accepted by SLICE 4's `_read_env_config` —
# routed into `sidecar_address` only when the canonical var is unset.
# Kept separate so `_merge_inline_and_env` does not regress operators
# still using the legacy spelling.
_LEGACY_SIDECAR_UDS_ENV = "SPENDGUARD_SIDECAR_UDS"

# Keys LiteLLM's registry adds to the kwargs splice that are NOT
# SpendGuard config — they describe the registry binding itself and
# must be filtered before we hand the dict to `from_config`.
# `guardrail` is the dotted module path of THIS factory; `mode` is the
# event-hook spelling (`pre_call` / etc.); `default_on` toggles the
# registry-side default; `event_hook` is LiteLLM's internal alias for
# `mode` passed by `_guardrail_class(...)` callsite.
_REGISTRY_KEYS_TO_DROP: frozenset[str] = frozenset({
    "guardrail",
    "mode",
    "event_hook",
    "default_on",
    "guardrail_name",
    "supported_event_hooks",
    "skip_system_message_in_guardrail",
    "skip_tool_message_in_guardrail",
})


def _merge_inline_and_env(
    litellm_params: dict[str, Any],
) -> dict[str, Any]:
    """Merge inline yaml params with environment-variable fallbacks.

    Inline keys win over env vars (operator's explicit yaml is
    authoritative). Missing keys are filled from the corresponding
    ``SPENDGUARD_*`` env var when present.

    Filters out LiteLLM-registry binding keys (``guardrail`` / ``mode``
    / ``event_hook`` / ``default_on`` / ``guardrail_name``) so the
    merged dict shape matches what ``from_config`` expects.

    Returns:
        A dict suitable for ``SpendGuardGuardrail.from_config``. The
        dict is a fresh object — never mutates the caller's
        ``litellm_params``.
    """
    if not isinstance(litellm_params, dict):
        raise SpendGuardConfigError(
            "spendguard_guardrail_factory expects a dict of litellm_params; "
            f"got {type(litellm_params).__name__}. Operators using "
            "proxy_config.yaml should reference this function via the "
            f"dotted module path: {SPENDGUARD_GUARDRAIL_MODULE_PATH!r}.",
        )

    # Step 1: copy inline params with registry-binding keys stripped
    # so they never leak into `from_config` and surface as a
    # confusing "unknown key" error from a future hardener pass.
    merged: dict[str, Any] = {
        key: value
        for key, value in litellm_params.items()
        if key not in _REGISTRY_KEYS_TO_DROP
    }

    # Step 2: fill missing keys from env vars. We treat a None / empty
    # string inline value as "missing" so a yaml entry like
    # `tenant_id: ""` (operator mistake) does not silently mask an
    # env var that would otherwise satisfy the binding — instead the
    # env var takes over. Explicit non-empty inline values win.
    for env_var, config_key in _ENV_VAR_TO_CONFIG_KEY.items():
        inline_value = merged.get(config_key)
        if _is_explicit_value(inline_value):
            continue
        env_value = os.environ.get(env_var, "").strip()
        if env_value:
            merged[config_key] = env_value

    # Legacy alias: route SPENDGUARD_SIDECAR_UDS into `sidecar_address`
    # only when neither inline nor the canonical env var supplied a
    # value. Mirrors SLICE 4's `_read_env_config` fallback.
    if not _is_explicit_value(merged.get("sidecar_address")):
        legacy = os.environ.get(_LEGACY_SIDECAR_UDS_ENV, "").strip()
        if legacy:
            merged["sidecar_address"] = legacy

    return merged


def _is_explicit_value(value: Any) -> bool:
    """A value is considered "explicit" when the operator clearly meant
    to set it. None / empty-string / whitespace-only do NOT count as
    explicit so env-var fallback fires. Boolean False DOES count
    (otherwise `disabled: false` would silently mask an env var).
    """
    if value is None:
        return False
    if isinstance(value, str):
        return bool(value.strip())
    return True


def spendguard_guardrail_factory(
    litellm_params: dict[str, Any] | None = None,
    /,
    **kwargs: Any,
) -> SpendGuardGuardrail:
    """LiteLLM ``proxy_config.yaml`` guardrail registry factory.

    Called by LiteLLM during proxy boot when ``proxy_config.yaml``'s
    ``guardrails:`` section references the dotted module path
    ``spendguard.integrations.litellm_guardrail.spendguard_guardrail_factory``.
    Reads SpendGuard config keys from ``litellm_params`` (inline yaml)
    with env-var fallback for missing keys.

    Two calling conventions:

    1. Direct programmatic call (a single dict argument)::

           spendguard_guardrail_factory({
               "tenant_id": "my-tenant",
               "sidecar_address": "unix:///run/spendguard.sock",
           })

    2. LiteLLM registry call (kwargs splice; see
       ``litellm/proxy/guardrails/guardrail_registry.py:563-568``)::

           spendguard_guardrail_factory(
               guardrail_name="spendguard",
               event_hook="pre_call",
               default_on=True,
               tenant_id="my-tenant",
               sidecar_address="unix:///run/spendguard.sock",
           )

       LiteLLM's registry filters ``guardrail`` / ``mode`` /
       ``default_on`` itself but passes through every other key in the
       yaml ``litellm_params:`` block as ``**extra_params``. Our
       factory tolerates the binding-key splice
       (``guardrail_name`` / ``event_hook`` / ``supported_event_hooks``
       / ``skip_system_message_in_guardrail`` etc.) by stripping them
       inside ``_merge_inline_and_env`` before delegating to
       ``SpendGuardGuardrail.from_config``.

    Inline config keys (all optional; env vars fill gaps):
        * ``tenant_id`` (required if env unset)
        * ``sidecar_address`` (required if env unset; legacy
          aliases ``socket_path`` / ``sidecar_uds`` honoured)
        * ``api_key`` / ``disabled`` / ``proxy_timeout_ms``
        * ``resolver_module``
        * ``budget_id`` / ``window_instance_id`` / ``unit_id``
        * ``pricing_version`` / ``fx_rate_version`` /
          ``unit_conversion_version`` / ``price_snapshot_hash_hex``

    Bootstrap validator (review-standards §5.4 Major): when required
    keys are absent from BOTH inline and env, OR when
    ``resolver_module`` is set but the module is non-importable,
    raises ``SpendGuardConfigError`` at boot time (NOT first hook
    call). This is the "fail-closed at boot" gate that surfaces
    misconfiguration BEFORE the first request reaches the proxy.

    Returns:
        A fully constructed ``SpendGuardGuardrail`` ready to register
        with the LiteLLM proxy. Non-singleton — calling the factory
        twice returns two distinct instances (no module-level
        mutable state; mirrors review-standards 1.4).

    Raises:
        SpendGuardConfigError: required key missing from both inline
            and env; or ``resolver_module`` is set but cannot be
            imported / does not satisfy the triple-factory contract;
            or ``litellm_params`` is not a dict.
    """
    # Accept both `factory(dict)` and `factory(key=value, ...)` calling
    # conventions. When LiteLLM passes a kwargs splice, the positional
    # `litellm_params` is None and we treat the kwargs dict as the
    # config payload. When tests / ad-hoc callers pass a single dict,
    # we use it directly.
    if litellm_params is None:
        litellm_params = kwargs
    elif kwargs:
        # Defensive: a caller that supplies both a dict AND kwargs is
        # ambiguous. We merge kwargs into the dict (kwargs win) so
        # LiteLLM's splice — which always lands in kwargs — overrides
        # any inline dict shipped through an alternate code path. The
        # SLICE 4b "kwargs are authoritative" contract carries over.
        litellm_params = {**litellm_params, **kwargs}

    parsed = _merge_inline_and_env(litellm_params)
    return SpendGuardGuardrail.from_config(parsed)


class _NoopGuardrailDelegate:
    """No-op stand-in for ``_LoopBoundCallback`` in disabled mode.

    Implements the same three async surface methods
    (``async_pre_call_hook`` / ``async_log_success_event`` /
    ``async_log_failure_event``) so ``SpendGuardGuardrail``'s hooks
    can delegate without conditional branching. Each method is a
    coroutine that returns ``None`` immediately — no gRPC channel,
    no event loop affinity, no audit row.

    Used when ``SPENDGUARD_DISABLED`` (or the config dict's
    ``disabled``) is truthy. Adapter authors flip the flag for
    canary deploys / staged rollouts without removing the guardrail
    from ``proxy_config.yaml``.
    """

    # Exposed for test introspection — assertions can check the
    # delegate type to confirm disabled-mode wiring.
    disabled = True

    async def async_pre_call_hook(self, *_a: Any, **_kw: Any) -> None:
        return None

    async def async_log_success_event(self, *_a: Any, **_kw: Any) -> None:
        return None

    async def async_log_failure_event(self, *_a: Any, **_kw: Any) -> None:
        return None


__all__ = [
    "SPENDGUARD_GUARDRAIL_MODULE_PATH",
    "SpendGuardGuardrail",
    "spendguard_guardrail_factory",
]
