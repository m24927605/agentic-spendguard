#!/usr/bin/env python3
"""COV_D39 SLICE 3 — the hard AG-UI event-stream gate (design.md §9 step 2).

Parses an SSE capture (``data: `` prefix, blank-line delimited) taken from
the ag-ui-runner's ``GET /events`` replay and asserts — exact, not ``>=``,
since the capture is one fresh scripted run:

  1. exactly 4 ``data:`` frames; order: ``spendguard.budget.snapshot``,
     ``spendguard.reservation.created``, ``spendguard.reservation.committed``,
     ``spendguard.decision.denied``;
  2. every required field of design.md §5.3-§5.7 present and non-empty;
  3. ``unit_id`` present and non-empty on snapshot/created/committed (the
     demo passes SPENDGUARD_UNIT_ID — HARDEN_D05_UR);
  4. ``created.reservation_id == committed.reservation_id`` and
     ``created.decision_id == committed.decision_id``;
  5. ``denied.decision == "DENY"`` and ``denied.reason_codes`` non-empty;
  6. every frame re-serializes to the identical bytes under the §7 canonical
     rule (wire == canonical form);
  7. prints ``RESERVATION_ID=<uuid>`` (consumed by the Makefile's
     display↔ledger psql join) + ``DENIED_DECISION_ID=<uuid>`` on success.

Python 3 stdlib only. Exits non-zero with a ``COV_D39_GATE:`` message on the
first failure. The canonical-bytes round-trip re-implements the §7 rule
inline via ``json.dumps(json.loads(payload), ensure_ascii=False,
sort_keys=True, separators=(",", ":"))`` — deliberately NOT importing
``spendguard.integrations.ag_ui``, so this gate is independent of the
library under test (implementation.md §9).
"""

from __future__ import annotations

import json
import sys

EXPECTED_ORDER = [
    "spendguard.budget.snapshot",
    "spendguard.reservation.created",
    "spendguard.reservation.committed",
    "spendguard.decision.denied",
]

# Required payload keys per design.md §5.3-§5.7 (non-empty unless noted).
REQUIRED_FIELDS = {
    "spendguard.budget.snapshot": [
        "schema_version",
        "budget_id",
        "window_instance_id",
        "unit",
        "remaining_atomic",
        "reserved_atomic",
        "spent_atomic",
        "as_of",
    ],
    "spendguard.reservation.created": [
        "schema_version",
        "decision_id",
        "reservation_id",
        "budget_id",
        "window_instance_id",
        "unit",
        "amount_atomic_reserved",
        "decision",
        "ttl_expires_at",
        "event_time",
    ],
    "spendguard.reservation.committed": [
        "schema_version",
        "decision_id",
        "reservation_id",
        "budget_id",
        "window_instance_id",
        "unit",
        "amount_atomic_estimated",
        "outcome",
        "event_time",
    ],
    "spendguard.decision.denied": [
        "schema_version",
        "decision_id",
        "decision",
        "denied_kind",
        "event_time",
    ],
}

DENIED_KIND_VALUES = {"DENY", "STOP", "STOP_RUN_PROJECTION", "SKIP", "APPROVAL_REQUIRED"}
COMMIT_OUTCOME_VALUES = {"SUCCESS", "PROVIDER_ERROR", "CLIENT_TIMEOUT", "RUN_ABORTED"}
CREATED_DECISION_VALUES = {"ALLOW", "ALLOW_WITH_CAPS"}

# Frames that MUST carry a non-empty unit_id in this demo (the runner passes
# SPENDGUARD_UNIT_ID; an omitted/empty unit_id here means the demo lost the
# HARDEN_D05_UR threading).
UNIT_ID_REQUIRED = EXPECTED_ORDER[:3]


def fail(msg: str) -> None:
    print(f"COV_D39_GATE: {msg}", file=sys.stderr)
    sys.exit(1)


def parse_sse(raw: str) -> list[str]:
    """Return the raw `data: ` payload strings, in capture order."""
    payloads: list[str] = []
    for block in raw.split("\n\n"):
        block = block.strip("\n")
        if not block:
            continue
        # Every non-empty block must be a single data-only frame (design.md
        # §7: data-only SSE framing — no event:/id: fields).
        for line in block.split("\n"):
            if not line.startswith("data: "):
                fail(f"non-data SSE line in capture: {line[:120]!r}")
            payloads.append(line[len("data: ") :])
    return payloads


