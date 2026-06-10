# ruff: noqa: S101
"""TA-25..TA-26 — SSE framing (design.md §7 LOCKED framing; mirror twins
of TP-25..TP-26)."""

from __future__ import annotations

from spendguard.integrations.ag_ui import canonical_event_json, encode_sse

from ._vectors import build_all


def test_ta25_frame_is_data_plus_canonical_plus_blank_line():
    """TA-25: encode_sse(e) == "data: " + canonical_event_json(e) +
    "\\n\\n" exactly, for every event type."""
    events = build_all(timestamp_ms=1765843200000) + build_all()
    assert len(events) == 10  # all five builders, with and without timestamp
    for evt in events:
        assert encode_sse(evt) == "data: " + canonical_event_json(evt) + "\n\n"


def test_ta26_no_interior_newline():
    """TA-26: the frame contains no newline other than the terminating
    blank line — interior newlines would split the SSE frame."""
    for evt in build_all(timestamp_ms=1765843200000):
        frame = encode_sse(evt)
        assert frame.endswith("\n\n")
        body = frame[: -len("\n\n")]
        assert "\n" not in body
        assert "\r" not in body
