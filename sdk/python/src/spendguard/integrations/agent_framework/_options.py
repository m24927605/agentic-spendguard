"""Options dataclass for the MAF SpendGuard middleware.

Mirrors the .NET ``SpendGuardOptions`` POCO from
``sdk/dotnet/src/Spendguard.AgentFramework/SpendGuardOptions.cs`` per
review-standards.md §2.3 P2 (public-API name parity, case-translated):

    .NET                          Python
    ---------------------------   --------------------------
    SocketPath                    sidecar_socket_path
    TenantId                      tenant_id
    BudgetId                      budget_id
    WindowInstanceId              window_instance_id
    OnSidecarUnavailable          on_sidecar_unavailable
    ClaimEstimator                claim_estimator (on middleware)

The Python options dataclass intentionally only holds the *config* shape;
the ``client``, ``unit``, ``pricing``, and ``claim_estimator`` live on
the ``SpendGuardMiddleware`` constructor because they are not POCO-able
(grpc channel, proto message, callable).
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from ._errors import SpendGuardConfigError

# Literal type so callers get IDE completion + the validate step has a
# single source of truth for the allowed values.
OnSidecarUnavailable = Literal["deny", "allow"]
"""Behavior when the sidecar UDS is unreachable.

``"deny"`` (default per ADR-005): raise ``SidecarUnavailable`` and short-circuit
the LLM call. ``"allow"`` opts into fail-open with a warning logged on every
unreachable request. The .NET side calls these ``Deny`` / ``Allow`` per the
``OnSidecarUnavailable`` enum.
"""


@dataclass(frozen=True, slots=True)
class SpendGuardAgentFrameworkOptions:
    """Per-middleware configuration for the MAF integration.

    Attributes:
        tenant_id: SpendGuard tenant scope. REQUIRED — validated non-empty.
        budget_id: Budget the reservation debits. REQUIRED — validated non-empty.
            Reviewer N1 per tests.md §7.
        window_instance_id: Time-window scope on the budget.
            REQUIRED — validated non-empty.
        sidecar_socket_path: Path to the sidecar UDS. REQUIRED — validated
            non-empty per N2.
        on_sidecar_unavailable: ``"deny"`` (default, fail-closed per ADR-005)
            or ``"allow"`` (opt-in fail-open with logged warning).

    Raises:
        SpendGuardConfigError: any required string field empty / whitespace.
    """

    tenant_id: str
    budget_id: str
    window_instance_id: str
    sidecar_socket_path: str = "/var/run/spendguard/sidecar.sock"
    on_sidecar_unavailable: OnSidecarUnavailable = "deny"
    unit_id: str | None = None
    """Canonical-truth UUID of the ledger unit row.

    When provided, the middleware threads it through to
    ``BudgetClaim.unit.unit_id`` on the wire so the sidecar ledger can
    resolve the budget claim. Most operators source this from the
    ``SPENDGUARD_UNIT_ID`` env var at adapter construction time.

    Omitting leaves the wire field empty and the ledger rejects the
    reserve with ``INVALID_REQUEST: claim[N].unit.unit_id empty`` —
    recipe-style integrations (no ledger reserve) MAY omit. NB: this
    is the ledger UUID, distinct from the free-form ``unit`` slug —
    they are NOT interchangeable.

    Additive optional field shipped under HARDEN_D05_UR (the Python
    SDK proto ``UnitRef.unit_id`` field already exists; this option
    threads it through the adapter's reserve path).
    """

    def __post_init__(self) -> None:
        """Validate required fields are non-empty (review-standards §2.4 N1/N2)."""
        # We touch validation in __post_init__ so the slice-7 tests can
        # exercise it via constructor without spinning a middleware up.
        # Frozen=True + slots=True still permit __post_init__ to read.
        if not self.tenant_id or not self.tenant_id.strip():
            raise SpendGuardConfigError(
                "SpendGuardAgentFrameworkOptions.tenant_id must be a "
                "non-empty string (per review-standards.md §7 N1)."
            )
        if not self.budget_id or not self.budget_id.strip():
            raise SpendGuardConfigError(
                "SpendGuardAgentFrameworkOptions.budget_id must be a "
                "non-empty string (per review-standards.md §7 N1)."
            )
        if not self.window_instance_id or not self.window_instance_id.strip():
            raise SpendGuardConfigError(
                "SpendGuardAgentFrameworkOptions.window_instance_id must be "
                "a non-empty string."
            )
        if not self.sidecar_socket_path or not self.sidecar_socket_path.strip():
            raise SpendGuardConfigError(
                "SpendGuardAgentFrameworkOptions.sidecar_socket_path must be "
                "a non-empty string (per review-standards.md §7 N2)."
            )
        if self.on_sidecar_unavailable not in ("deny", "allow"):
            raise SpendGuardConfigError(
                f"SpendGuardAgentFrameworkOptions.on_sidecar_unavailable must "
                f"be 'deny' or 'allow'; got {self.on_sidecar_unavailable!r}."
            )


__all__ = [
    "OnSidecarUnavailable",
    "SpendGuardAgentFrameworkOptions",
]
