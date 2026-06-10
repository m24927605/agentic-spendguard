# ruff: noqa: S101
"""TA-20..TA-24 — canonical-JSON rule conformance (design.md §7 LOCKED;
mirror twins of TP-20..TP-24)."""

from __future__ import annotations

import json

import pytest

from spendguard.integrations.ag_ui import (
    AgUiEventValidationError,
    canonical_event_json,
)

from ._vectors import build_all


def _event(value, timestamp=None):
    evt = {"type": "CUSTOM", "name": "spendguard.budget.snapshot", "value": value}
    if timestamp is not None:
        evt["timestamp"] = timestamp
    return evt


def test_ta20_recursive_key_sorting():
    """TA-20: keys sorted lexicographically by code point, recursively —
    including nested objects inside value."""
    evt = _event({"zeta": "1", "alpha": {"inner_z": "1", "inner_a": ["x"], "B": "2"}})
    out = canonical_event_json(evt)
    assert out == (
        '{"name":"spendguard.budget.snapshot","type":"CUSTOM",'
        '"value":{"alpha":{"B":"2","inner_a":["x"],"inner_z":"1"},"zeta":"1"}}'
    )
    # Envelope keys themselves sorted: name < timestamp < type < value.
    out_ts = canonical_event_json(_event({"a": "1"}, timestamp=5))
    assert out_ts.index('"name"') < out_ts.index('"timestamp"') \
        < out_ts.index('"type"') < out_ts.index('"value"')


def test_ta21_no_whitespace_no_bom():
    """TA-21: no `": "`, no `", "`, no newline, no trailing whitespace;
    UTF-8 without BOM."""
    for evt in build_all(timestamp_ms=1765843200000):
        out = canonical_event_json(evt)
        assert '": ' not in out
        assert '", "' not in out
        assert ", " not in out
        assert "\n" not in out
        assert out == out.strip()
        raw = out.encode("utf-8")
        assert not raw.startswith(b"\xef\xbb\xbf")


def test_ta22_unicode_passthrough_and_control_escapes():
    """TA-22: CJK/emoji/astral pass through as raw UTF-8 (not \\uXXXX);
    C0 controls escape exactly like JSON.stringify (shorthand for
    \\b \\t \\n \\f \\r, \\u00XX otherwise)."""
    s = "預算 🚀 \U0001f9e0 ok"
    out = canonical_event_json(_event({"k": s}))
    assert "預算" in out
    assert "🚀" in out
    assert "\\u" not in out  # no ASCII-escaping of non-ASCII chars
    ctl = canonical_event_json(_event({"k": "a\b\t\n\f\rb\x01"}))
    assert '"a\\b\\t\\n\\f\\rb\\u0001"' in ctl
    # The quote and backslash escapes are shared too.
    q = canonical_event_json(_event({"k": 'sa"y\\'}))
    assert '"sa\\"y\\\\"' in q


@pytest.mark.parametrize(
    "value",
    [
        {"k": 1.5},                      # float
        {"k": float("nan")},             # NaN
        {"k": float("inf")},             # Infinity
        {"k": -0.0},                     # -0 (float negative zero)
        {"k": 2**53},                    # int > 2^53 - 1
        {"k": None},                     # null value
        {"ké": "v"},                # non-ASCII key
        {"k": "bad\ud800surrogate"},     # unpaired surrogate
        {"k": object()},                 # unsupported type
        {" spaced key": "v"},            # space is outside [\x21-\x7e]+ key rule
    ],
)
def test_ta23_rejections(value):
    """TA-23: floats, non-finite, -0, unsafe ints, null, non-ASCII keys,
    unpaired surrogates all raise AgUiEventValidationError."""
    with pytest.raises(AgUiEventValidationError):
        canonical_event_json(_event(value))


def test_ta23_boundary_safe_integer_accepted():
    """Complement: 2^53 - 1 itself is legal (design.md §7.5)."""
    out = canonical_event_json(_event({"k": "v"}, timestamp=2**53 - 1))
    assert f'"timestamp":{2**53 - 1}' in out


def test_ta24_idempotent_on_own_output():
    """TA-24: canonical_event_json is idempotent — parse → re-serialize
    of its own output is byte-identical."""
    for evt in build_all(timestamp_ms=1765843200000):
        once = canonical_event_json(evt)
        again = canonical_event_json(json.loads(once))
        assert again == once
        assert again.encode("utf-8") == once.encode("utf-8")
