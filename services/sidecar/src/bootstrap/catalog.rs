//! Endpoint Catalog manifest pull + verify + cache refresh loop.
//!
//! Per Sidecar §8 + Stage 2 §8.2.4:
//!   * GET /v1/catalog/manifest (no-cache); verify ed25519 signature
//!     against the Helm-pinned root.
//!   * GET /v1/catalog/{version_id} (immutable); verify body sha256
//!     matches manifest.current_catalog_hash.
//!   * Cache + atomically swap into SidecarState.
//!
//! Refresh cadence: every `manifest_pull_seconds`. Sidecars fail-closed
//! for enforcement routes if `last_verified_critical_version_age` exceeds
//! `critical_max_stale_seconds` (Sidecar §7).

use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use chrono::{DateTime, SecondsFormat, Utc};
use ed25519_dalek::{Verifier, VerifyingKey};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::Duration;
use tracing::{info, warn};

use crate::{
    config::Config,
    domain::{
        error::DomainError,
        state::{CachedCatalog, SidecarState},
    },
};

#[derive(Debug, Clone, Deserialize)]
struct Manifest {
    manifest_version: String,
    current_catalog_version_id: String,
    current_catalog_url: String,
    current_catalog_hash: String, // hex
    issued_at: DateTime<Utc>,
    valid_until: DateTime<Utc>,
    signing_key_id: String,
    signature: String, // base64
    #[serde(default)]
    tenant_overrides: Vec<serde_json::Value>,
}

pub async fn refresh_loop(cfg: Config, state: SidecarState, verifying_key: VerifyingKey) {
    loop {
        match refresh_once(&cfg, &state, &verifying_key).await {
            Ok(version_id) => {
                info!(version_id = %version_id, "endpoint catalog manifest verified");
            }
            Err(e) => {
                warn!(err = %e, "endpoint catalog manifest refresh failed");
            }
        }
        if state.is_draining() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(cfg.manifest_pull_seconds)).await;
    }
}

pub async fn refresh_once(
    cfg: &Config,
    state: &SidecarState,
    verifying_key: &VerifyingKey,
) -> Result<String, DomainError> {
    // Use the Helm-pinned root CA (Stage 2 §12.1) as the trust anchor
    // for HTTPS to the endpoint catalog. reqwest's default trust store
    // is rustls-tls under the `rustls-tls` feature flag (see
    // services/sidecar/Cargo.toml), which does NOT consult the OS
    // /etc/ssl/certs bundle. Without explicit `add_root_certificate`,
    // the manifest fetch fails the moment the catalog server uses any
    // private CA. (Codex Round 1 of demo bring-up caught this hole.)
    let ca_cert = reqwest::Certificate::from_pem(cfg.trust_root_ca_pem.as_bytes())
        .map_err(|e| DomainError::Internal(anyhow!(
            "parse trust_root_ca_pem for catalog client: {e}"
        )))?;
    let client = reqwest::Client::builder()
        .https_only(true)
        .add_root_certificate(ca_cert)
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| DomainError::Internal(anyhow!("build http client: {e}")))?;

    let manifest = fetch_manifest(&client, &cfg.endpoint_catalog_manifest_url).await?;

    if manifest.signing_key_id.trim().is_empty() {
        return Err(DomainError::ManifestSignatureInvalid(
            "missing signing_key_id".into(),
        ));
    }
    verify_manifest_signature(&manifest, verifying_key)?;

    if Utc::now() > manifest.valid_until {
        return Err(DomainError::ManifestStale(format!(
            "manifest valid_until {} already past",
            manifest.valid_until
        )));
    }

    let body_bytes = client
        .get(&manifest.current_catalog_url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| DomainError::Internal(anyhow!("fetch catalog body: {e}")))?
        .bytes()
        .await
        .map_err(|e| DomainError::Internal(anyhow!("read catalog body: {e}")))?;
    let actual_hash = hex::encode(Sha256::digest(&body_bytes));
    if actual_hash != manifest.current_catalog_hash {
        return Err(DomainError::ManifestSignatureInvalid(format!(
            "catalog body hash mismatch: expected={}, actual={}",
            manifest.current_catalog_hash, actual_hash
        )));
    }

    let body: Value = serde_json::from_slice(&body_bytes)
        .map_err(|e| DomainError::Internal(anyhow!("parse catalog body: {e}")))?;

    let cached = CachedCatalog {
        version_id: manifest.current_catalog_version_id.clone(),
        fetched_at: Utc::now(),
        valid_until: manifest.valid_until,
        body,
    };
    *state.inner.catalog.write() = Some(cached);
    *state.inner.last_manifest_verified_at.write() = Some(Utc::now());
    Ok(manifest.current_catalog_version_id)
}

