use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,

    pub database_url: String,

    #[serde(default = "default_db_max_connections")]
    pub db_max_connections: u32,

    pub region: String,

    /// Logical ingest shard id. Multiple instances can share a shard or
    /// own their own; (region_id, ingest_shard_id) is the canonical
    /// ordering scope per Trace §10.5.
    pub ingest_shard_id: String,

    /// Quarantine reaper deadline. audit.outcome events without their
    /// matching audit.decision get marked ORPHAN_OUTCOME after this delay.
    #[serde(default = "default_orphan_after_seconds")]
    pub orphan_after_seconds: u64,

    /// Backpressure threshold (per Trace §10.1 v2.1 patch).
    /// Enforcement-route appends past this depth fail_closed; observability
    /// route appends past this depth buffer_then_retry.
    #[serde(default = "default_backpressure_threshold")]
    pub backpressure_threshold: u64,

    /// When true, ingest requires + verifies per-event ed25519 producer
    /// signatures (per Trace §13). POC default = false: the keystore +
    /// producer-key sync from sidecar handshake / Bundle Registry is
    /// deferred to vertical slice expansion. In POC mode events are admitted
    /// without signature verification; enabling strict in production
    /// requires the keystore to be populated.
    #[serde(default)]
    pub strict_signatures: bool,

    // -- mTLS server bootstrap (Stage 2 §12.1) -----------------------------
    /// Path to the canonical-ingest workload TLS cert PEM. When set
    /// together with `tls_key_pem` and `tls_ca_pem`, the gRPC server
    /// starts in mTLS mode. Leave unset for plaintext (POC dev only).
    #[serde(default)]
    pub tls_cert_pem: Option<String>,

    #[serde(default)]
    pub tls_key_pem: Option<String>,

    #[serde(default)]
    pub tls_ca_pem: Option<String>,
}

fn default_bind_addr() -> String {
    "0.0.0.0:50061".to_string()
}
fn default_db_max_connections() -> u32 {
    32
}
fn default_orphan_after_seconds() -> u64 {
    30
}
fn default_backpressure_threshold() -> u64 {
    10_000
}

impl Config {
    pub fn from_env() -> Result<Self, envy::Error> {
        envy::prefixed("SPENDGUARD_CANONICAL_INGEST_").from_env::<Config>()
    }
}
