"""The five pure builders — design.md §5.3-§5.7 payload schemas (LOCKED)
+ §8.2 signatures (LOCKED). Each ``build_*`` mirrors its TS twin
(``sdk/typescript-ag-ui/src/builders.ts``) key-for-key, including the
injected ``schema_version: "1"``, the injected ``decision: "DENY"`` on
denied, the omit-if-empty rule, and the ``approval_required``
validation (implementation.md §5.2).

Purity contract (design.md §4, §11.3): no clock reads, no RNG, no env,
no I/O, no global state. ``event_time`` / ``as_of`` / ``timestamp_ms``
are inputs. Every ID is an input — builders never mint or hash
(design.md §11.6).

Return value is a plain ``dict``; insertion order is NOT significant —
the canonical serializer owns ordering; builders own shape
(implementation.md §5.2).

Runtime imports: stdlib only (implementation.md §1.2).
"""

from __future__ import annotations

from collections.abc import Sequence
from typing import Any

from ._errors import AgUiEventValidationError
from ._types import (
    BudgetSnapshotInput,
    DecisionDeniedInput,
    ReservationCommittedInput,
    ReservationCreatedInput,
    ReservationReleasedInput,
)
from ._validate import (
    optional_entry,
    require_atomic,
    require_non_empty,
    require_rfc3339,
    require_safe_integer,
    require_string_array,
)

__all__ = [
    "BUDGET_SNAPSHOT",
    "RESERVATION_CREATED",
    "RESERVATION_COMMITTED",
    "RESERVATION_RELEASED",
    "DECISION_DENIED",
    "SPENDGUARD_AG_UI_EVENT_NAMES",
    "build_budget_snapshot",
    "build_reservation_created",
    "build_reservation_committed",
    "build_reservation_released",
    "build_decision_denied",
]

# ── The five-event SpendGuard AG-UI vocabulary — design.md §5.2, LOCKED ──
# These five strings are the public vocabulary. Renames, additions, or
# removals after the spec merge require a design.md re-spec
# (review-standards §2 P0). No sixth name exists anywhere.
BUDGET_SNAPSHOT: str = "spendguard.budget.snapshot"
RESERVATION_CREATED: str = "spendguard.reservation.created"
RESERVATION_COMMITTED: str = "spendguard.reservation.committed"
RESERVATION_RELEASED: str = "spendguard.reservation.released"
DECISION_DENIED: str = "spendguard.decision.denied"

# All five, §5.2 table order (design.md §8.2).
SPENDGUARD_AG_UI_EVENT_NAMES: tuple[str, ...] = (
    BUDGET_SNAPSHOT,
    RESERVATION_CREATED,
    RESERVATION_COMMITTED,
    RESERVATION_RELEASED,
    DECISION_DENIED,
)

# SpendGuard wire mapping (design.md §5.4): CONTINUE → ALLOW;
# DEGRADE → ALLOW_WITH_CAPS (ASP Draft-01 §2 pattern). The mapping is the
# CALLER's: builders accept only the ASP enum verbatim.
_CREATED_DECISIONS: tuple[str, ...] = ("ALLOW", "ALLOW_WITH_CAPS")

# SpendGuard `CommitEstimatedRequest.outcome` enum, verbatim, all four
# values (design.md §5.5).
_COMMITTED_OUTCOMES: tuple[str, ...] = (
    "SUCCESS",
    "PROVIDER_ERROR",
    "CLIENT_TIMEOUT",
    "RUN_ABORTED",
)

# SpendGuard sidecar decision-outcome taxonomy (design.md §5.7).
_DENIED_KINDS: tuple[str, ...] = (
    "DENY",
    "STOP",
    "STOP_RUN_PROJECTION",
    "SKIP",
    "APPROVAL_REQUIRED",
)


def _envelope(
    name: str, value: dict[str, Any], timestamp_ms: int | None
) -> dict[str, Any]:
    """Assemble the envelope. Purity contract: no clock reads, no
    randomness. ``timestamp`` is present iff the caller supplied
    ``timestamp_ms`` — ``0`` is a valid epoch ms and IS emitted when
    explicitly provided (mirrors builders.ts ``envelope``)."""
    if timestamp_ms is not None:
        require_safe_integer("timestamp", timestamp_ms)
        return {"type": "CUSTOM", "name": name, "value": value,
                "timestamp": timestamp_ms}
    return {"type": "CUSTOM", "name": name, "value": value}


