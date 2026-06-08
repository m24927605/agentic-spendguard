"""Options dataclass for the AWS Strands SpendGuard hook provider.

A thin, optional config bundle. The provider's primary configuration is
passed through positional / keyword args on the constructor (``client``,
``budget_id``, ``window_instance_id``, ``unit``, ``pricing``,
``claim_reconciler``); this dataclass exists for users who want to keep
the non-proto, POCO-shaped knobs in one place (mirrors the ADK and MAF
integrations' ``Options`` pattern).

Unlike the MAF options object, this is **not required** — the
``SpendGuardStrandsHookProvider`` constructor accepts the same fields
directly for ergonomic parity with the LangChain / OpenAI Agents priors.
Use this dataclass when wiring the adapter via a config file or
``hydra`` / ``pydantic-settings``.
"""

from __future__ import annotations

from dataclasses import dataclass

from ._errors import SpendGuardConfigError


@dataclass(frozen=True, slots=True)
class SpendGuardStrandsOptions:
    """Per-provider configuration POCO for the Strands integration.

    Attributes:
        tenant_id: SpendGuard tenant scope. REQUIRED — validated non-empty.
        budget_id: Budget the reservation debits. REQUIRED — validated non-empty.
        window_instance_id: Time-window scope on the budget.
            REQUIRED — validated non-empty.
        sidecar_socket_path: Path to the sidecar UDS. Default matches the
            project-wide convention.
        fail_closed: When ``True`` (the default) a sidecar DEGRADE or
            unavailability raises ``SpendGuardDegradeBlocked`` /
            ``SidecarUnavailable``. When ``False`` (dev only — also
            triggered by ``SPENDGUARD_STRANDS_FAIL_OPEN=1``) the
            invocation proceeds without a reservation; commit will not
            fire.

    Raises:
        SpendGuardConfigError: any required string field empty / whitespace.
    """

    tenant_id: str
    budget_id: str
    window_instance_id: str
    sidecar_socket_path: str = "/var/run/spendguard/sidecar.sock"
    fail_closed: bool = True
    unit_id: str | None = None
    """Canonical-truth UUID of the ledger unit row.

    When provided, the hook provider threads it through to
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
        """Validate required fields are non-empty."""
        if not self.tenant_id or not self.tenant_id.strip():
            raise SpendGuardConfigError(
                "SpendGuardStrandsOptions.tenant_id must be a non-empty string."
            )
        if not self.budget_id or not self.budget_id.strip():
            raise SpendGuardConfigError(
                "SpendGuardStrandsOptions.budget_id must be a non-empty string."
            )
        if not self.window_instance_id or not self.window_instance_id.strip():
            raise SpendGuardConfigError(
                "SpendGuardStrandsOptions.window_instance_id must be a "
                "non-empty string."
            )
        if not self.sidecar_socket_path or not self.sidecar_socket_path.strip():
            raise SpendGuardConfigError(
                "SpendGuardStrandsOptions.sidecar_socket_path must be a "
                "non-empty string."
            )


@dataclass(frozen=True, slots=True)
class StrandsRunContext:
    """Optional run-scope correlation context.

    Strands' typed event bus already carries ``invocation_id`` on
    ``BeforeInvocationEvent`` / ``AfterInvocationEvent``, so a
    ``RunContext`` is *not* required (unlike LangChain / MAF where the
    user must bind one before ``agent.run()``).

    Provide this when bridging Strands runs to a cross-framework
    ``run_id`` (e.g. a parent LangChain run wrapping the Strands agent).

    Attributes:
        run_id: Caller-minted run identifier. When set, the provider
            uses it as the ``RequestDecision.ids.run_id`` instead of
            deriving from the Strands ``invocation_id``.
        step_id: Optional step prefix override. Defaults to
            ``"strands:" + invocation_id[:16]`` when omitted.
    """

    run_id: str
    step_id: str | None = None


@dataclass(slots=True)
class _PendingInvocation:
    """Stash entry, keyed by Strands invocation_id.

    Holds the reservation companion identifiers between the
    ``before_invocation`` reserve and the matching ``after_invocation``
    commit/release. Mutable because operators may want to attach
    per-invocation metadata (e.g. tracing span ids) via a wrapper class
    in advanced setups.
    """

    decision_id: str
    reservation_ids: tuple[str, ...]
    llm_call_id: str
    run_id: str
    step_id: str
    # Frozen primitives snapshot of the estimator claim — survives a
    # claim_reconciler exception so we can still commit with the original
    # reserve amount rather than dropping the row.
    estimator_amount_atomic: str
    estimator_unit_id: str
    model_backend: str = ""
    model_id: str = ""


__all__ = [
    "SpendGuardStrandsOptions",
    "StrandsRunContext",
    "_PendingInvocation",
]
