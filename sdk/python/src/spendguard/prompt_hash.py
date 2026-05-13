"""Cost Advisor P0.5 — prompt_hash normalization + computation (Python).

Cost Advisor rules dedupe retried LLM calls by ``(run_id, prompt_hash)``
per spec §5.1. The hash must be deterministic across this Python adapter
AND the Rust sidecar; the matching Rust implementation lives in
``services/sidecar/src/prompt_hash.rs``.

The normalization rules (v1):

1. UTF-8 only. Non-UTF8 input is a programmer error — the SDK accepts
   ``str`` which is UTF-8 by construction.
2. Trim leading + trailing ASCII whitespace ONLY (space, tab, newline,
   form feed, carriage return). Internal whitespace stays untouched
   because LLMs may be token-boundary-sensitive.
3. NO Unicode normalization (NFC) in v1 — most adapters produce NFC by
   default. NFC will land in v0.2 when we also wire LangChain support.
4. Output: lowercase hex SHA-256 (64 chars), matching the Rust side.

The shared test vector set lives in
``sdk/python/tests/test_prompt_hash.py`` and mirrors
``services/sidecar/src/prompt_hash.rs``'s ``SHARED_VECTORS``. Any drift
between the two is a P0 bug — cost_advisor's run-scope dedup breaks
silently if Python and Rust disagree on the hash.
"""

from __future__ import annotations

import hashlib

# Same set as Rust's `char::is_ascii_whitespace` — space, tab, newline,
# form feed, carriage return. Explicit-spec parity with the Rust side.
# NOTE: vertical tab (0x0B) and NBSP (0xA0) are intentionally NOT in
# this set, mirroring Rust's behavior.
_ASCII_WHITESPACE = " \t\n\x0c\r"


def compute(prompt_text: str) -> str:
    """Return lowercase hex SHA-256 of the canonically-normalized prompt.

    Determinism guarantee: ``compute(s) == compute(s)`` for any str ``s``.
    Cross-language guarantee: byte-for-byte identical to the Rust side's
    ``services::sidecar::prompt_hash::compute`` for the shared test
    vectors.

    Returns a 64-char lowercase hex string. For empty input (after trim)
    returns ``e3b0c4...`` (SHA-256 of empty bytes).
    """
    trimmed = prompt_text.strip(_ASCII_WHITESPACE)
    return hashlib.sha256(trimmed.encode("utf-8")).hexdigest()


# Re-exported for use in cross-language test fixtures + SDK callers
# that want to verify a specific normalization decision.
__all__ = ["compute"]
