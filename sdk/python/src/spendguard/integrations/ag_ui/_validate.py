"""Field validators — implementation.md §4.2, LOCKED rules.

The rules AND the regexes are part of the cross-language contract: this
module mirrors ``sdk/typescript-ag-ui/src/validate.ts``
character-for-character. A string accepted by one language and rejected
by the other is a fixture-level break (review-standards §4.5).

Two Python-vs-JS semantic deltas are neutralized deliberately so the
character-identical regex SOURCE also has identical BEHAVIOR:

1. ``re.ASCII`` — JS ``\\d`` is ASCII-only; Python ``\\d`` defaults to
   Unicode digits. The flag pins Python to the JS semantics.
2. ``re.fullmatch`` — Python ``$`` also matches just before a trailing
   newline; JS ``$`` (no ``m`` flag) matches only at end-of-string.
   ``fullmatch`` restores end-of-string-only semantics.
"""

from __future__ import annotations

import re

from ._errors import AgUiEventValidationError

__all__ = [
    "require_non_empty",
    "require_atomic",
    "require_rfc3339",
    "require_string_array",
    "require_safe_integer",
    "optional_entry",
]

# Non-negative atomic decimal string: no sign, no leading zeros.
# Character-identical to validate.ts ATOMIC_RE.
_ATOMIC_RE = re.compile(r"^(0|[1-9][0-9]*)$", re.ASCII)

# RFC 3339 format gate — format check only, no date parsing libs.
# Character-identical to validate.ts RFC3339_RE.
_RFC3339_RE = re.compile(
    r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$", re.ASCII
)

# 2^53 - 1 — JS Number.MAX_SAFE_INTEGER (design.md §7.5).
_MAX_SAFE_INTEGER = 2**53 - 1


def require_non_empty(field: str, s: object) -> str:
    """``isinstance(s, str) and len(s) > 0`` (no trimming — exactness)."""
    if not isinstance(s, str) or len(s) == 0:
        raise AgUiEventValidationError(
            field, f'field "{field}" must be a non-empty string'
        )
    return s


def require_atomic(field: str, s: object) -> str:
    if not isinstance(s, str) or _ATOMIC_RE.fullmatch(s) is None:
        raise AgUiEventValidationError(
            field,
            f'field "{field}" must be a non-negative atomic decimal string '
            "(no sign, no leading zeros)",
        )
    return s


def require_rfc3339(field: str, s: object) -> str:
    if not isinstance(s, str) or _RFC3339_RE.fullmatch(s) is None:
        raise AgUiEventValidationError(
            field, f'field "{field}" must be an RFC 3339 timestamp'
        )
    return s


def require_string_array(field: str, a: object, *, min_len: int) -> list[str]:
    """Array of non-empty strings; ``min_len`` 0 or 1 per design §5."""
    if not isinstance(a, (list, tuple)) or len(a) < min_len:
        raise AgUiEventValidationError(
            field,
            f'field "{field}" must be an array of non-empty strings '
            f"(>= {min_len} entries)",
        )
    for entry in a:
        if not isinstance(entry, str) or len(entry) == 0:
            raise AgUiEventValidationError(
                field, f'field "{field}" entries must be non-empty strings'
            )
    return list(a)


def require_safe_integer(field: str, n: object) -> int:
    """Non-negative safe integer. ``bool`` is rejected explicitly —
    Python ``bool`` is an ``int`` subclass but ``True`` is not a number
    in the canonical payload sense (mirrors TS ``typeof n === "number"``).
    Python has no integer ``-0`` so the TS ``Object.is(n, -0)`` arm has
    no Python twin to express."""
    if isinstance(n, bool) or not isinstance(n, int) or n < 0 or n > _MAX_SAFE_INTEGER:
        raise AgUiEventValidationError(
            field, f'field "{field}" must be a non-negative safe integer'
        )
    return n


def optional_entry(field: str, s: object) -> dict[str, str]:
    """Returns ``{field: s}`` when ``s`` is a non-empty string, else ``{}``.
    Never raises. This is the design.md §6 omit-if-empty collapse: empty
    string and absent are the same thing and serialize identically —
    load-bearing for cross-language byte-equivalence (HARDEN_D05_UR)."""
    if isinstance(s, str) and len(s) > 0:
        return {field: s}
    return {}
