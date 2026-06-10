"""SpendGuard spend-event family for AG-UI — ``spendguard.integrations.ag_ui``.

**Display-only.** AG-UI events are a presentation surface. SpendGuard
enforcement happens in the SpendGuard adapters and sidecar before the
provider call; these events report decisions already made and can neither
grant nor deny spend.

The 1:1 snake_case Python mirror of ``@spendguard/ag-ui``
(``sdk/typescript-ag-ui/``): five ``spendguard.*`` AG-UI ``CUSTOM`` event
names, five frozen-dataclass inputs, five pure builders, a canonical JSON
serializer (design.md §7 — byte-identical to the TS output for identical
inputs), and an SSE encode helper. Zero runtime dependencies beyond the
stdlib; the optional ``ag-ui`` extra (``pip install 'spendguard-sdk[ag-ui]'``)
exists only for users who validate events through the ``ag-ui-protocol``
pydantic models. There is deliberately NO import-time extras guard here
(unlike ``spendguard.integrations.dspy``) — the module works with zero
extras installed (implementation.md §1.2).

Quickstart::

    from spendguard.integrations.ag_ui import (
        ReservationCreatedInput, build_reservation_created, encode_sse,
    )

    evt = build_reservation_created(ReservationCreatedInput(
        decision_id=outcome.decision_id,
        reservation_id=outcome.reservation_ids[0],
        budget_id=budget_id, window_instance_id=window_instance_id,
        unit="usd_micros", unit_id=unit_id,
        amount_atomic_reserved="1000000",
        decision="ALLOW", ttl_expires_at="2026-06-10T08:00:00Z",
        event_time="2026-06-10T07:59:58Z",
    ))
    sse_frame = encode_sse(evt)   # hand to your AG-UI transport

Spec set: ``docs/specs/coverage/D39_ag_ui/`` (design.md LOCKED).
"""

from __future__ import annotations

from ._builders import (
    BUDGET_SNAPSHOT,
    DECISION_DENIED,
    RESERVATION_COMMITTED,
    RESERVATION_CREATED,
    RESERVATION_RELEASED,
    SPENDGUARD_AG_UI_EVENT_NAMES,
    build_budget_snapshot,
    build_decision_denied,
    build_reservation_committed,
    build_reservation_created,
    build_reservation_released,
)
from ._canonical import AgUiEmit, canonical_event_json, encode_sse
from ._errors import AgUiEventValidationError
from ._types import (
    BudgetSnapshotInput,
    DecisionDeniedInput,
    ReservationCommittedInput,
    ReservationCreatedInput,
    ReservationReleasedInput,
)

# implementation.md §5.3: exactly the 5 input dataclasses, the 5 builders,
# the 6 name constants (5 names + the tuple), canonical_event_json,
# encode_sse, AgUiEmit, AgUiEventValidationError. Nothing else.
__all__ = [
    # Input dataclasses
    "BudgetSnapshotInput",
    "ReservationCreatedInput",
    "ReservationCommittedInput",
    "ReservationReleasedInput",
    "DecisionDeniedInput",
    # Builders
    "build_budget_snapshot",
    "build_reservation_created",
    "build_reservation_committed",
    "build_reservation_released",
    "build_decision_denied",
    # Name constants
    "BUDGET_SNAPSHOT",
    "RESERVATION_CREATED",
    "RESERVATION_COMMITTED",
    "RESERVATION_RELEASED",
    "DECISION_DENIED",
    "SPENDGUARD_AG_UI_EVENT_NAMES",
    # Serialization + transport helper
    "canonical_event_json",
    "encode_sse",
    "AgUiEmit",
    # Errors
    "AgUiEventValidationError",
]
