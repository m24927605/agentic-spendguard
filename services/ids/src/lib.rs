//! Cross-language deterministic ID derivation for SpendGuard.
//!
//! Mirrors `sdk/python/src/spendguard/ids.py` byte-for-byte for the
//! subset of helpers that need cross-language convergence:
//!
//! - `default_call_signature_jcs(body) -> String` — blake2b-128 over
//!   canonicalized JSON body. Returns 32 hex chars. NEW for the
//!   proxy; the Python SDK's `default_call_signature` (ids.py:104)
//!   operates on different inputs (Pydantic-AI messages, not JSON
//!   bodies) — the two functions are NOT byte-equivalent and aren't
//!   supposed to be. Cross-mode convergence is at the
//!   `derive_uuid_from_signature` boundary, not here.
//! - `derive_uuid_from_signature(signature, scope) -> Uuid` —
//!   UUIDv4-shape (blake2b-masked to v4 bits). NOT RFC 4122 v5.
//!   **Byte-equivalent with `sdk/python/src/spendguard/ids.py:161-173`**.
//!   Cross-language fixture at `tests/fixtures/python_v1.json` is the
//!   load-bearing contract.
//!
//! The load-bearing test (`tests/byte_equivalence.rs`) loads a JSON
//! fixture committed by the Python SDK and asserts each (signature,
//! scope) → UUID mapping matches. CI runs the fixture generation
//! script + the Rust test in lockstep — any divergence fails CI.
//!
//! Per `docs/specs/auto-instrument-egress-proxy-spec.md` v7 §4.1.5
//! + Staff escalation r5 consensus (3-of-4 majority on hash function
//! and UUID flavor).

use blake2::{digest::consts::U16, Blake2b, Digest};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

type Blake2b128 = Blake2b<U16>;

#[derive(Error, Debug)]
pub enum IdsError {
    /// Codex r5 Staff #3 operability fix: explicit error, no silent
    /// `repr()` fallback like the Python helper. Callers (e.g. the
    /// egress proxy) increment `egress_proxy_unserializable_total`
    /// counter and 400 / 502 the request.
    #[error("body not serializable to canonical JSON: {0}")]
    Unserializable(#[from] serde_json::Error),

    #[error("hex decode error: {0}")]
    HexDecode(String),
}

/// Compute the 32-hex-char (128-bit) blake2b signature of a JSON
/// body, with deterministic key ordering.
///
/// Canonicalization: `serde_json::to_vec` with feature
/// `preserve_order` disabled (sorted by insertion order — we
/// explicitly re-serialize via `to_value` which uses `BTreeMap` style
/// for objects to enforce sorted-key order). This matches the
/// Python SDK's `sort_keys=True` invocation at `ids.py:154`.
///
/// v0.2 will tighten to RFC 8785 JCS (full Unicode NFC, number
/// normalization). v0.1 floor is sorted-keys.
pub fn default_call_signature_jcs(body: &Value) -> Result<String, IdsError> {
    let canonical = canonicalize(body)?;
    let mut hasher = Blake2b128::new();
    hasher.update(canonical.as_bytes());
    let out = hasher.finalize();
    Ok(hex::encode(out))
}

/// Sorted-key JSON serialization. serde_json's `to_string` does NOT
/// sort keys by default; we walk the tree and re-emit with sorted
/// object keys to match Python's `json.dumps(sort_keys=True)`.
fn canonicalize(value: &Value) -> Result<String, IdsError> {
    let canonical = sort_keys(value);
    serde_json::to_string(&canonical).map_err(IdsError::from)
}

fn sort_keys(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            // Re-emit with BTreeMap to enforce sorted keys.
            let mut sorted = std::collections::BTreeMap::new();
            for (k, v) in map {
                sorted.insert(k.clone(), sort_keys(v));
            }
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(items) => {
            Value::Array(items.iter().map(sort_keys).collect())
        }
        // Scalars are byte-identical regardless of canonicalization.
        other => other.clone(),
    }
}