def _required_array_entry(field: str, a: Sequence[str]) -> list[str]:
    """Required string[] payload entry: validated (>= 1 entry of
    non-empty strings) and copied so later caller mutation cannot reach
    the event. Array order is preserved as given (design.md §7.6)."""
    return require_string_array(field, a, min_len=1)


def _optional_array_entry(
    field: str, a: Sequence[str] | None
) -> dict[str, list[str]]:
    """Optional string[] payload entry: emitted only when provided AND
    non-empty (design.md §5.4/§5.7 omit-if-absent/empty); entries must
    be non-empty strings when emitted."""
    if a is None or (isinstance(a, (list, tuple)) and len(a) == 0):
        return {}
    return {field: require_string_array(field, a, min_len=1)}


# ── spendguard.budget.snapshot — design.md §5.3 ─────────────────────────
def build_budget_snapshot(
    input: BudgetSnapshotInput, *, timestamp_ms: int | None = None
) -> dict[str, Any]:
    value: dict[str, Any] = {
        "schema_version": "1",
        "budget_id": require_non_empty("budget_id", input.budget_id),
        "window_instance_id": require_non_empty(
            "window_instance_id", input.window_instance_id
        ),
        "unit": require_non_empty("unit", input.unit),
        "remaining_atomic": require_atomic(
            "remaining_atomic", input.remaining_atomic
        ),
        "reserved_atomic": require_atomic("reserved_atomic", input.reserved_atomic),
        "spent_atomic": require_atomic("spent_atomic", input.spent_atomic),
        "as_of": require_rfc3339("as_of", input.as_of),
        **optional_entry("unit_id", input.unit_id),  # omit-if-empty (design §6)
    }
    return _envelope(BUDGET_SNAPSHOT, value, timestamp_ms)


# ── spendguard.reservation.created — design.md §5.4 ─────────────────────
def build_reservation_created(
    input: ReservationCreatedInput, *, timestamp_ms: int | None = None
) -> dict[str, Any]:
    if input.decision not in _CREATED_DECISIONS:
        raise AgUiEventValidationError(
            "decision",
            'field "decision" must be "ALLOW" or "ALLOW_WITH_CAPS" '
            "(ASP decision enum, design.md §5.4)",
        )
    value: dict[str, Any] = {
        "schema_version": "1",
        "decision_id": require_non_empty("decision_id", input.decision_id),
        "reservation_id": require_non_empty("reservation_id", input.reservation_id),
        "budget_id": require_non_empty("budget_id", input.budget_id),
        "window_instance_id": require_non_empty(
            "window_instance_id", input.window_instance_id
        ),
        "unit": require_non_empty("unit", input.unit),
        "amount_atomic_reserved": require_atomic(
            "amount_atomic_reserved", input.amount_atomic_reserved
        ),
        "decision": input.decision,
        "ttl_expires_at": require_rfc3339("ttl_expires_at", input.ttl_expires_at),
        "event_time": require_rfc3339("event_time", input.event_time),
        **optional_entry("unit_id", input.unit_id),  # §6 unitId invariant
        **_optional_array_entry("reason_codes", input.reason_codes),
        **_optional_array_entry("matched_rule_ids", input.matched_rule_ids),
        **optional_entry("run_id", input.run_id),
        **optional_entry("llm_call_id", input.llm_call_id),
    }
    return _envelope(RESERVATION_CREATED, value, timestamp_ms)


