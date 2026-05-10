use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(rename = "spendguard_outbox_forwarder_database_url")]
    pub database_url: String,

    #[serde(rename = "spendguard_outbox_forwarder_canonical_ingest_url")]
    pub canonical_ingest_url: String,

    #[serde(rename = "spendguard_outbox_forwarder_tls_client_cert")]
    pub tls_client_cert: String,
    #[serde(rename = "spendguard_outbox_forwarder_tls_client_key")]
    pub tls_client_key: String,
    #[serde(rename = "spendguard_outbox_forwarder_tls_ca_pem")]
    pub tls_ca_pem: String,

    #[serde(rename = "spendguard_outbox_forwarder_workload_instance_id")]
    pub workload_instance_id: String,

    #[serde(rename = "spendguard_outbox_forwarder_schema_bundle_id")]
    pub schema_bundle_id: String,

    #[serde(rename = "spendguard_outbox_forwarder_schema_bundle_hash_hex")]
    pub schema_bundle_hash_hex: String,

    #[serde(default = "default_poll_interval")]
    #[serde(rename = "spendguard_outbox_forwarder_poll_interval_seconds")]
    pub poll_interval_seconds: u64,

    #[serde(default = "default_batch_size")]
    #[serde(rename = "spendguard_outbox_forwarder_batch_size")]
    pub batch_size: i64,

    // Phase 5 S1 — leader election config.
    #[serde(default = "default_lease_mode")]
    #[serde(rename = "spendguard_leader_election_mode")]
    pub leader_election_mode: String,
    #[serde(default = "default_lease_name")]
    #[serde(rename = "spendguard_leader_lease_name")]
    pub leader_lease_name: String,
    #[serde(default = "default_lease_region")]
    #[serde(rename = "spendguard_leader_region")]
    pub leader_region: String,
    #[serde(default = "default_lease_ttl_ms")]
    #[serde(rename = "spendguard_leader_lease_ttl_ms")]
    pub leader_lease_ttl_ms: u64,
    #[serde(default = "default_lease_renew_ms")]
    #[serde(rename = "spendguard_leader_renew_interval_ms")]
    pub leader_renew_interval_ms: u64,
    #[serde(default = "default_lease_retry_ms")]
    #[serde(rename = "spendguard_leader_retry_interval_ms")]
    pub leader_retry_interval_ms: u64,

    /// Round-2 #11: Prometheus /metrics endpoint bind addr. Defaults
    /// to `0.0.0.0:9096` per the round-2 port table (ledger=9092,
    /// sidecar=9093, control_plane=9094, dashboard=9095,
    /// outbox_forwarder=9096). Empty disables.
    #[serde(default = "default_metrics_addr")]
    #[serde(rename = "spendguard_outbox_forwarder_metrics_addr")]
    pub metrics_addr: String,
}

fn default_metrics_addr() -> String {
    "0.0.0.0:9096".to_string()
}

fn default_poll_interval() -> u64 {
    2
}
fn default_batch_size() -> i64 {
    50
}
fn default_lease_mode() -> String {
    // Default to Postgres so multi-pod is safe by default once enabled.
    // Disabled mode requires explicit opt-in (single-pod escape hatch).
    "postgres".into()
}
fn default_lease_name() -> String {
    "outbox-forwarder".into()
}
fn default_lease_region() -> String {
    "demo".into()
}
fn default_lease_ttl_ms() -> u64 {
    15_000
}
fn default_lease_renew_ms() -> u64 {
    5_000
}
fn default_lease_retry_ms() -> u64 {
    1_000
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let cfg: Self = envy::from_env()?;
        if cfg.batch_size <= 0 {
            anyhow::bail!("batch_size must be > 0");
        }
        let valid_modes = ["postgres", "k8s", "disabled"];
        if !valid_modes.contains(&cfg.leader_election_mode.as_str()) {
            anyhow::bail!(
                "SPENDGUARD_LEADER_ELECTION_MODE must be one of {:?}, got {}",
                valid_modes,
                cfg.leader_election_mode
            );
        }
        if cfg.leader_renew_interval_ms >= cfg.leader_lease_ttl_ms {
            anyhow::bail!(
                "leader_renew_interval_ms ({}) must be < leader_lease_ttl_ms ({})",
                cfg.leader_renew_interval_ms,
                cfg.leader_lease_ttl_ms
            );
        }
        Ok(cfg)
    }
}
