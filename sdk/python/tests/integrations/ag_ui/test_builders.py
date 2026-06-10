# ruff: noqa: S101
"""TA-01..TA-13 — builder purity, envelope, and payload shape
(docs/specs/coverage/D39_ag_ui/tests.md; mirror twins of TP-01..TP-13)."""

from __future__ import annotations

import copy
import dataclasses
import time

import pytest

from spendguard.integrations.ag_ui import (
    AgUiEventValidationError,
    DecisionDeniedInput,
    ReservationCommittedInput,
    ReservationCreatedInput,
    ReservationReleasedInput,
    canonical_event_json,
    build_decision_denied,
    build_reservation_committed,
    build_reservation_created,
    build_reservation_released,
)

from ._vectors import (
    ALL_BUILDERS,
    COMMITTED_KW,
    CREATED_KW,
    DENIED_KW,
    RELEASED_KW,
)

# §5.3-§5.7 locked payload key sets (maximal vectors — all optionals set).
SNAPSHOT_KEYS = {
    "schema_version", "budget_id", "window_instance_id", "unit", "unit_id",
    "remaining_atomic", "reserved_atomic", "spent_atomic", "as_of",
}
CREATED_KEYS = {
    "schema_version", "decision_id", "reservation_id", "budget_id",
    "window_instance_id", "unit", "unit_id", "amount_atomic_reserved",
    "decision", "ttl_expires_at", "reason_codes", "matched_rule_ids",
    "run_id", "llm_call_id", "event_time",
}
COMMITTED_KEYS = {
    "schema_version", "decision_id", "reservation_id", "budget_id",
    "window_instance_id", "unit", "unit_id", "amount_atomic_estimated",
    "outcome", "run_id", "llm_call_id", "event_time",
}
RELEASED_KEYS = {
    "schema_version", "reservation_id", "decision_id", "reason_codes",
    "ledger_transaction_id", "run_id", "llm_call_id", "event_time",
}
DENIED_KEYS = {
    "schema_version", "decision_id", "decision", "denied_kind",
    "reason_codes", "matched_rule_ids", "budget_id", "window_instance_id",
    "unit", "unit_id", "run_id", "llm_call_id", "event_time",
}


@pytest.mark.parametrize("fn,cls,kw,name", ALL_BUILDERS)
def test_ta01_type_custom_and_locked_name(fn, cls, kw, name):
    """TA-01: each builder returns type CUSTOM + the exact §5.2 name."""
    evt = fn(cls(**kw))
    assert evt["type"] == "CUSTOM"
    assert evt["name"] == name


@pytest.mark.parametrize("fn,cls,kw,name", ALL_BUILDERS)
def test_ta02_purity_deterministic(fn, cls, kw, name):
    """TA-02: deep-equal inputs → deep-equal events; 100 repeated calls
    → identical canonical_event_json bytes."""
    a = fn(cls(**copy.deepcopy(kw)), timestamp_ms=1765843200000)
    b = fn(cls(**copy.deepcopy(kw)), timestamp_ms=1765843200000)
    assert a == b
    reference = canonical_event_json(a).encode("utf-8")
    for _ in range(100):
        again = fn(cls(**copy.deepcopy(kw)), timestamp_ms=1765843200000)
        assert canonical_event_json(again).encode("utf-8") == reference


@pytest.mark.parametrize("fn,cls,kw,name", ALL_BUILDERS)
def test_ta03_clock_free(fn, cls, kw, name, monkeypatch):
    """TA-03: time.time monkeypatched to raise; builders still work."""

    def _boom(*_args, **_kwargs):  # pragma: no cover - must never run
        raise AssertionError("builder read the clock — purity violation")

    monkeypatch.setattr(time, "time", _boom)
    monkeypatch.setattr(time, "time_ns", _boom)
    monkeypatch.setattr(time, "monotonic", _boom)
    evt = fn(cls(**kw))
    assert evt["type"] == "CUSTOM"


@pytest.mark.parametrize("fn,cls,kw,name", ALL_BUILDERS)
def test_ta04_timestamp_exact_or_absent(fn, cls, kw, name):
    """TA-04: timestamp_ms provided → envelope timestamp equals it
    exactly; omitted → key ABSENT (not null, not 0); explicit 0 is a
    valid epoch ms and IS emitted."""
    with_ts = fn(cls(**kw), timestamp_ms=1765843200000)
    assert with_ts["timestamp"] == 1765843200000
    without_ts = fn(cls(**kw))
    assert "timestamp" not in without_ts
    zero_ts = fn(cls(**kw), timestamp_ms=0)
    assert zero_ts["timestamp"] == 0


def test_ta05_snapshot_key_set_and_schema_version():
    """TA-05: snapshot payload = exactly the §5.3 key set; schema_version
    is the injected literal "1"."""
    from spendguard.integrations.ag_ui import BudgetSnapshotInput, build_budget_snapshot
    from ._vectors import SNAPSHOT_KW

    evt = build_budget_snapshot(BudgetSnapshotInput(**SNAPSHOT_KW))
    assert set(evt["value"].keys()) == SNAPSHOT_KEYS
    assert evt["value"]["schema_version"] == "1"
    assert set(evt.keys()) == {"type", "name", "value"}


