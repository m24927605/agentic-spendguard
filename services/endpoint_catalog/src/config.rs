use serde::Deserialize;

/// Storage configuration shared by both server and publisher.
#[derive(Debug, Clone, Deserialize)]
pub struct StorageConfig {
    #[serde(default = "default_storage_backend")]
    pub storage_backend: String,
    pub filesystem_root: Option<String>,
    pub s3_bucket: Option<String>,
    #[serde(default = "default_s3_prefix")]
    pub s3_prefix: String,
    pub region: String,
}

/// HTTP server config. Does NOT include any signing material — server
/// only serves pre-signed manifests + immutable catalog objects.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,

    #[serde(flatten)]
    pub storage: StorageConfig,

    /// SSE keepalive interval (seconds). Sidecars time out stuck connections
    /// after this delay.
    #[serde(default = "default_sse_keepalive_seconds")]
    pub sse_keepalive_seconds: u64,

    /// Bearer token for the internal `/v1/internal/notify-catalog-change`
    /// endpoint. When None or empty, the endpoint returns 403.
    pub internal_notify_token: Option<String>,
}

/// Publisher CLI config. Carries signing material.
#[derive(Debug, Clone, Deserialize)]
pub struct PublisherConfig {
    #[serde(flatten)]
    pub storage: StorageConfig,

    /// Manifest signature validity (`valid_until - issued_at`). Defaults
    /// to 600s; operators MUST set this longer than the sidecar's manifest
    /// pull cadence (default 300s) so a sidecar that pulled near the
    /// signing instant does NOT immediately fail-closed at expiry. The
    /// 5-min critical_revocation_max_stale gate (Sidecar §8) is measured
    /// from the sidecar's last successful pull, NOT from issued_at.
    #[serde(default = "default_manifest_validity_seconds")]
    pub manifest_validity_seconds: u64,

    /// Path to ed25519 signing key (PEM PKCS#8). Required.
    pub signing_key_pem_path: String,

    /// Public key id for the manifest's `signing_key_id` field. Sidecars
    /// validate signatures against the Helm-pinned root which signs this id.
    pub signing_key_id: String,

    /// Public base URL the catalog is served from (e.g.,
    /// `https://catalog.us-west-2.spendguard.ai`). Used to emit absolute
    /// `current_catalog_url` in the manifest, per
    /// `proto/spendguard/endpoint_catalog/v1/manifest.schema.json`.
    pub public_base_url: String,

    /// Optional notify BASE URL — the base of a running endpoint catalog
    /// HTTP server (e.g., `https://catalog.us-west-2.spendguard.ai`). The
    /// publisher appends `/v1/internal/notify-catalog-change` itself; do
    /// NOT include a path here. When unset, publisher skips the SSE hint
    /// (sidecars still fall back to manifest pull).
    pub notify_base_url: Option<String>,
    pub notify_token: Option<String>,
}

fn default_bind_addr() -> String {
    "0.0.0.0:8443".to_string()
}
fn default_storage_backend() -> String {
    "filesystem".to_string()
}
fn default_s3_prefix() -> String {
    "endpoint-catalog/".to_string()
}
fn default_manifest_validity_seconds() -> u64 {
    600
}
fn default_sse_keepalive_seconds() -> u64 {
    20
}

impl ServerConfig {
    pub fn from_env() -> Result<Self, envy::Error> {
        envy::prefixed("SPENDGUARD_ENDPOINT_CATALOG_").from_env::<ServerConfig>()
    }
}

impl PublisherConfig {
    pub fn from_env() -> Result<Self, envy::Error> {
        envy::prefixed("SPENDGUARD_ENDPOINT_CATALOG_").from_env::<PublisherConfig>()
    }
}
