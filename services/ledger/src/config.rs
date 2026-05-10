//! Service configuration loaded from environment.
//!
//! Phase 1 POC uses env vars; Phase 2 may switch to file-based config with
//! signed manifest distribution. All defaults are POC-safe.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    /// gRPC bind address; e.g., "0.0.0.0:50051".
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,

    /// Postgres connection string; libpq format.
    pub database_url: String,

    /// Pool size; tune for sync replica latency.
    #[serde(default = "default_db_max_connections")]
    pub db_max_connections: u32,

    /// Tenant id this ledger instance serves (POC: per-tenant Postgres DB).
    pub tenant_id: String,

    /// Region (us-west-2 for first customer).
    pub region: String,

    /// Workload identity for fencing scopes (per Sidecar §9 + Stage 2 §4.4).
    pub workload_instance_id: String,

    /// Sidecar producer signing key id (for verifying audit_event signatures).
    pub producer_signing_key_id: String,

    // -- mTLS server bootstrap (Stage 2 §12.1) -----------------------------
    /// Path to the ledger's workload TLS cert PEM. When set together with
    /// `tls_key_pem` and `tls_ca_pem`, the gRPC server starts in mTLS
    /// mode and rejects clients whose cert chain doesn't validate against
    /// the configured CA. Leave unset for plaintext (POC dev only).
    #[serde(default)]
    pub tls_cert_pem: Option<String>,

    /// Path to the ledger's workload TLS private key PEM.
    #[serde(default)]
    pub tls_key_pem: Option<String>,

    /// Path to the trust root CA PEM the ledger uses to validate mTLS
    /// client certs (sidecar workload certs).
    #[serde(default)]
    pub tls_ca_pem: Option<String>,

    /// Round-2 #11: Prometheus /metrics endpoint bind addr. Defaults to
    /// `0.0.0.0:9092` (ledger gets 9092; sidecar 9093, etc. — see
    /// followup #11 port table). Set to empty string to disable.
    #[serde(default = "default_metrics_addr")]
    pub metrics_addr: String,
}

fn default_bind_addr() -> String {
    "0.0.0.0:50051".to_string()
}

fn default_db_max_connections() -> u32 {
    32
}

fn default_metrics_addr() -> String {
    "0.0.0.0:9092".to_string()
}

impl Config {
    pub fn from_env() -> Result<Self, envy::Error> {
        envy::prefixed("SPENDGUARD_LEDGER_").from_env::<Config>()
    }
}
