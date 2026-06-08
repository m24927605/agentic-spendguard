"""Options dataclasses for the Letta SpendGuard adapter.

Parity with the AutoGen / Strands / ADK / Agno adapters: a thin POCO
config bundle is *optional* — the public wrapper class
``SpendGuardLettaClient`` accepts the same fields as keyword args. Use
``SpendGuardLettaOptions`` when wiring the adapter via a config file or
``hydra`` / ``pydantic-settings``.

The shared run-context binding ``RunContext(run_id="...")`` is
*re-exported* from ``spendguard.integrations.openai_agents`` in
``__init__.py`` per design.md §5: "Reuse RunContext / run_context()
from openai_agents instead of duplicating. Polyglot agent stacks
share one trace." This file therefore declares only the per-adapter
options dataclass — the ``RunContext`` symbol is the OpenAI Agents one.
"""

from __future__ import annotations

from dataclasses import dataclass

from ._errors import SpendGuardConfigError


@dataclass(frozen=True, slots=True)
class SpendGuardLettaOptions:
    """Per-adapter configuration POCO.

    Attributes:
        tenant_id: SpendGuard tenant scope. REQUIRED — validated non-empty.
        budget_id: Budget the reservation debits. REQUIRED — validated
            non-empty.
        window_instance_id: Time-window scope on the budget.
            REQUIRED — validated non-empty.
        sidecar_socket_path: Path to the sidecar UDS. Default matches the
            project-wide convention.
        route: ``request_decision.route``. Defaults to ``"llm.call"`` so
            outbox rows line up with the LangChain / OpenAI Agents /
            DSPy / Agno / AutoGen integrations on the dashboard.

    Raises:
        SpendGuardConfigError: any required string field empty / whitespace.
    """

    tenant_id: str
    budget_id: str
    window_instance_id: str
    sidecar_socket_path: str = "/var/run/spendguard/sidecar.sock"
    route: str = "llm.call"
    unit_id: str | None = None
    """Canonical-truth UUID of the ledger unit row.

    When provided, the Letta client wrapper threads it through to
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
                "SpendGuardLettaOptions.tenant_id must be a non-empty string."
            )
        if not self.budget_id or not self.budget_id.strip():
            raise SpendGuardConfigError(
                "SpendGuardLettaOptions.budget_id must be a non-empty string."
            )
        if not self.window_instance_id or not self.window_instance_id.strip():
            raise SpendGuardConfigError(
                "SpendGuardLettaOptions.window_instance_id must be a "
                "non-empty string."
            )
        if not self.sidecar_socket_path or not self.sidecar_socket_path.strip():
            raise SpendGuardConfigError(
                "SpendGuardLettaOptions.sidecar_socket_path must be a "
                "non-empty string."
            )


__all__ = [
    "SpendGuardLettaOptions",
]
