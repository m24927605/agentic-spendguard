"""Options dataclasses + run-context POCO for the BeeAI SpendGuard adapter.

Parity with the Agno / Strands / DSPy adapters: a thin POCO config
bundle is *optional* — the public ``subscribe_spendguard`` function
accepts the same fields as keyword args. Use
``SpendGuardBeeAIOptions`` when wiring the adapter via a config file
or ``hydra`` / ``pydantic-settings``.

The shared run-context binding ``RunContext(run_id="...")`` lives at
the module level next to ``run_context``/``current_run_context`` in
``_hook.py``; this file defines only the frozen dataclass that the
hook reads.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from ._errors import SpendGuardConfigError


@dataclass(frozen=True, slots=True)
class RunContext:
    """Per ``BaseAgent.run()`` (or ``Workflow.run()``) identifiers.

    Bind a ``RunContext`` for the duration of the run with the
    ``run_context(...)`` async-context-manager exported from
    ``spendguard.integrations.beeai``. Multi-framework agents reuse
    the same ``run_id`` across LangChain / Pydantic-AI / OpenAI
    Agents / Agno / DSPy / BeeAI because all six adapters read the
    same module-level ``spendguard_run_context`` contextvar.

    Attributes:
        run_id: Caller-minted run identifier. Required.
    """

    run_id: str

    def __post_init__(self) -> None:
        """Validate ``run_id`` is non-empty."""
        if not self.run_id or not str(self.run_id).strip():
            raise SpendGuardConfigError(
                "spendguard.integrations.beeai.RunContext.run_id must be a "
                "non-empty string."
            )


@dataclass(frozen=True, slots=True)
class SpendGuardBeeAIOptions:
    """Per-adapter configuration POCO.

    Attributes:
        tenant_id: SpendGuard tenant scope. REQUIRED — validated non-empty.
        budget_id: Budget the reservation debits. REQUIRED — validated non-empty.
        window_instance_id: Time-window scope on the budget.
            REQUIRED — validated non-empty.
        sidecar_socket_path: Path to the sidecar UDS. Default matches the
            project-wide convention.
        route: ``request_decision.route``. Defaults to ``"llm.call"`` so
            outbox rows line up with the LangChain / OpenAI-Agents /
            Agno / DSPy integrations on the dashboard.

    Raises:
        SpendGuardConfigError: any required string field empty / whitespace.
    """

    tenant_id: str
    budget_id: str
    window_instance_id: str
    sidecar_socket_path: str = "/var/run/spendguard/sidecar.sock"
    route: str = "llm.call"

    def __post_init__(self) -> None:
        """Validate required fields are non-empty."""
        if not self.tenant_id or not self.tenant_id.strip():
            raise SpendGuardConfigError(
                "SpendGuardBeeAIOptions.tenant_id must be a non-empty string."
            )
        if not self.budget_id or not self.budget_id.strip():
            raise SpendGuardConfigError(
                "SpendGuardBeeAIOptions.budget_id must be a non-empty string."
            )
        if not self.window_instance_id or not self.window_instance_id.strip():
            raise SpendGuardConfigError(
                "SpendGuardBeeAIOptions.window_instance_id must be a "
                "non-empty string."
            )
        if not self.sidecar_socket_path or not self.sidecar_socket_path.strip():
            raise SpendGuardConfigError(
                "SpendGuardBeeAIOptions.sidecar_socket_path must be a "
                "non-empty string."
            )


@dataclass(slots=True)
class _InflightReservation:
    """Stash entry, keyed by the stable BeeAI call-path key.

    Holds the reservation companion identifiers between the
    ``*.start`` reserve and the matching ``*.success`` /
    ``*.error`` commit/release. Mutable so the success handler can
    attach the captured ``outcome`` snapshot without rebuilding the
    dataclass.

    Per ``docs/specs/coverage/D23_beeai/review-standards.md`` §3 the
    inflight map is a ``collections.OrderedDict`` bound at
    ``_INFLIGHT_MAX = 10_000`` with FIFO eviction, matching D04 §5
    (LangChain) and D22 (Agno) precedents.
    """

    signature: str
    reservation_ids: list[str]
    decision_id: str
    llm_call_id: str
    step_id: str
    run_id: str
    unit: Any
    pricing: Any
    model_id: str = ""


# Module-shared inflight map type alias.
InflightMap = "OrderedDict[str, _InflightReservation]"


__all__ = [
    "InflightMap",
    "RunContext",
    "SpendGuardBeeAIOptions",
    "_InflightReservation",
]
