//! Cost Advisor P0.5 — prompt_hash normalization + computation.
//!
//! Cost Advisor rules dedupe retried LLM calls by `(run_id, prompt_hash)`
//! per spec §5.1. The hash must be deterministic across Python adapters
//! (which compute it pre-call) AND the Rust sidecar (which carries it
//! into `canonical_events.payload_json`).
//!
//! Normalization rules (v1 — see audit-report §0.1 + CA-P0.5-prompt.md):
//!
//! 1. UTF-8 (any non-UTF8 byte → degraded: return None / empty).
//! 2. Trim leading + trailing ASCII whitespace ONLY. We do NOT
//!    collapse internal whitespace because LLMs may be token-boundary-
//!    sensitive — `"hello world"` ≠ `"hello  world"` for
//!    fingerprinting purposes.
//! 3. NO Unicode normalization (NFC) in v1. Most adapters generate
//!    prompts in normalized form already (Python `str` and Node strings
//!    default to NFC-equivalent producers). If two prompts that visually
//!    look identical produce different hashes due to NFC/NFD mismatch,
//!    that's a v0.2 issue and a unicode-normalization dep will be
//!    introduced then. Document as a known limit in the spec FAQ.
//! 4. Output: lowercase hex SHA-256 (matches `FindingEvidence.fingerprint`
//!    encoding in `services/cost_advisor/src/fingerprint.rs`).
//!
//! The Python adapter side (`adapters/pydantic-ai/spendguard_pydantic_ai
//! /prompt_hash.py`) implements the matching algorithm; test vectors
//! shared via `tests/prompt_hash_vectors.rs` keep them in lockstep.

use sha2::{Digest, Sha256};

/// Compute the canonical prompt hash for the given prompt text.
///
/// Returns lowercase hex SHA-256 (64 chars). For an empty prompt
/// (after trim), returns the SHA-256 of empty bytes — caller decides
/// whether that's a valid identifier or should be skipped.
pub fn compute(prompt_text: &str) -> String {
    let trimmed = prompt_text.trim_matches(is_ascii_whitespace);
    hex::encode(Sha256::digest(trimmed.as_bytes()))
}

/// Same set as Rust's `char::is_ascii_whitespace`, exposed here for
/// explicit-spec parity with the Python side. Specifically:
///   space (0x20), tab (0x09), newline (0x0A), form feed (0x0C),
///   carriage return (0x0D).
/// Note: this set does NOT include vertical tab (0x0B) or NBSP (0xA0).
fn is_ascii_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0C' | '\r')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shared test vectors. The Python adapter has identical vectors
    /// in adapters/pydantic-ai/tests/test_prompt_hash.py — any drift is
    /// a P0 bug for cost_advisor's run-scoped dedup. Expected hashes
    /// are the literal SHA-256 of (trimmed) bytes; verified against
    /// `printf '%s' "..." | shasum -a 256` at vector creation time.
    const SHARED_VECTORS: &[(&str, &str)] = &[
        // 1. Empty string → SHA-256 of empty bytes (RFC 6234).
        (
            "",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        // 2. Simple ASCII prompt.
        (
            "What is the capital of France?",
            "115049a298532be2f181edb03f766770c0db84c22aff39003fec340deaec7545",
        ),
        // 3. Leading + trailing whitespace stripped to same hash as 2.
        (
            "  What is the capital of France?\n",
            "115049a298532be2f181edb03f766770c0db84c22aff39003fec340deaec7545",
        ),
        // 4. Internal whitespace preserved (DISTINCT from 2).
        (
            "What is  the capital of France?",
            "4dfda0d35066824cbcb5d92f0345eb3644089840a411f97da9753f66a85bdf2d",
        ),
        // 5. Unicode prompt (NFC byte-identical from str/String).
        (
            "Réponds en français.",
            "7dc9af64dd6ef3f824acb8bf5a6f6e5a7c2b08587f4b0720ddb66a46bc791663",
        ),
    ];

    #[test]
    fn shared_vectors_match_pinned_hashes() {
        for (i, (input, expected)) in SHARED_VECTORS.iter().enumerate() {
            let got = compute(input);
            assert_eq!(
                got, *expected,
                "vector {} drift: input={:?} expected={} got={}",
                i, input, expected, got
            );
        }
    }

    #[test]
    fn trim_collapses_outer_whitespace() {
        let canonical = compute("hello");
        assert_eq!(compute("  hello"), canonical);
        assert_eq!(compute("hello\n"), canonical);
        assert_eq!(compute("\t hello \r\n"), canonical);
    }

    #[test]
    fn internal_whitespace_preserved() {
        // Two distinct hashes — internal whitespace is semantically
        // load-bearing for LLM tokenization.
        assert_ne!(compute("hello world"), compute("hello  world"));
        assert_ne!(compute("hello\nworld"), compute("hello world"));
    }

    #[test]
    fn output_is_lowercase_hex() {
        let h = compute("anything");
        assert_eq!(h.len(), 64);
        for c in h.chars() {
            assert!(c.is_ascii_digit() || ('a'..='f').contains(&c), "got {}", c);
        }
    }

    #[test]
    fn empty_prompt_is_sha256_of_empty() {
        // RFC 6234 test vector: SHA-256("") =
        // e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        assert_eq!(
            compute(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        // After trim, "  " also is empty bytes.
        assert_eq!(compute("  "), compute(""));
    }
}
