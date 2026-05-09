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
}

fn default_poll_interval() -> u64 {
    2
}
fn default_batch_size() -> i64 {
    50
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let cfg: Self = envy::from_env()?;
        if cfg.batch_size <= 0 {
            anyhow::bail!("batch_size must be > 0");
        }
        Ok(cfg)
    }
}
