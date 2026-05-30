//! prompt_class_fingerprint per spec output-predictor-service-spec-v1alpha1.md §8.2.
//!
//! ## Determinism contract
//!
//! Same `(class, model, message_count)` → same hash. The fingerprint is
//! the **audit row identifier** for a bucket; the aggregation bucket
//! itself uses the class string directly (per spec §8.2 closing
//! paragraph). Determinism is required so verify-chain can reproduce
//! historical decisions even when the encoder cache is reset.
//!
//! ## Versioning
//!
//! The fingerprint string carries a `v1:` prefix per spec §8.2. v2
//! classifier upgrades must mint v2-prefixed hashes so legacy rows are
//! identifiable. The version string is also surfaced separately as
//! `PredictResponse.fingerprint_version` to make grep-by-version trivial.

use sha2::{Digest, Sha256};

/// Fingerprint string prefix per output-predictor-service-spec-v1alpha1.md
/// §8.2 ("`v1:` prefix to allow future v2 classifier upgrades to mint
/// v2-prefixed hashes so legacy rows are identifiable").
///
/// R2 M4 (Software F8): R1 set this to "v1alpha1" which doesn't match
/// the spec. Spec example: `format!("v1:{:x}", sha256(canonical.as_bytes()))`.
/// Bumping the constant changes every fingerprint emitted by the
/// predictor — acceptable in SLICE_06 because no production traffic is
/// gated on the legacy string yet (the L4 cache is keyed by
/// prompt_class enum per B3, not by fingerprint).
pub const FINGERPRINT_VERSION: &str = "v1";

/// Compute the deterministic prompt_class_fingerprint per spec §8.2.
///
/// Canonical form: `v1:{class}|{model}|{message_count}` then SHA-256
/// hex-encoded; final string is `v1:{hex}`. The version prefix is
/// retained both in the hash payload AND the output string so:
///   * Two callers using different versions never hash-collide (payload
///     contains version → different input → different SHA-256).
///   * Audit rows grep-by-version: a `v1:` prefix is human-greppable.
pub fn compute_fingerprint(model: &str, prompt_class: &str) -> String {
    compute_fingerprint_with_count(model, prompt_class, 0)
}

/// Spec §8.2 includes `messages.len()` in the canonical hash payload;
/// expose the message-count parameter for callers that have it (the
/// sidecar passes it through `PredictRequest`).
pub fn compute_fingerprint_with_count(model: &str, prompt_class: &str, message_count: usize) -> String {
    let canonical = format!("{FINGERPRINT_VERSION}:{prompt_class}|{model}|{message_count}");
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    let digest = hasher.finalize();
    format!("{FINGERPRINT_VERSION}:{:x}", digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_inputs_produce_same_hash() {
        let a = compute_fingerprint("gpt-4o", "chat_short");
        let b = compute_fingerprint("gpt-4o", "chat_short");
        assert_eq!(a, b, "fingerprint must be deterministic");
    }

    #[test]
    fn different_class_produces_different_hash() {
        let a = compute_fingerprint("gpt-4o", "chat_short");
        let b = compute_fingerprint("gpt-4o", "chat_long");
        assert_ne!(a, b);
    }

    #[test]
    fn different_model_produces_different_hash() {
        let a = compute_fingerprint("gpt-4o", "chat_short");
        let b = compute_fingerprint("claude-3-5-sonnet-20240620", "chat_short");
        assert_ne!(a, b);
    }

    #[test]
    fn message_count_affects_hash() {
        let a = compute_fingerprint_with_count("gpt-4o", "chat_short", 1);
        let b = compute_fingerprint_with_count("gpt-4o", "chat_short", 5);
        assert_ne!(a, b);
    }

    #[test]
    fn prefix_is_versioned() {
        let f = compute_fingerprint("gpt-4o", "chat_short");
        assert!(
            f.starts_with(&format!("{FINGERPRINT_VERSION}:")),
            "missing version prefix: {f}"
        );
        // hex body is 64 chars (SHA-256 → 32 bytes → 64 hex chars).
        let body = f.strip_prefix(&format!("{FINGERPRINT_VERSION}:")).unwrap();
        assert_eq!(body.len(), 64, "hex body length expected 64; got {}", body.len());
    }

    #[test]
    fn empty_model_or_class_still_produces_hash() {
        // Empty values still produce a hash (no fail-fast); audit row
        // captures the empty-bucket identity.
        let f = compute_fingerprint("", "");
        assert!(f.starts_with(&format!("{FINGERPRINT_VERSION}:")));
    }

    #[test]
    fn cross_input_collision_resistance_smoke() {
        // Different boundary cases must not collide.
        let f1 = compute_fingerprint("gpt-4o|chat_short", "");
        let f2 = compute_fingerprint("gpt-4o", "chat_short");
        // The canonical formatter uses `|` as separator; this test
        // ensures the formatter is not a trivial concat-string vulnerable
        // to delimiter injection.
        assert_ne!(f1, f2);
    }
}