async fn fetch_manifest(client: &reqwest::Client, url: &str) -> Result<Manifest, DomainError> {
    let resp = client
        .get(url)
        .header(reqwest::header::CACHE_CONTROL, "no-cache")
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| DomainError::Internal(anyhow!("fetch manifest {url}: {e}")))?;
    let m: Manifest = resp
        .json()
        .await
        .map_err(|e| DomainError::Internal(anyhow!("parse manifest json: {e}")))?;
    Ok(m)
}

fn verify_manifest_signature(
    manifest: &Manifest,
    verifying_key: &VerifyingKey,
) -> Result<(), DomainError> {
    // Canonical body MUST match the publisher's signing body byte-for-byte:
    //   * recursive key sort (publisher's sort_keys + serde_json::to_string)
    //   * `tenant_overrides` only included when non-empty (publisher's
    //     `#[serde(skip_serializing_if = "<[T]>::is_empty")]`).
    let mut body = serde_json::Map::new();
    body.insert("manifest_version".into(), Value::String(manifest.manifest_version.clone()));
    body.insert(
        "current_catalog_version_id".into(),
        Value::String(manifest.current_catalog_version_id.clone()),
    );
    body.insert(
        "current_catalog_url".into(),
        Value::String(manifest.current_catalog_url.clone()),
    );
    body.insert(
        "current_catalog_hash".into(),
        Value::String(manifest.current_catalog_hash.clone()),
    );
    // Canonicalize timestamps to a stable RFC 3339 form with `Z` suffix
    // and second-precision (`SecondsFormat::Secs`). Going via
    // `serde_json::to_value(DateTime<Utc>)` would be version-dependent
    // (different chrono releases have flipped between `Z` and `+00:00`
    // suffixes), which makes cross-language signing brittle. Pinning
    // the format here AND on the publisher side guarantees byte-equal
    // canonical input regardless of chrono version. (Codex Round 1 of
    // demo bring-up caught this risk.)
    body.insert(
        "issued_at".into(),
        Value::String(manifest.issued_at.to_rfc3339_opts(SecondsFormat::Secs, true)),
    );
    body.insert(
        "valid_until".into(),
        Value::String(manifest.valid_until.to_rfc3339_opts(SecondsFormat::Secs, true)),
    );
    body.insert(
        "signing_key_id".into(),
        Value::String(manifest.signing_key_id.clone()),
    );
    if !manifest.tenant_overrides.is_empty() {
        body.insert(
            "tenant_overrides".into(),
            Value::Array(manifest.tenant_overrides.clone()),
        );
    }
    let body = Value::Object(body);
    let canonical = canonicalize(body)
        .context("canonicalize manifest body")
        .map_err(|e| DomainError::Internal(anyhow!(e)))?;

    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(&manifest.signature)
        .map_err(|e| {
            DomainError::ManifestSignatureInvalid(format!("base64 sig decode: {e}"))
        })?;
    let sig: ed25519_dalek::Signature = sig_bytes.as_slice().try_into().map_err(|_| {
        DomainError::ManifestSignatureInvalid("signature length".into())
    })?;
    verifying_key
        .verify(canonical.as_bytes(), &sig)
        .map_err(|e| DomainError::ManifestSignatureInvalid(format!("verify: {e}")))?;
    Ok(())
}

fn canonicalize(value: Value) -> Result<String> {
    Ok(serde_json::to_string(&sort_keys(value))?)
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

/// Return Ok if the cached manifest is fresh enough for enforcement routes.
/// Per Sidecar §7, enforcement fails closed if
/// `last_verified_critical_version_age > critical_max_stale_seconds`.
pub fn enforce_freshness_gate(state: &SidecarState, cfg: &Config) -> Result<(), DomainError> {
    let age = state.manifest_age_seconds().ok_or_else(|| {
        DomainError::ManifestStale("no manifest verified yet".into())
    })?;
    if age > cfg.critical_max_stale_seconds as i64 {
        return Err(DomainError::ManifestStale(format!(
            "manifest age {}s > critical_max_stale {}s",
            age, cfg.critical_max_stale_seconds
        )));
    }
    Ok(())
}
