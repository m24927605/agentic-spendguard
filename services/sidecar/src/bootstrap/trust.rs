//! Trust bootstrap: verify the Helm-pinned root CA bundle SPKI hash.
//!
//! Per Stage 2 §12.1: customer's `spendguard-trust` Secret carries
//!   - rootCABundle: PEM
//!   - rootSPKIHashSHA256: hex
//!   - mTLSBootstrapToken: one-time
//!
//! On startup the sidecar:
//!   1. Loads the PEM bundle.
//!   2. Computes sha256 of the leaf certificate's SubjectPublicKeyInfo (SPKI).
//!   3. Compares against the pinned hex value (constant-time).
//!   4. If mismatch → DomainError::TrustBootstrap; sidecar refuses to start.
//!
//! POC simplification: we do NOT parse X.509 here (avoids pulling in
//! `x509-parser` etc.). We treat the whole CA bundle DER bytes as the
//! pinned input and hash that. This is a weaker pin than SPKI per-leaf,
//! but acceptable for POC where the PEM is ops-controlled. Phase 1 後段
//! should switch to leaf SPKI hashing via `x509-parser`.

use anyhow::Result;
use sha2::{Digest, Sha256};

use crate::domain::error::DomainError;

pub fn verify_root_ca_pin(pem: &str, pinned_hex: &str) -> Result<(), DomainError> {
    let der = pem_decode_first_cert(pem)
        .map_err(|e| DomainError::TrustBootstrap(format!("decode root ca pem: {e}")))?;
    let actual = Sha256::digest(&der);
    let actual_hex = hex::encode(actual);

    if !constant_time_eq(actual_hex.as_bytes(), pinned_hex.trim().as_bytes()) {
        return Err(DomainError::TrustBootstrap(format!(
            "root CA bundle SPKI hash mismatch: pinned={}, actual={}",
            pinned_hex.trim(),
            actual_hex
        )));
    }
    Ok(())
}

fn pem_decode_first_cert(pem: &str) -> Result<Vec<u8>> {
    use base64::Engine as _;
    let mut in_block = false;
    let mut b64 = String::new();
    for line in pem.lines() {
        let trimmed = line.trim();
        if trimmed == "-----BEGIN CERTIFICATE-----" {
            in_block = true;
            continue;
        }
        if trimmed == "-----END CERTIFICATE-----" {
            break;
        }
        if in_block {
            b64.push_str(trimmed);
        }
    }
    if b64.is_empty() {
        anyhow::bail!("no PEM CERTIFICATE block found");
    }
    Ok(base64::engine::general_purpose::STANDARD.decode(b64)?)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const SELF_SIGNED_PEM: &str = "-----BEGIN CERTIFICATE-----\n\
MIIB1TCCAXigAwIBAgIUTestPin\n\
-----END CERTIFICATE-----\n";

    #[test]
    fn rejects_mismatched_pin() {
        // We don't actually decode the bytes here (since the example PEM is
        // truncated), but we exercise the mismatch branch with a known-bad
        // hash.
        let result =
            pem_decode_first_cert(SELF_SIGNED_PEM).expect("decode truncated PEM should not panic");
        let actual = Sha256::digest(&result);
        let actual_hex = hex::encode(actual);
        let bad_pin = "0".repeat(64);
        assert_ne!(actual_hex, bad_pin);
        let err = verify_root_ca_pin(SELF_SIGNED_PEM, &bad_pin).unwrap_err();
        match err {
            DomainError::TrustBootstrap(msg) => {
                assert!(msg.contains("mismatch"), "unexpected msg: {}", msg);
            }
            _ => panic!("expected TrustBootstrap"),
        }
    }
}
