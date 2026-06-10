# ruff: noqa: S101
"""TA-14..TA-19 — omission rules, rejection rules, taxonomy validation
(docs/specs/coverage/D39_ag_ui/tests.md; mirror twins of TP-14..TP-19)."""

from __future__ import annotations

from pathlib import Path

import pytest

from spendguard.integrations.ag_ui import (
    AgUiEventValidationError,
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
from spendguard.integrations.ag_ui._validate import require_atomic, require_rfc3339

from ._vectors import COMMITTED_KW, CREATED_KW, DENIED_KW, RELEASED_KW, SNAPSHOT_KW

_CORPUS = (
    Path(__file__).resolve().parents[3].parent
    / "fixtures" / "cross-language" / "ag_ui_v1.json"
)

_UNIT_BUILDERS = [
    (build_budget_snapshot, BudgetSnapshotInput, SNAPSHOT_KW),
    (build_reservation_created, ReservationCreatedInput, CREATED_KW),
    (build_reservation_committed, ReservationCommittedInput, COMMITTED_KW),
    (build_decision_denied, DecisionDeniedInput, DENIED_KW),
]


@pytest.mark.parametrize("fn,cls,kw", _UNIT_BUILDERS)
@pytest.mark.parametrize("empty", [None, ""])
def test_ta14_unit_id_omission(fn, cls, kw, empty):
    """TA-14 (P0, HARDEN_D05_UR): None/"" → no unit_id key; non-empty →
    verbatim; on snapshot, created, committed, AND denied."""
    evt = fn(cls(**{**kw, "unit_id": empty}))
    assert "unit_id" not in evt["value"]
    evt = fn(cls(**{**kw, "unit_id": "unit-abc"}))
    assert evt["value"]["unit_id"] == "unit-abc"


def test_ta14_corpus_never_contains_empty_unit_id():
    """TA-14 corpus-wide half: `"unit_id":""` never appears in the
    frozen corpus (either canonical or pretty separators)."""
    text = _CORPUS.read_text(encoding="utf-8")
    assert '\\"unit_id\\":\\"\\"' not in text
    assert '"unit_id": ""' not in text


# (builder, input class, base kwargs, input field == payload key) — every
# REQUIRED string field of §5.3-§5.7.
_REQUIRED_STRING_FIELDS = [
    (build_budget_snapshot, BudgetSnapshotInput, SNAPSHOT_KW, "budget_id"),
    (build_budget_snapshot, BudgetSnapshotInput, SNAPSHOT_KW, "window_instance_id"),
    (build_budget_snapshot, BudgetSnapshotInput, SNAPSHOT_KW, "unit"),
    (build_reservation_created, ReservationCreatedInput, CREATED_KW, "decision_id"),
    (build_reservation_created, ReservationCreatedInput, CREATED_KW, "reservation_id"),
    (build_reservation_created, ReservationCreatedInput, CREATED_KW, "budget_id"),
    (build_reservation_created, ReservationCreatedInput, CREATED_KW, "window_instance_id"),
    (build_reservation_created, ReservationCreatedInput, CREATED_KW, "unit"),
    (build_reservation_committed, ReservationCommittedInput, COMMITTED_KW, "decision_id"),
    (build_reservation_committed, ReservationCommittedInput, COMMITTED_KW, "reservation_id"),
    (build_reservation_committed, ReservationCommittedInput, COMMITTED_KW, "budget_id"),
    (build_reservation_committed, ReservationCommittedInput, COMMITTED_KW, "window_instance_id"),
    (build_reservation_committed, ReservationCommittedInput, COMMITTED_KW, "unit"),
    (build_reservation_released, ReservationReleasedInput, RELEASED_KW, "reservation_id"),
    (build_decision_denied, DecisionDeniedInput, DENIED_KW, "decision_id"),
]


@pytest.mark.parametrize("fn,cls,kw,field", _REQUIRED_STRING_FIELDS)
def test_ta15_empty_required_string_raises_with_payload_key(fn, cls, kw, field):
    """TA-15: empty required string → AgUiEventValidationError whose
    ``field`` names the snake_case payload key."""
    with pytest.raises(AgUiEventValidationError) as exc:
        fn(cls(**{**kw, field: ""}))
    assert exc.value.field == field


def test_ta16_atomic_rule():
    """TA-16: atomic rule rejects bad shapes; accepts canonical decimal
    strings including a 40-digit value."""
    for bad in ["", "-1", "1.5", "01", "1e3", " 1", "+1"]:
        with pytest.raises(AgUiEventValidationError):
            require_atomic("remaining_atomic", bad)
    for good in ["0", "1", "100000", "9" * 40]:
        assert require_atomic("remaining_atomic", good) == good
    # Builder-level: same rule wired to the payload fields.
    with pytest.raises(AgUiEventValidationError) as exc:
        build_budget_snapshot(
            BudgetSnapshotInput(**{**SNAPSHOT_KW, "remaining_atomic": "01"})
        )
    assert exc.value.field == "remaining_atomic"


def test_ta17_rfc3339_gate():
    """TA-17: RFC 3339 gate rejects date-only / prose / empty / epoch
    ints; accepts the two valid forms (Z and numeric offset)."""
    for bad in ["2026-06-10", "yesterday", "", 1765843200, 1765843200000]:
        with pytest.raises(AgUiEventValidationError):
            require_rfc3339("event_time", bad)
    assert require_rfc3339("event_time", "2026-06-10T08:00:00Z")
    assert require_rfc3339("event_time", "2026-06-10T08:00:00.123+02:00")
    # Builder-level wiring.
    with pytest.raises(AgUiEventValidationError) as exc:
        build_budget_snapshot(BudgetSnapshotInput(**{**SNAPSHOT_KW, "as_of": "2026-06-10"}))
    assert exc.value.field == "as_of"


def test_ta18_denied_taxonomy():
    """TA-18: denied reason_codes=[] raises; APPROVAL_REQUIRED without
    "approval_required" raises citing ASP Draft-01 §2; with it → builds,
    no silent append, order preserved."""
    with pytest.raises(AgUiEventValidationError) as exc:
        build_decision_denied(DecisionDeniedInput(**{**DENIED_KW, "reason_codes": []}))
    assert exc.value.field == "reason_codes"

    with pytest.raises(AgUiEventValidationError) as exc:
        build_decision_denied(
            DecisionDeniedInput(
                **{**DENIED_KW, "denied_kind": "APPROVAL_REQUIRED",
                   "reason_codes": ["needs_human"]}
            )
        )
    assert exc.value.field == "reason_codes"
    assert "ASP Draft-01 §2" in str(exc.value)

    codes = ["needs_human", "approval_required", "spend_cap"]
    evt = build_decision_denied(
        DecisionDeniedInput(
            **{**DENIED_KW, "denied_kind": "APPROVAL_REQUIRED",
               "reason_codes": list(codes)}
        )
    )
    assert evt["value"]["reason_codes"] == codes  # no append, order preserved


def test_ta19_released_requires_reason_codes_created_does_not():
    """TA-19: released reason_codes=[] raises (>= 1 required); created
    reason_codes may be omitted entirely."""
    with pytest.raises(AgUiEventValidationError) as exc:
        build_reservation_released(
            ReservationReleasedInput(**{**RELEASED_KW, "reason_codes": []})
        )
    assert exc.value.field == "reason_codes"

    kw = {k: v for k, v in CREATED_KW.items() if k != "reason_codes"}
    evt = build_reservation_created(ReservationCreatedInput(**kw))
    assert "reason_codes" not in evt["value"]
