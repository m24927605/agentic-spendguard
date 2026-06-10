"""Builder input dataclasses — design.md §8.2 (LOCKED), mirroring the
§8.1 TS interfaces field-for-field (snake_case).

All five are frozen + slots (design.md §8.2 verbatim). ``kw_only=True``
is the Python mechanism that lets optional fields keep their §8.1
declaration ORDER while carrying ``None`` defaults (a positional
dataclass cannot place a defaulted field before a required one);
construction is keyword-only, exactly like the implementation.md §5.3
quickstart. Optional-field defaults are ``None``; ``None`` and ``""``
both mean "omit" (design.md §6).

Builders never read clocks: ``event_time`` / ``as_of`` / ``timestamp_ms``
are inputs (design.md §11.3).
"""

from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass

__all__ = [
    "BudgetSnapshotInput",
    "ReservationCreatedInput",
    "ReservationCommittedInput",
    "ReservationReleasedInput",
    "DecisionDeniedInput",
]


@dataclass(frozen=True, slots=True, kw_only=True)
class BudgetSnapshotInput:
    """Input for ``build_budget_snapshot`` — design.md §5.3."""

    budget_id: str
    window_instance_id: str
    unit: str
    unit_id: str | None = None
    remaining_atomic: str
    reserved_atomic: str
    spent_atomic: str
    as_of: str  # RFC 3339


@dataclass(frozen=True, slots=True, kw_only=True)
class ReservationCreatedInput:
    """Input for ``build_reservation_created`` — design.md §5.4."""

    decision_id: str
    reservation_id: str
    budget_id: str
    window_instance_id: str
    unit: str
    unit_id: str | None = None
    amount_atomic_reserved: str
    decision: str  # "ALLOW" | "ALLOW_WITH_CAPS"
    ttl_expires_at: str  # RFC 3339
    reason_codes: Sequence[str] | None = None
    matched_rule_ids: Sequence[str] | None = None
    run_id: str | None = None
    llm_call_id: str | None = None
    event_time: str  # RFC 3339


@dataclass(frozen=True, slots=True, kw_only=True)
class ReservationCommittedInput:
    """Input for ``build_reservation_committed`` — design.md §5.5."""

    decision_id: str
    reservation_id: str
    budget_id: str
    window_instance_id: str
    unit: str
    unit_id: str | None = None
    amount_atomic_estimated: str
    amount_atomic_observed: str | None = None  # reserved — future observed lane
    outcome: str  # "SUCCESS" | "PROVIDER_ERROR" | "CLIENT_TIMEOUT" | "RUN_ABORTED"
    run_id: str | None = None
    llm_call_id: str | None = None
    event_time: str  # RFC 3339


@dataclass(frozen=True, slots=True, kw_only=True)
class ReservationReleasedInput:
    """Input for ``build_reservation_released`` — design.md §5.6."""

    reservation_id: str
    decision_id: str | None = None
    reason_codes: Sequence[str]  # >= 1 entry
    ledger_transaction_id: str | None = None
    run_id: str | None = None
    llm_call_id: str | None = None
    event_time: str  # RFC 3339


@dataclass(frozen=True, slots=True, kw_only=True)
class DecisionDeniedInput:
    """Input for ``build_decision_denied`` — design.md §5.7."""

    decision_id: str
    denied_kind: str  # "DENY" | "STOP" | "STOP_RUN_PROJECTION" | "SKIP" | "APPROVAL_REQUIRED"
    reason_codes: Sequence[str]  # >= 1; APPROVAL_REQUIRED => must include "approval_required"
    matched_rule_ids: Sequence[str] | None = None
    budget_id: str | None = None
    window_instance_id: str | None = None
    unit: str | None = None
    unit_id: str | None = None
    run_id: str | None = None
    llm_call_id: str | None = None
    event_time: str  # RFC 3339
