"""Shared deterministic input vectors for the TA suite — mirror of the
TS ``tests/_support/vectors.ts`` role. Pure data, no clocks, no RNG."""

from __future__ import annotations

from typing import Any, Callable

from spendguard.integrations.ag_ui import (
    BudgetSnapshotInput,
    DecisionDeniedInput,
    ReservationCommittedInput,
    ReservationCreatedInput,
    ReservationReleasedInput,
    build_budget_snapshot,
    build_decision_denied,
    build_reservation_committed,
    build_reservation_created,
    build_reservation_released,
)

SNAPSHOT_KW: dict[str, Any] = {
    "budget_id": "budget-dev-monthly",
    "window_instance_id": "0197a001-0000-7000-8000-00000000win1",
    "unit": "usd_micros",
    "unit_id": "0197a001-2222-7000-8000-0000000unit1",
    "remaining_atomic": "24000000",
    "reserved_atomic": "1000000",
    "spent_atomic": "950000",
    "as_of": "2026-06-10T08:00:05Z",
}

CREATED_KW: dict[str, Any] = {
    "decision_id": "0197a001-3333-7000-8000-000000000dec",
    "reservation_id": "0197a001-4444-7000-8000-000000000res",
    "budget_id": "budget-dev-monthly",
    "window_instance_id": "0197a001-0000-7000-8000-00000000win1",
    "unit": "usd_micros",
    "unit_id": "0197a001-2222-7000-8000-0000000unit1",
    "amount_atomic_reserved": "1000000",
    "decision": "ALLOW",
    "ttl_expires_at": "2026-06-10T08:05:00Z",
    "reason_codes": ["within_budget"],
    "matched_rule_ids": ["rule-default-allow"],
    "run_id": "0197a001-5555-7000-8000-000000000run",
    "llm_call_id": "0197a001-6666-7000-8000-00000000call",
    "event_time": "2026-06-10T08:00:00Z",
}

COMMITTED_KW: dict[str, Any] = {
    "decision_id": "0197a001-3333-7000-8000-000000000dec",
    "reservation_id": "0197a001-4444-7000-8000-000000000res",
    "budget_id": "budget-dev-monthly",
    "window_instance_id": "0197a001-0000-7000-8000-00000000win1",
    "unit": "usd_micros",
    "unit_id": "0197a001-2222-7000-8000-0000000unit1",
    "amount_atomic_estimated": "950000",
    "outcome": "SUCCESS",
    "run_id": "0197a001-5555-7000-8000-000000000run",
    "llm_call_id": "0197a001-6666-7000-8000-00000000call",
    "event_time": "2026-06-10T08:00:03Z",
}

RELEASED_KW: dict[str, Any] = {
    "reservation_id": "0197a001-4444-7000-8000-000000000res",
    "decision_id": "0197a001-3333-7000-8000-000000000dec",
    "reason_codes": ["client_timeout"],
    "ledger_transaction_id": "0197a001-7777-7000-8000-00000000ltx1",
    "run_id": "0197a001-5555-7000-8000-000000000run",
    "llm_call_id": "0197a001-6666-7000-8000-00000000call",
    "event_time": "2026-06-10T08:00:04Z",
}

DENIED_KW: dict[str, Any] = {
    "decision_id": "0197a001-8888-7000-8000-000000000dec",
    "denied_kind": "DENY",
    "reason_codes": ["BUDGET_EXCEEDED"],
    "matched_rule_ids": ["rule-hard-cap"],
    "budget_id": "budget-dev-monthly",
    "window_instance_id": "0197a001-0000-7000-8000-00000000win1",
    "unit": "usd_micros",
    "unit_id": "0197a001-2222-7000-8000-0000000unit1",
    "run_id": "0197a001-5555-7000-8000-000000000run",
    "llm_call_id": "0197a001-6666-7000-8000-00000000call",
    "event_time": "2026-06-10T08:00:06Z",
}

# (builder, input class, kwargs, locked §5.2 name) — table order.
ALL_BUILDERS: list[tuple[Callable[..., dict[str, Any]], type, dict[str, Any], str]] = [
    (build_budget_snapshot, BudgetSnapshotInput, SNAPSHOT_KW,
     "spendguard.budget.snapshot"),
    (build_reservation_created, ReservationCreatedInput, CREATED_KW,
     "spendguard.reservation.created"),
    (build_reservation_committed, ReservationCommittedInput, COMMITTED_KW,
     "spendguard.reservation.committed"),
    (build_reservation_released, ReservationReleasedInput, RELEASED_KW,
     "spendguard.reservation.released"),
    (build_decision_denied, DecisionDeniedInput, DENIED_KW,
     "spendguard.decision.denied"),
]


def build_all(timestamp_ms: int | None = None) -> list[dict[str, Any]]:
    """One built event per builder, from the canonical vectors."""
    return [
        fn(cls(**kw), timestamp_ms=timestamp_ms)
        for fn, cls, kw, _name in ALL_BUILDERS
    ]
