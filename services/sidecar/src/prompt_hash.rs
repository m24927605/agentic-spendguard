//! Cost Advisor P0.5 — prompt_hash normalization + computation.
//!
//! Cost Advisor rules dedupe retried LLM calls by `(run_id, prompt_hash)`
//! per spec §5.1. The hash must be deterministic across Python adapters
//! (which compute it pre-call) AND the Rust sidecar (which carries it
//! into `canonical_events.payload_json`).
//!
//! Privacy: per codex P0.5 r1 P2, prompt_hash is **tenant-salted HMAC**
//! not plain SHA-256. HMAC-SHA256 with `tenant_id` as the key defeats
//! cross-tenant correlation (two tenants asking the same prompt produce
//! different hashes) and raises the bar against dictionary attacks on
//! common prompts. Rules dedupe WITHIN a tenant, where the HMAC key is
//! constant, so behavior is unchanged.
//!
//! Normalization rules (v1):
//!
//! 1. UTF-8 (any non-UTF8 byte → degraded: caller passes empty).
//! 2. Trim leading + trailing ASCII whitespace ONLY. We do NOT
//!    collapse internal whitespace because LLMs may be token-boundary-
//!    sensitive — `"hello world"` ≠ `"hello  world"` for
//!    fingerprinting purposes.
//! 3. NO Unicode normalization (NFC) in v1.
//! 4. Output: lowercase hex HMAC-SHA256 (64 chars).
//!
//! The Python adapter side (`sdk/python/src/spendguard/prompt_hash.py`)
//! implements the matching algorithm; the `SHARED_VECTORS` test set
//! below mirrors `sdk/python/tests/test_prompt_hash.py` and locks them
//! byte-equal.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Compute the canonical tenant-salted prompt hash.
///
/// `tenant_id` becomes the HMAC key; `prompt_text` is the message.
/// Returns 64-char lowercase hex string.
pub fn compute(prompt_text: &str, tenant_id: &str) -> String {
    let trimmed = prompt_text.trim_matches(is_ascii_whitespace);
    let mut mac = HmacSha256::new_from_slice(tenant_id.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(trimmed.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn is_ascii_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0C' | '\r')
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_TENANT: &str = "00000000-0000-4000-8000-000000000001";

    /// Pinned HMAC-SHA256 vectors. Verified against
    /// `printf '%s' "..." | openssl dgst -sha256 -hmac "$TEST_TENANT"`.
    /// Mirror in sdk/python tests.
    const SHARED_VECTORS: &[(&str, &str)] = &[
        // 1. Empty string.
        (
            "",
            "f35cfe956f859804e9c85f0f9b7ab40f754518045f0af59d5d0da0906f000a08",
        ),
        // 2. Simple ASCII prompt.
        (
            "What is the capital of France?",
            "fcc518b02824c4728ab70e698328685894e07a6f1fa1b19886407188425af723",
        ),
        // 3. Leading + trailing whitespace stripped to same hash as 2.
        (
            "  What is the capital of France?\n",
            "fcc518b02824c4728ab70e698328685894e07a6f1fa1b19886407188425af723",
        ),
        // 4. Internal whitespace preserved (DISTINCT from 2).
        (
            "What is  the capital of France?",
            "b0c13ce5053c66c6d3883662db65c6ce2034920e3bc4544ff370f070e9ed5bf4",
        ),
        // 5. Unicode prompt.
        (
            "Réponds en français.",
            "9a8c1201d05402bad1cb9eea3a6c09ffc6e905aab7dceaa787b5d6e05dadce0e",
        ),
    ];

    #[test]
    fn shared_vectors_match_pinned_hashes() {
        for (i, (input, expected)) in SHARED_VECTORS.iter().enumerate() {
            let got = compute(input, TEST_TENANT);
            assert_eq!(
                got, *expected,
                "vector {} drift: input={:?} expected={} got={}",
                i, input, expected, got
            );
        }
    }

    #[test]
    fn trim_collapses_outer_whitespace() {
        let canonical = compute("hello", TEST_TENANT);
        assert_eq!(compute("  hello", TEST_TENANT), canonical);
        assert_eq!(compute("hello\n", TEST_TENANT), canonical);
        assert_eq!(compute("\t hello \r\n", TEST_TENANT), canonical);
    }

    #[test]
    fn internal_whitespace_preserved() {
        assert_ne!(
            compute("hello world", TEST_TENANT),
            compute("hello  world", TEST_TENANT)
        );
        assert_ne!(
            compute("hello\nworld", TEST_TENANT),
            compute("hello world", TEST_TENANT)
        );
    }

    #[test]
    fn output_is_lowercase_hex() {
        let h = compute("anything", TEST_TENANT);
        assert_eq!(h.len(), 64);
        for c in h.chars() {
            assert!(c.is_ascii_digit() || ('a'..='f').contains(&c), "got {}", c);
        }
    }

    #[test]
    fn different_tenants_produce_different_hashes() {
        // P0.5 r1 P2 fix: cross-tenant linkability defeated.
        let same_prompt = "What is the capital of France?";
        let tenant_a = "11111111-1111-4111-8111-111111111111";
        let tenant_b = "22222222-2222-4222-8222-222222222222";
        assert_ne!(
            compute(same_prompt, tenant_a),
            compute(same_prompt, tenant_b)
        );
    }
}