/// Derive a deterministic UUIDv4-shape from a signature + scope.
///
/// Byte-equivalent with `sdk/python/src/spendguard/ids.py:161-173`:
///
/// ```python
/// digest = hashlib.blake2b(f"{scope}|{signature}".encode("utf-8"),
///                          digest_size=16).digest()
/// buf = bytearray(digest)
/// buf[6] = (buf[6] & 0x0F) | 0x40  # version 4
/// buf[8] = (buf[8] & 0x3F) | 0x80  # variant 10
/// ```
///
/// **Pipe separator is load-bearing** (codex slice-2 r1 P1.1 fix):
/// `f"{scope}|{signature}"` is the exact Python format. A naive
/// concat without `|` would produce different digests AND would
/// introduce a collision class: `("scope1", "Xabc")` would equal
/// `("scope1X", "abc")` without a separator. The `|` is what makes
/// the scoping namespace actually scope.
///
/// NOT RFC 4122 UUIDv5 (which is SHA-1 based). Codex r5 Staff #2
/// security review confirmed v4-shape masking is acceptable for
/// content-addressing labels.
pub fn derive_uuid_from_signature(signature: &str, scope: &str) -> Uuid {
    let mut hasher = Blake2b128::new();
    // Match Python exactly: scope, then literal "|", then signature.
    // Python: hashlib.blake2b(f"{scope}|{signature}".encode("utf-8"), digest_size=16).
    hasher.update(scope.as_bytes());
    hasher.update(b"|");
    hasher.update(signature.as_bytes());
    let mut bytes: [u8; 16] = hasher.finalize().into();

    // Mask to UUIDv4 shape:
    //   byte 6: clear top 4 bits, set 0x40 (version = 4)
    //   byte 8: clear top 2 bits, set 0x80 (variant = RFC 4122)
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    Uuid::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn signature_is_32_hex_chars() {
        let body = json!({"model": "gpt-4o-mini", "messages": [{"role": "user", "content": "hi"}]});
        let sig = default_call_signature_jcs(&body).unwrap();
        assert_eq!(sig.len(), 32);
        assert!(sig.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn signature_is_deterministic_across_key_order() {
        let a = json!({"model": "gpt-4o", "stream": false});
        let b = json!({"stream": false, "model": "gpt-4o"});
        assert_eq!(
            default_call_signature_jcs(&a).unwrap(),
            default_call_signature_jcs(&b).unwrap()
        );
    }

    #[test]
    fn signature_changes_with_content() {
        let a = json!({"messages": [{"role": "user", "content": "hi"}]});
        let b = json!({"messages": [{"role": "user", "content": "hello"}]});
        assert_ne!(
            default_call_signature_jcs(&a).unwrap(),
            default_call_signature_jcs(&b).unwrap()
        );
    }

    #[test]
    fn derive_uuid_is_v4_shape() {
        let u = derive_uuid_from_signature("abc", "scope");
        // Version 4 — high nibble of bytes[6]
        assert_eq!(u.as_bytes()[6] >> 4, 4);
        // Variant 10 (RFC 4122) — top 2 bits of bytes[8]
        assert_eq!(u.as_bytes()[8] >> 6, 0b10);
    }

    #[test]
    fn derive_uuid_is_deterministic() {
        let a = derive_uuid_from_signature("abc123", "decision_id");
        let b = derive_uuid_from_signature("abc123", "decision_id");
        assert_eq!(a, b);
    }

    #[test]
    fn derive_uuid_differs_by_scope() {
        let a = derive_uuid_from_signature("abc123", "decision_id");
        let b = derive_uuid_from_signature("abc123", "llm_call_id");
        assert_ne!(a, b);
    }

    #[test]
    fn derive_uuid_differs_by_signature() {
        let a = derive_uuid_from_signature("aaa", "scope");
        let b = derive_uuid_from_signature("bbb", "scope");
        assert_ne!(a, b);
    }

    #[test]
    fn nested_objects_sort_recursively() {
        let a = json!({"outer": {"a": 1, "b": 2}});
        let b = json!({"outer": {"b": 2, "a": 1}});
        assert_eq!(
            default_call_signature_jcs(&a).unwrap(),
            default_call_signature_jcs(&b).unwrap()
        );
    }

    #[test]
    fn array_order_is_meaningful() {
        // Arrays preserve order (semantically meaningful in OpenAI messages list).
        let a = json!([{"role": "user"}, {"role": "assistant"}]);
        let b = json!([{"role": "assistant"}, {"role": "user"}]);
        assert_ne!(
            default_call_signature_jcs(&a).unwrap(),
            default_call_signature_jcs(&b).unwrap()
        );
    }
}