def canonical(obj: object) -> str:
    """design.md §7 rule, re-implemented inline (stdlib only)."""
    return json.dumps(obj, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def require_non_empty_str(name: str, payload_key: str, value: object) -> None:
    if not isinstance(value, str) or value == "":
        fail(f"{name}: required field {payload_key!r} missing or empty (got {value!r})")


def main() -> None:
    if len(sys.argv) != 2:
        fail("usage: verify_sse.py <capture-file>")
    try:
        raw = open(sys.argv[1], encoding="utf-8").read()
    except OSError as exc:
        fail(f"cannot read capture file: {exc}")

    payloads = parse_sse(raw)

    # 1) exactly 4 frames, exact order.
    if len(payloads) != 4:
        fail(f"expected exactly 4 data: frames, got {len(payloads)}")

    events = []
    for idx, payload in enumerate(payloads):
        try:
            event = json.loads(payload)
        except json.JSONDecodeError as exc:
            fail(f"frame {idx}: invalid JSON ({exc})")
        # 6) canonical-bytes round-trip: wire form == §7 canonical form.
        rebuilt = canonical(json.loads(payload))
        if rebuilt != payload:
            fail(f"frame {idx}: wire bytes are not canonical (§7 round-trip mismatch)")
        events.append(event)

    for idx, (event, expected_name) in enumerate(zip(events, EXPECTED_ORDER)):
        if event.get("type") != "CUSTOM":
            fail(f"frame {idx}: envelope type != CUSTOM (got {event.get('type')!r})")
        if event.get("name") != expected_name:
            fail(
                f"frame {idx}: expected event name {expected_name!r}, "
                f"got {event.get('name')!r} (order is part of the contract)"
            )
        value = event.get("value")
        if not isinstance(value, dict):
            fail(f"frame {idx} ({expected_name}): envelope value is not an object")

        # 2) required fields present + non-empty.
        for key in REQUIRED_FIELDS[expected_name]:
            require_non_empty_str(expected_name, key, value.get(key))

        # 3) unit_id present and non-empty where the demo threads it.
        if expected_name in UNIT_ID_REQUIRED:
            require_non_empty_str(expected_name, "unit_id", value.get("unit_id"))

    snapshot, created, committed, denied = (e["value"] for e in events)

    # Enum sanity per §5.4 / §5.5 / §5.7.
    if created["decision"] not in CREATED_DECISION_VALUES:
        fail(f"created.decision {created['decision']!r} not in {sorted(CREATED_DECISION_VALUES)}")
    if committed["outcome"] not in COMMIT_OUTCOME_VALUES:
        fail(f"committed.outcome {committed['outcome']!r} not in {sorted(COMMIT_OUTCOME_VALUES)}")
    if denied["denied_kind"] not in DENIED_KIND_VALUES:
        fail(f"denied.denied_kind {denied['denied_kind']!r} not in {sorted(DENIED_KIND_VALUES)}")

    # 4) ID joins across the ALLOW pair.
    if created["reservation_id"] != committed["reservation_id"]:
        fail(
            "created.reservation_id != committed.reservation_id "
            f"({created['reservation_id']!r} vs {committed['reservation_id']!r})"
        )
    if created["decision_id"] != committed["decision_id"]:
        fail(
            "created.decision_id != committed.decision_id "
            f"({created['decision_id']!r} vs {committed['decision_id']!r})"
        )

    # 5) deny taxonomy: ASP decision literal + non-empty reason codes.
    if denied["decision"] != "DENY":
        fail(f'denied.decision must be the ASP literal "DENY", got {denied["decision"]!r}')
    reason_codes = denied.get("reason_codes")
    if (
        not isinstance(reason_codes, list)
        or len(reason_codes) == 0
        or not all(isinstance(c, str) and c for c in reason_codes)
    ):
        fail(f"denied.reason_codes must be a non-empty array of non-empty strings, got {reason_codes!r}")

    print("COV_D39 SSE OK: 4 frames, exact order, required fields, canonical bytes, ID joins")
    # 7) handles for the Makefile's display↔ledger join.
    print(f"RESERVATION_ID={created['reservation_id']}")
    print(f"DENIED_DECISION_ID={denied['decision_id']}")


if __name__ == "__main__":
    main()
