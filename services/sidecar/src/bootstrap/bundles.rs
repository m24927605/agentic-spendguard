//! Bundle Registry pull + cosign verify + cache (Stage 2 §5).
//!
//! POC: bundles are loaded from a local directory baked by the
//! customer's CI mirror (e.g., Helm-init-container-loaded). Production
//! uses OCI pull + cosign verify against the Helm-pinned trust root.
//! This module exposes a Trait so tests + Phase 1 後段 can swap backends.

use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::domain::{
    error::DomainError,
    state::{CachedContractBundle, CachedSchemaBundle, SidecarState},
};

#[derive(Debug, Clone)]
pub struct BundleSource {
    /// Local directory holding bundle.tgz + sigstore signature .sig file.
    /// POC default `/var/lib/spendguard/bundles/`.
    pub root: PathBuf,
}

impl Default for BundleSource {
    fn default() -> Self {
        Self {
            root: "/var/lib/spendguard/bundles".into(),
        }
    }
}

/// Load + verify a contract bundle by id. Returns CachedContractBundle.
///
/// Cosign verification stub: POC checks the .sig file's existence + size
/// only. Phase 1 後段 wires real cosign verify against the Helm-pinned
/// trust root (the OIDC certificate identity from GHA workflows).
pub fn load_contract_bundle(
    source: &BundleSource,
    bundle_id: Uuid,
    expected_hash_hex: &str,
) -> Result<CachedContractBundle, DomainError> {
    let path = source.root.join(format!("contract_bundle/{bundle_id}.tgz"));
    let raw = std::fs::read(&path).map_err(|e| {
        DomainError::Internal(anyhow!("read contract bundle {}: {}", path.display(), e))
    })?;
    let actual_hash = Sha256::digest(&raw);
    let actual_hex = hex::encode(actual_hash);
    if actual_hex != expected_hash_hex {
        return Err(DomainError::BundleSignatureInvalid(format!(
            "contract bundle {} hash mismatch: expected={}, actual={}",
            bundle_id, expected_hash_hex, actual_hex
        )));
    }
    let sig_path = source
        .root
        .join(format!("contract_bundle/{bundle_id}.tgz.sig"));
    let sig = std::fs::metadata(&sig_path).map_err(|e| {
        DomainError::BundleSignatureInvalid(format!(
            "no signature file at {}: {}",
            sig_path.display(),
            e
        ))
    })?;
    if sig.len() == 0 {
        return Err(DomainError::BundleSignatureInvalid(
            "signature file empty".into(),
        ));
    }
    // TODO Phase 1 後段: cosign verify against pinned trust root.

    // POC: extract pricing snapshot fields via a sibling JSON metadata
    // file. Real format embeds these as OCI annotations on the artifact.
    let meta_path = source
        .root
        .join(format!("contract_bundle/{bundle_id}.metadata.json"));
    let meta_bytes = std::fs::read(&meta_path).with_context(|| {
        format!("read bundle metadata {}", meta_path.display())
    }).map_err(|e| DomainError::Internal(anyhow!(e)))?;
    let meta: BundleMetadata = serde_json::from_slice(&meta_bytes).map_err(|e| {
        DomainError::BundleSignatureInvalid(format!("bundle metadata json: {e}"))
    })?;

    let snapshot_hash = hex::decode(&meta.price_snapshot_hash).map_err(|e| {
        DomainError::BundleSignatureInvalid(format!("price_snapshot_hash hex: {e}"))
    })?;

    // Phase 3 wedge: parse contract.yaml out of the bundle tarball so the
    // hot-path evaluator (`decision/transaction.rs` Stage 2) can read
    // structured rules. Fail-closed: malformed YAML → sidecar refuses
    // to start. Silent fallback to "no rules → CONTINUE everything" is
    // worse than refusing to come up — it would leave decisions
    // ungated with no audit trace of what should have gated them.
    let parsed = crate::contract::parse_from_tgz(&raw).map_err(|e| {
        DomainError::BundleSignatureInvalid(format!(
            "contract bundle {} parse: {:#}",
            bundle_id, e
        ))
    })?;

    Ok(CachedContractBundle {
        bundle_id,
        bundle_hash: actual_hash.to_vec(),
        signing_key_id: meta.signing_key_id,
        raw,
        pricing_version: meta.pricing_version,
        price_snapshot_hash: snapshot_hash,
        fx_rate_version: meta.fx_rate_version,
        unit_conversion_version: meta.unit_conversion_version,
        parsed: std::sync::Arc::new(parsed),
    })
}

/// Schema bundle is content-addressed (sha256). Sidecar pulls once at
/// startup; updates only when contract bundle pins a new schema version.
pub fn load_schema_bundle(
    source: &BundleSource,
    bundle_id: Uuid,
    canonical_schema_version: &str,
) -> Result<CachedSchemaBundle, DomainError> {
    let path = source.root.join(format!("schema_bundle/{bundle_id}.tgz"));
    let raw = std::fs::read(&path).map_err(|e| {
        DomainError::Internal(anyhow!("read schema bundle {}: {}", path.display(), e))
    })?;
    let hash = Sha256::digest(&raw).to_vec();
    Ok(CachedSchemaBundle {
        bundle_id,
        bundle_hash: hash,
        canonical_schema_version: canonical_schema_version.to_string(),
    })
}

#[derive(Debug, serde::Deserialize)]
struct BundleMetadata {
    pricing_version: String,
    price_snapshot_hash: String, // hex
    fx_rate_version: String,
    unit_conversion_version: String,
    signing_key_id: String,
}

/// Atomically swap a freshly-loaded contract bundle into the runtime
/// state. Returns the previous bundle id (if any) for telemetry.
pub fn install_contract_bundle(state: &SidecarState, bundle: CachedContractBundle) -> Option<Uuid> {
    let prev = state.inner.contract_bundle.read().as_ref().map(|b| b.bundle_id);
    *state.inner.contract_bundle.write() = Some(bundle);
    prev
}

pub fn install_schema_bundle(state: &SidecarState, bundle: CachedSchemaBundle) -> Option<Uuid> {
    let prev = state.inner.schema_bundle.read().as_ref().map(|b| b.bundle_id);
    *state.inner.schema_bundle.write() = Some(bundle);
    prev
}
