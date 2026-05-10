//! Env-driven config.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(rename = "spendguard_ttl_sweeper_database_url")]
    pub database_url: String,

    #[serde(rename = "spendguard_ttl_sweeper_ledger_url")]
    pub ledger_url: String,

    #[serde(rename = "spendguard_ttl_sweeper_tenant_id")]
    pub tenant_id: String,

    #[serde(rename = "spendguard_ttl_sweeper_tls_client_cert")]
    pub tls_client_cert: String,
    #[serde(rename = "spendguard_ttl_sweeper_tls_client_key")]
    pub tls_client_key: String,
    #[serde(rename = "spendguard_ttl_sweeper_tls_ca_pem")]
    pub tls_ca_pem: String,

    #[serde(rename = "spendguard_ttl_sweeper_fencing_scope_id")]
    pub fencing_scope_id: String,
    #[serde(rename = "spendguard_ttl_sweeper_workload_instance_id")]
    pub workload_instance_id: String,
    #[serde(rename = "spendguard_ttl_sweeper_fencing_initial_epoch")]
    pub fencing_initial_epoch: i64,

    #[serde(default = "default_poll_interval")]
    #[serde(rename = "spendguard_ttl_sweeper_poll_interval_seconds")]
    pub poll_interval_seconds: u64,

    #[serde(default = "default_batch_size")]
    #[serde(rename = "spendguard_ttl_sweeper_batch_size")]
    pub batch_size: i64,

    // Phase 5 S1 — leader election config (shared across singleton workers).
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
}

fn default_poll_interval() -> u64 {
    5
}
fn default_batch_size() -> i64 {
    10
}
fn default_lease_mode() -> String {
    "postgres".into()
}
fn default_lease_name() -> String {
    "ttl-sweeper".into()
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
        if cfg.fencing_initial_epoch <= 0 {
            anyhow::bail!("fencing_initial_epoch must be > 0");
        }
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
