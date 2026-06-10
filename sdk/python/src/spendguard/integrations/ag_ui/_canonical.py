"""canonical_event_json + encode_sse — the design.md §7 cross-language
byte-equivalence rule, LOCKED. Applied to the WHOLE envelope
(``{type, name, value, timestamp?}``).

``json.dumps(obj, ensure_ascii=False, sort_keys=True, separators=(",", ":"))``
matches the TS recursive key-sorted rebuild + ``JSON.stringify``
byte-for-byte under the §7 constraints — proven by TA-27 against the
frozen corpus ``sdk/fixtures/cross-language/ag_ui_v1.json``.

Key facts that make the equivalence hold:
- Keys are enforced printable-ASCII (``[\\x21-\\x7e]``), so Python's
  code-point sort and JS's UTF-16 code-unit sort are identical.
- ``null``, floats, non-finite numbers, out-of-safe-range ints, and
  unpaired surrogates are rejected (raise) instead of serialized.
- Both serializers then agree on the escape set: ``"`` ``\\`` and the C0
  controls (shorthand ``\\b \\t \\n \\f \\r``, ``\\u00XX`` otherwise);
  all other characters pass through as raw UTF-8.

Runtime imports: stdlib only (implementation.md §1.2).
"""

from __future__ import annotations

import json
import re
from collections.abc import Callable, Mapping
from typing import Any

from ._errors import AgUiEventValidationError

__all__ = ["canonical_event_json", "encode_sse", "AgUiEmit"]

# Printable ASCII only — mirrors canonical.ts ASCII_KEY_RE (design.md §7.4).
_ASCII_KEY_RE = re.compile(r"^[\x21-\x7e]+$")

# 2^53 - 1 — JS Number.MAX_SAFE_INTEGER (design.md §7.5).
_MAX_SAFE_INTEGER = 2**53 - 1

# Transport-agnostic emit callback type (design.md §8.2).
AgUiEmit = Callable[[Mapping[str, Any]], None]


def canonical_event_json(event: Mapping[str, Any]) -> str:
    """Serialize ``event`` under the design.md §7 canonical rule."""
    _check(event)
    return json.dumps(
        event, ensure_ascii=False, sort_keys=True, separators=(",", ":"),
        allow_nan=False,
    )


def _check(v: object) -> None:
    """Recursive constraints per design.md §7: ASCII keys, no null, no
    float, safe-range ints, well-formed strings (no unpaired surrogates).

    ``bool`` is checked BEFORE ``int`` (Python ``bool`` is an ``int``
    subclass); ``True``/``False`` serialize as ``true``/``false`` in
    both languages (implementation.md §5.1).
    """
    if isinstance(v, str):
        _assert_well_formed(v)
        return
    if isinstance(v, bool):
        return
    if isinstance(v, int):
        _assert_canonical_int(v)
        return
    if isinstance(v, float):
        raise AgUiEventValidationError(
            "(value)",
            "floats, non-finite numbers, -0, and unsafe integers are "
            "forbidden in canonical payload",
        )
    if isinstance(v, (list, tuple)):
        # Array order is preserved as given — arrays are caller-ordered,
        # e.g. reason_codes (design.md §7.6).
        for entry in v:
            _check(entry)
        return
    if isinstance(v, Mapping):
        for k in v:
            _assert_ascii_key(k)
            _check(v[k])
        return
    # null is forbidden — omit the key instead (design.md §7.5).
    raise AgUiEventValidationError(
        "(value)", "null/None/unsupported type in canonical payload"
    )


def _assert_well_formed(s: str) -> None:
    """Strings must be valid Unicode; unpaired surrogates are rejected
    (design.md §7.5). Python strings can carry lone surrogates (e.g. via
    ``surrogateescape``); a strict UTF-8 encode detects them — the same
    well-formedness predicate as JS ``String.prototype.isWellFormed()``.
    """
    try:
        s.encode("utf-8", errors="strict")
    except UnicodeEncodeError:
        raise AgUiEventValidationError(
            "(value)", "unpaired surrogate in canonical string value"
        ) from None


def _assert_canonical_int(n: int) -> None:
    """Integers only: |n| <= 2^53 - 1 (design.md §7.5). Floats and
    non-finite numbers were already rejected by type; Python has no
    integer ``-0`` so the TS ``Object.is(n, -0)`` arm cannot occur here
    (float ``-0.0`` is rejected as a float)."""
    if abs(n) > _MAX_SAFE_INTEGER:
        raise AgUiEventValidationError(
            "(value)",
            "floats, non-finite numbers, -0, and unsafe integers are "
            "forbidden in canonical payload",
        )


def _assert_ascii_key(k: object) -> None:
    if not isinstance(k, str) or _ASCII_KEY_RE.fullmatch(k) is None:
        raise AgUiEventValidationError(
            "(key)", "object keys must be printable ASCII [\\x21-\\x7e]"
        )


def encode_sse(event: Mapping[str, Any]) -> str:
    """SSE encode helper — design.md §7, LOCKED framing:

        encode_sse(e) == "data: " + canonical_event_json(e) + "\\n\\n"

    Data-only frames are exactly what the AG-UI reference client
    consumes (slice-1 marker resolution: the pinned ``@ag-ui/client``
    SSE parser splits on blank lines, reads only ``data:`` lines, and
    ignores ``event:``/``id:`` fields entirely).
    """
    return f"data: {canonical_event_json(event)}\n\n"