def test_ta06_created_key_set_and_decision_passthrough():
    """TA-06: created matches §5.4 key set; decision passes through
    "ALLOW" / "ALLOW_WITH_CAPS" verbatim; anything else raises."""
    evt = build_reservation_created(ReservationCreatedInput(**CREATED_KW))
    assert set(evt["value"].keys()) == CREATED_KEYS
    assert evt["value"]["schema_version"] == "1"
    for decision in ("ALLOW", "ALLOW_WITH_CAPS"):
        evt = build_reservation_created(
            ReservationCreatedInput(**{**CREATED_KW, "decision": decision})
        )
        assert evt["value"]["decision"] == decision
    with pytest.raises(AgUiEventValidationError) as exc:
        build_reservation_created(
            ReservationCreatedInput(**{**CREATED_KW, "decision": "DENY"})
        )
    assert exc.value.field == "decision"


def test_ta07_committed_key_set_and_outcomes():
    """TA-07: committed matches §5.5; all four outcome values accepted
    verbatim; a 5th raises."""
    evt = build_reservation_committed(ReservationCommittedInput(**COMMITTED_KW))
    assert set(evt["value"].keys()) == COMMITTED_KEYS
    assert evt["value"]["schema_version"] == "1"
    for outcome in ("SUCCESS", "PROVIDER_ERROR", "CLIENT_TIMEOUT", "RUN_ABORTED"):
        evt = build_reservation_committed(
            ReservationCommittedInput(**{**COMMITTED_KW, "outcome": outcome})
        )
        assert evt["value"]["outcome"] == outcome
    with pytest.raises(AgUiEventValidationError) as exc:
        build_reservation_committed(
            ReservationCommittedInput(**{**COMMITTED_KW, "outcome": "PARTIAL"})
        )
    assert exc.value.field == "outcome"


def test_ta08_estimated_present_observed_reserved():
    """TA-08: emits amount_atomic_estimated; amount_atomic_observed
    ABSENT unless supplied, verbatim when supplied."""
    evt = build_reservation_committed(ReservationCommittedInput(**COMMITTED_KW))
    assert evt["value"]["amount_atomic_estimated"] == "950000"
    assert "amount_atomic_observed" not in evt["value"]
    evt = build_reservation_committed(
        ReservationCommittedInput(**{**COMMITTED_KW, "amount_atomic_observed": "942117"})
    )
    assert evt["value"]["amount_atomic_observed"] == "942117"


def test_ta09_released_key_set_and_draft01_reason_codes():
    """TA-09: released matches §5.6; ASP Draft-01 §4 example reason_codes
    round-trip verbatim."""
    evt = build_reservation_released(ReservationReleasedInput(**RELEASED_KW))
    assert set(evt["value"].keys()) == RELEASED_KEYS
    assert evt["value"]["schema_version"] == "1"
    draft01_examples = ["provider_error", "client_timeout", "run_cancelled"]
    evt = build_reservation_released(
        ReservationReleasedInput(**{**RELEASED_KW, "reason_codes": list(draft01_examples)})
    )
    assert evt["value"]["reason_codes"] == draft01_examples


@pytest.mark.parametrize(
    "denied_kind",
    ["DENY", "STOP", "STOP_RUN_PROJECTION", "SKIP", "APPROVAL_REQUIRED"],
)
def test_ta10_denied_injects_literal_deny(denied_kind):
    """TA-10: denied injects literal decision: "DENY" regardless of
    denied_kind; §5.7 key set."""
    kw = {**DENIED_KW, "denied_kind": denied_kind}
    if denied_kind == "APPROVAL_REQUIRED":
        kw["reason_codes"] = ["approval_required"]
    evt = build_decision_denied(DecisionDeniedInput(**kw))
    assert evt["value"]["decision"] == "DENY"
    assert evt["value"]["denied_kind"] == denied_kind
    assert set(evt["value"].keys()) == DENIED_KEYS


def test_ta11_denied_kind_taxonomy_closed():
    """TA-11: all five denied_kind values accepted verbatim (TA-10
    covers acceptance); a 6th raises."""
    with pytest.raises(AgUiEventValidationError) as exc:
        build_decision_denied(
            DecisionDeniedInput(**{**DENIED_KW, "denied_kind": "THROTTLE"})
        )
    assert exc.value.field == "denied_kind"


@pytest.mark.parametrize("fn,cls,kw,name", ALL_BUILDERS)
def test_ta12_inputs_unchanged_and_frozen(fn, cls, kw, name):
    """TA-12: builder inputs unchanged (frozen-dataclass input compared
    against a snapshot taken before the build); dataclass is frozen."""
    inp = cls(**copy.deepcopy(kw))
    snapshot = copy.deepcopy(dataclasses.asdict(inp))
    fn(inp, timestamp_ms=1765843200000)
    assert dataclasses.asdict(inp) == snapshot
    first_field = dataclasses.fields(inp)[0].name
    with pytest.raises(dataclasses.FrozenInstanceError):
        setattr(inp, first_field, "mutated")


def test_ta13_created_arrays_verbatim_or_absent():
    """TA-13: created reason_codes/matched_rule_ids: non-empty → verbatim
    caller order; empty/omitted → key ABSENT."""
    codes = ["zeta", "alpha", "mid"]
    rules = ["rule-9", "rule-1"]
    evt = build_reservation_created(
        ReservationCreatedInput(
            **{**CREATED_KW, "reason_codes": list(codes), "matched_rule_ids": list(rules)}
        )
    )
    assert evt["value"]["reason_codes"] == codes  # caller order, no sorting
    assert evt["value"]["matched_rule_ids"] == rules

    for absent in (None, []):
        evt = build_reservation_created(
            ReservationCreatedInput(
                **{**CREATED_KW, "reason_codes": absent, "matched_rule_ids": absent}
            )
        )
        assert "reason_codes" not in evt["value"]
        assert "matched_rule_ids" not in evt["value"]