# ── spendguard.reservation.committed — design.md §5.5 ───────────────────
def build_reservation_committed(
    input: ReservationCommittedInput, *, timestamp_ms: int | None = None
) -> dict[str, Any]:
    if input.outcome not in _COMMITTED_OUTCOMES:
        raise AgUiEventValidationError(
            "outcome",
            'field "outcome" must be one of "SUCCESS" | "PROVIDER_ERROR" | '
            '"CLIENT_TIMEOUT" | "RUN_ABORTED" (design.md §5.5)',
        )
    value: dict[str, Any] = {
        "schema_version": "1",
        "decision_id": require_non_empty("decision_id", input.decision_id),
        "reservation_id": require_non_empty("reservation_id", input.reservation_id),
        "budget_id": require_non_empty("budget_id", input.budget_id),
        "window_instance_id": require_non_empty(
            "window_instance_id", input.window_instance_id
        ),
        "unit": require_non_empty("unit", input.unit),
        # SpendGuard extension, documented delta (design.md §5.5): the only
        # commit lane today is CommitEstimated — named distinctly from ASP's
        # amount_atomic_observed so no consumer mistakes it for provider-
        # reported usage.
        "amount_atomic_estimated": require_atomic(
            "amount_atomic_estimated", input.amount_atomic_estimated
        ),
        "outcome": input.outcome,
        "event_time": require_rfc3339("event_time", input.event_time),
        **optional_entry("unit_id", input.unit_id),  # §6 unitId invariant
        # Reserved ASP field (design.md §5.5): omit-if-ABSENT — when supplied
        # it must be a valid atomic decimal string (the §6 empty≡absent
        # collapse is scoped to optional ID-style string fields; an amount is
        # validated).
        **(
            {
                "amount_atomic_observed": require_atomic(
                    "amount_atomic_observed", input.amount_atomic_observed
                )
            }
            if input.amount_atomic_observed is not None
            else {}
        ),
        **optional_entry("run_id", input.run_id),
        **optional_entry("llm_call_id", input.llm_call_id),
    }
    return _envelope(RESERVATION_COMMITTED, value, timestamp_ms)


# ── spendguard.reservation.released — design.md §5.6 ────────────────────
def build_reservation_released(
    input: ReservationReleasedInput, *, timestamp_ms: int | None = None
) -> dict[str, Any]:
    value: dict[str, Any] = {
        "schema_version": "1",
        "reservation_id": require_non_empty("reservation_id", input.reservation_id),
        # REQUIRED with >= 1 entry — "Reason for release goes in reason_codes"
        # (ASP audit.release, design.md §5.6).
        "reason_codes": _required_array_entry("reason_codes", input.reason_codes),
        "event_time": require_rfc3339("event_time", input.event_time),
        # Optional here because the adapter-wire ReleaseRequest does not
        # carry it (design.md §5.6).
        **optional_entry("decision_id", input.decision_id),
        **optional_entry("ledger_transaction_id", input.ledger_transaction_id),
        **optional_entry("run_id", input.run_id),
        **optional_entry("llm_call_id", input.llm_call_id),
    }
    return _envelope(RESERVATION_RELEASED, value, timestamp_ms)


# ── spendguard.decision.denied — design.md §5.7 ─────────────────────────
def build_decision_denied(
    input: DecisionDeniedInput, *, timestamp_ms: int | None = None
) -> dict[str, Any]:
    if input.denied_kind not in _DENIED_KINDS:
        raise AgUiEventValidationError(
            "denied_kind",
            'field "denied_kind" must be one of "DENY" | "STOP" | '
            '"STOP_RUN_PROJECTION" | "SKIP" | "APPROVAL_REQUIRED" '
            "(design.md §5.7)",
        )
    reason_codes = require_string_array("reason_codes", input.reason_codes, min_len=1)
    # ASP Draft-01 §2: approval-required is DENY + the "approval_required"
    # reason code. The builder validates and raises — it does NOT silently
    # append (design.md §5.7).
    if input.denied_kind == "APPROVAL_REQUIRED" and "approval_required" not in reason_codes:
        raise AgUiEventValidationError(
            "reason_codes",
            'denied_kind APPROVAL_REQUIRED requires reason_codes to include '
            '"approval_required" (ASP Draft-01 §2)',
        )
    value: dict[str, Any] = {
        "schema_version": "1",
        "decision_id": require_non_empty("decision_id", input.decision_id),
        # Injected literal — every deny-class SpendGuard outcome is ASP DENY
        # (design.md §5.7).
        "decision": "DENY",
        "denied_kind": input.denied_kind,
        "reason_codes": reason_codes,
        "event_time": require_rfc3339("event_time", input.event_time),
        **_optional_array_entry("matched_rule_ids", input.matched_rule_ids),
        # a deny can fire before budget binding (design.md §5.7)
        **optional_entry("budget_id", input.budget_id),
        **optional_entry("window_instance_id", input.window_instance_id),
        **optional_entry("unit", input.unit),
        **optional_entry("unit_id", input.unit_id),  # §6 unitId invariant
        **optional_entry("run_id", input.run_id),
        **optional_entry("llm_call_id", input.llm_call_id),
    }
    return _envelope(DECISION_DENIED, value, timestamp_ms)
