"""Cost Advisor P0.5 — prompt_hash normalization + computation (Python).

Cost Advisor rules dedupe retried LLM calls by ``(run_id, prompt_hash)``
per spec §5.1. The hash must be deterministic across this Python adapter
AND the Rust sidecar; matching implementation in
``services/sidecar/src/prompt_hash.rs``.

Privacy (codex P0.5 r1 P2): prompt_hash is **tenant-salted HMAC**, not
plain SHA-256. HMAC-SHA256 with ``tenant_id`` as the key defeats
cross-tenant correlation and raises the bar against dictionary attacks
on common prompts. Rules dedupe within a tenant where the key is
constant, so behavior is unchanged.

Normalization (v1):
1. UTF-8 only.
2. Trim leading + trailing ASCII whitespace (space, tab, newline, FF, CR).
3. NO Unicode normalization (NFC) in v1.
4. Output: 64-char lowercase hex HMAC-SHA256.
"""

from __future__ import annotations

import hashlib
import hmac

# Mirror Rust's `char::is_ascii_whitespace` set.
_ASCII_WHITESPACE = " \t\n\x0c\r"


def compute(prompt_text: str, tenant_id: str) -> str:
    """Return lowercase hex HMAC-SHA256 of (normalized) prompt with tenant key.

    Determinism: ``compute(s, t) == compute(s, t)`` for any (s, t).
    Cross-language: byte-for-byte identical to Rust
    ``services::sidecar::prompt_hash::compute`` for the shared test
    vectors. Cross-tenant: two tenants asking the same prompt produce
    different hashes.
    """
    trimmed = prompt_text.strip(_ASCII_WHITESPACE)
    return hmac.new(
        tenant_id.encode("utf-8"),
        trimmed.encode("utf-8"),
        hashlib.sha256,
    ).hexdigest()


__all__ = ["compute"]
