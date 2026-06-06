# ruff: noqa: ANN401  # LiteLLM's CustomGuardrail interface uses untyped Any
"""LiteLLM proxy ``CustomGuardrail`` plugin — SLICE 1 + 2 + 3 + 4 wired.

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

import logging
import os
from typing import Any

from spendguard.errors import SpendGuardConfigError

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

        return {
            "tenant_id": tenant_id,
            "sidecar_address": sidecar_address,
            "api_key": api_key,
            "disabled": disabled,
            "proxy_timeout_ms": proxy_timeout_ms,
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

        return {
            "tenant_id": tenant_id,
            "sidecar_address": sidecar_address,
            "api_key": api_key,
            "disabled": disabled,
            "proxy_timeout_ms": proxy_timeout_ms,
        }

    @classmethod
    def _from_parsed_config(
        cls, parsed: dict[str, Any],
    ) -> SpendGuardGuardrail:
        """Construct a guardrail from the parsed-config dict shape.

        Single construction pipeline both ``from_env`` and
        ``from_config`` route through, so the disabled-mode + lazy
        delegate semantics stay identical regardless of where the
        config came from.
        """
        instance = cls(
            socket_path=parsed["sidecar_address"],
            tenant_id=parsed["tenant_id"],
        )

        # The default skeleton resolvers wired by __init__ would surface
        # a `SpendGuardConfigError("budget_resolver returned None")` on
        # first hook invocation. SLICE 4 honours operator intent: a
        # `from_env`-constructed guardrail without an explicit resolver
        # is the EXPECTED state for adapter-author wiring — operators
        # plug a real resolver via the future SLICE 5 yaml entry or via
        # `from_kwargs`. The disabled path below short-circuits before
        # the skeleton resolver fires, matching the TS SDK contract.

        if parsed["disabled"]:
            instance._install_disabled_delegate()

        # Stash parsed values on the instance so SLICE 5's bootstrap
        # validator can inspect what was applied without re-reading
        # the env. Underscored to keep the public surface stable.
        instance._config_api_key = parsed["api_key"]
        instance._config_disabled = parsed["disabled"]
        instance._config_proxy_timeout_ms = parsed["proxy_timeout_ms"]

        return instance

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


__all__ = ["SpendGuardGuardrail"]
