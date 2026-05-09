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
    /// signatures (per Trace §13). S8 makes this production-ready:
    /// strict mode rejects events with unknown_key / invalid_signature
    /// (typed gRPC error) and quarantines pre_s6 / disabled rows.
    /// Non-strict mode accepts everything but still records verification
    /// outcomes via metrics for forensics.
    #[serde(default)]
    pub strict_signatures: bool,

    /// Phase 5 GA hardening S8: filesystem path holding `<key_id>.pem`
    /// files for the trust store. Required when strict_signatures=true.
    /// In demo / Helm local mode this is the same Secret that producers
    /// mount their private keys from (Ed25519 PKCS8 PEM contains both
    /// the private and the embedded public key; verifier extracts the
    /// public).
    #[serde(default)]
    pub trust_store_dir: Option<String>,

    /// S8: where the Prometheus metrics endpoint binds. Defaults to
    /// `0.0.0.0:9091` so it doesn't collide with the gRPC server on
    /// `:50061`. Set to empty string to disable the metrics server.
    #[serde(default = "default_metrics_addr")]
    pub metrics_addr: String,

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
fn default_metrics_addr() -> String {
    "0.0.0.0:9091".to_string()
}

impl Config {
    pub fn from_env() -> Result<Self, envy::Error> {
        envy::prefixed("SPENDGUARD_CANONICAL_INGEST_").from_env::<Config>()
    }
}
