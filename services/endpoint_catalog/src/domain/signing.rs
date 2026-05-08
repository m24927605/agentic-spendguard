//! ed25519 signing for manifests (per Sidecar §8 + Stage 2 §8.2.4).
//!
//! Canonical signing input: the manifest body (excluding `signature`)
//! serialized to canonical JSON via serde_json's deterministic ordering
//! (we then sort top-level fields alphabetically for stable canonical bytes).

use anyhow::{Context, Result};
use base64::Engine as _;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use serde_json::Value;

use super::manifest::{Manifest, ManifestSigningBody};

/// Load ed25519 signing key from PKCS#8 PEM file.
pub fn load_signing_key_pem(path: &str) -> Result<SigningKey> {
    use ed25519_dalek::pkcs8::DecodePrivateKey;
    let pem = std::fs::read_to_string(path).with_context(|| format!("read {}", path))?;
    let key = SigningKey::from_pkcs8_pem(&pem).context("parse pkcs8 pem")?;
    Ok(key)
}

/// Sign a manifest body. Returns the base64 signature.
pub fn sign_manifest_body(
    key: &SigningKey,
    body: &ManifestSigningBody<'_>,
) -> Result<String> {
    let canonical = canonicalize(serde_json::to_value(body)?)?;
    let sig = key.sign(canonical.as_bytes());
    Ok(base64::engine::general_purpose::STANDARD.encode(sig.to_bytes()))
}

/// Verify a manifest signature against the embedded body fields.
pub fn verify_manifest(verify_key: &VerifyingKey, manifest: &Manifest) -> Result<()> {
    use ed25519_dalek::Verifier;

    let body = ManifestSigningBody {
        manifest_version: &manifest.manifest_version,
        current_catalog_version_id: &manifest.current_catalog_version_id,
        current_catalog_url: &manifest.current_catalog_url,
        current_catalog_hash: &manifest.current_catalog_hash,
        issued_at: manifest.issued_at,
        valid_until: manifest.valid_until,
        signing_key_id: &manifest.signing_key_id,
        tenant_overrides: &manifest.tenant_overrides,
    };
    let canonical = canonicalize(serde_json::to_value(&body)?)?;

    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(&manifest.signature)
        .context("base64 signature")?;
    let sig: ed25519_dalek::Signature = sig_bytes
        .as_slice()
        .try_into()
        .context("signature length")?;
    verify_key
        .verify(canonical.as_bytes(), &sig)
        .context("signature mismatch")?;
    Ok(())
}

/// Canonical JSON: sort object keys recursively, no whitespace, UTF-8.
/// Stable bytes for cryptographic signing.
fn canonicalize(value: Value) -> Result<String> {
    let sorted = sort_keys(value);
    Ok(serde_json::to_string(&sorted)?)
}

fn sort_keys(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut out = serde_json::Map::with_capacity(entries.len());
            for (k, v) in entries {
                out.insert(k, sort_keys(v));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_keys).collect()),
        other => other,
    }
}

/// sha256 of the canonical JSON bytes of a JSON value.
pub fn canonical_sha256_hex(value: &Value) -> Result<String> {
    use sha2::{Digest, Sha256};
    let canonical = canonicalize(value.clone())?;
    let digest = Sha256::digest(canonical.as_bytes());
    Ok(hex::encode(digest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use chrono::Utc;
    use crate::domain::manifest::{Manifest, ManifestSigningBody};

    #[test]
    fn sign_and_verify_roundtrip() {
        let mut rng = OsRng;
        let signing = SigningKey::generate(&mut rng);
        let verifying = signing.verifying_key();

        let body = ManifestSigningBody {
            manifest_version: "1.0.0",
            current_catalog_version_id: "ctlg-2026-05-07T10:00Z-rev42",
            current_catalog_url: "https://catalog.example/v1/catalog/ctlg-2026-05-07T10:00Z-rev42",
            current_catalog_hash: "abc",
            issued_at: Utc::now(),
            valid_until: Utc::now() + chrono::Duration::seconds(300),
            signing_key_id: "key-1",
            tenant_overrides: &[],
        };
        let signature = sign_manifest_body(&signing, &body).unwrap();

        let manifest = Manifest {
            manifest_version: body.manifest_version.into(),
            current_catalog_version_id: body.current_catalog_version_id.into(),
            current_catalog_url: body.current_catalog_url.into(),
            current_catalog_hash: body.current_catalog_hash.into(),
            issued_at: body.issued_at,
            valid_until: body.valid_until,
            signing_key_id: body.signing_key_id.into(),
            signature,
            tenant_overrides: vec![],
        };

        verify_manifest(&verifying, &manifest).expect("signature should verify");
    }
}
