//! Manifest pointer + versioned catalog model.
//!
//! Schemas mirror the wire JSON Schemas at
//!   proto/spendguard/endpoint_catalog/v1/manifest.schema.json
//!   proto/spendguard/endpoint_catalog/v1/catalog.schema.json
//!
//! Sidecar fetches the manifest first (no-cache; signed), then pulls the
//! versioned immutable catalog object referenced by `current_catalog_url`.
//! See Sidecar §8 + Stage 2 §8.2.4 for the atomic update model.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub manifest_version: String,
    pub current_catalog_version_id: String,
    pub current_catalog_url: String,
    /// sha256 of the canonical-JSON-encoded catalog object body.
    pub current_catalog_hash: String, // hex
    pub issued_at: DateTime<Utc>,
    pub valid_until: DateTime<Utc>,
    pub signing_key_id: String,
    /// base64-encoded ed25519 signature over canonical JSON of the manifest
    /// body excluding the `signature` field.
    pub signature: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tenant_overrides: Vec<TenantOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantOverride {
    pub tenant_id: String,
    pub current_catalog_version_id: String,
    pub current_catalog_url: String,
    pub current_catalog_hash: String,
}

/// Body of the manifest used for canonical signing.
/// `signature` field excluded by definition.
#[derive(Debug, Clone, Serialize)]
pub struct ManifestSigningBody<'a> {
    pub manifest_version: &'a str,
    pub current_catalog_version_id: &'a str,
    pub current_catalog_url: &'a str,
    pub current_catalog_hash: &'a str,
    pub issued_at: DateTime<Utc>,
    pub valid_until: DateTime<Utc>,
    pub signing_key_id: &'a str,
    #[serde(skip_serializing_if = "<[TenantOverride]>::is_empty")]
    pub tenant_overrides: &'a [TenantOverride],
}

/// Catalog versioned object. Body is opaque JSON validated against
/// `proto/spendguard/endpoint_catalog/v1/catalog.schema.json`. We don't
/// strongly type the inner fields here because the catalog format evolves
/// additively; sidecars validate against the schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogBlob {
    pub catalog_version_id: String,
    #[serde(flatten)]
    pub body: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct CatalogObject {
    pub version_id: String,
    pub body: serde_json::Value,
    pub hash_hex: String,
}
