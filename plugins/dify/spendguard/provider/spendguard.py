"""SpendGuard Dify ``ModelProvider`` — credential validation entrypoint.

review-standards.md 3.9 + INV-4: ``validate_credentials`` MUST issue a
1-token reserve+release roundtrip against the sidecar, NOT only an
upstream-credential probe. This catches sidecar misconfig at install
time, before the first runtime LLM call.
"""

from __future__ import annotations

import logging
from collections.abc import Mapping
from types import SimpleNamespace

from dify_plugin import ModelProvider
from dify_plugin.errors.model import CredentialsValidateFailedError
from spendguard.errors import (
    DecisionDenied,
    SidecarUnavailable,
    SpendGuardConfigError,
    SpendGuardError,
)

log = logging.getLogger("spendguard.dify_plugin.provider")


class SpendguardProvider(ModelProvider):
    """SpendGuard model provider class.

    Dify instantiates this once per workspace and calls
    ``validate_provider_credentials`` after the operator submits the
    provider form. We use that hook to run a 1-token reserve+release
    against the SpendGuard sidecar (INV-4) so install fails closed if
    SpendGuard wiring is broken — instead of the first runtime LLM call
    surprising the user.

    The legacy alias ``SpendGuardProvider`` (PascalCase) is exposed at
    the bottom of this file for direct ``from provider.spendguard import
    SpendGuardProvider`` imports from tests; the Dify plugin daemon
    resolves the class via the manifest, not the import path.
    """

    def validate_provider_credentials(self, credentials: Mapping) -> None:
        """1-token reserve+release roundtrip — review-standards.md 3.9."""
        # Lazy imports so module load doesn't hit the SDK / event loop.
        from models.llm._DifyReservation import (
            DifyCallContext,
            _DifyReservation,
        )
        from models.llm.spendguard_llm import _DaemonLoop

        # Step 1: build a reservation. ``_DifyReservation.__init__``
        # checks env vars + raises ``SpendGuardConfigError`` naming the
        # missing var (review-standards.md 3.2).
        try:
            reservation = _DifyReservation()
        except SpendGuardConfigError as exc:
            raise CredentialsValidateFailedError(
                f"spendguard sidecar env misconfigured: {exc}",
            ) from exc

        # Step 2: surface required credential keys early so the operator
        # sees what's missing before we hit the sidecar.
        for key in (
            "spendguard_budget_id",
            "spendguard_window_instance_id",
            "upstream_provider",
        ):
            if not str(credentials.get(key) or "").strip():
                raise CredentialsValidateFailedError(
                    f"credentials.{key} is missing or empty",
                )
        upstream = str(credentials.get("upstream_provider", "")).strip().lower()
        if upstream == "openai" and not (
            credentials.get("openai_api_key")
            or credentials.get("upstream_api_key")
        ):
            raise CredentialsValidateFailedError(
                "credentials.openai_api_key is missing or empty",
            )

        # Step 3: 1-token reserve + release roundtrip. Catches sidecar
        # reachability + tenant assertion + binding validation in one
        # call. Failure -> CredentialsValidateFailedError so Dify shows
        # the operator a clear install-time error.
        loop = _DaemonLoop.get()
        ctx = DifyCallContext(
            workspace_id="validate-credentials",
            app_id="install-probe",
            model="validate/probe",
            prompt_messages=[],
            stream=False,
            credentials=credentials,
            user=None,
        )
        try:
            handle = loop.run(
                reservation.reserve(ctx, estimated_amount_atomic="1"),
                timeout=10.0,
            )
        except DecisionDenied as exc:
            # DENY at install time is acceptable — the wiring is proven.
            # We still treat it as a failure surface so the operator
            # knows the chosen budget is denying probes; configure a
            # validation-friendly budget.
            raise CredentialsValidateFailedError(
                f"spendguard sidecar denied install probe "
                f"(decision_id={exc.decision_id}): {exc}",
            ) from exc
        except SidecarUnavailable as exc:
            raise CredentialsValidateFailedError(
                f"spendguard sidecar unavailable: {exc}",
            ) from exc
        except SpendGuardConfigError as exc:
            raise CredentialsValidateFailedError(
                f"spendguard binding/credentials invalid: {exc}",
            ) from exc
        except SpendGuardError as exc:
            raise CredentialsValidateFailedError(
                f"spendguard sidecar error during install probe: {exc}",
            ) from exc

        # Step 4: release the reservation immediately. Release errors are
        # swallowed inside release_failure (TTL sweep is the durable
        # backstop) so install never blocks on release.
        try:
            loop.run(
                reservation.release_failure(
                    handle, SimpleNamespace(__class__=type("ProbeRelease", (), {})),
                ),
                timeout=5.0,
            )
        except Exception as rel_exc:
            log.warning(
                "spendguard: install-probe release submission failed "
                "err=%r; reservation will TTL-sweep.", rel_exc,
            )


# Backwards-friendly alias (PascalCase) for explicit imports.
SpendGuardProvider = SpendguardProvider
