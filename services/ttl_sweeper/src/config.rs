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
}

fn default_poll_interval() -> u64 {
    5
}
fn default_batch_size() -> i64 {
    10
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
        Ok(cfg)
    }
}
