//! Env-driven configuration for the webhook receiver.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// HTTPS listen address (e.g. 0.0.0.0:8443).
    #[serde(rename = "spendguard_webhook_receiver_bind_addr")]
    pub bind_addr: String,

    /// Plain-HTTP healthz listen address (e.g. 0.0.0.0:8080).
    #[serde(rename = "spendguard_webhook_receiver_health_addr")]
    pub health_addr: String,

    /// Postgres URL (typically the same DB as ledger).
    #[serde(rename = "spendguard_webhook_receiver_database_url")]
    pub database_url: String,

    /// Ledger gRPC URL (https://ledger:50051).
    #[serde(rename = "spendguard_webhook_receiver_ledger_url")]
    pub ledger_url: String,

    #[serde(rename = "spendguard_webhook_receiver_tenant_id")]
    pub tenant_id: String,

    /// HTTPS server cert (also used as mTLS client cert to ledger).
    #[serde(rename = "spendguard_webhook_receiver_tls_server_cert")]
    pub tls_server_cert: String,

    #[serde(rename = "spendguard_webhook_receiver_tls_server_key")]
    pub tls_server_key: String,

    #[serde(rename = "spendguard_webhook_receiver_tls_client_cert")]
    pub tls_client_cert: String,

    #[serde(rename = "spendguard_webhook_receiver_tls_client_key")]
    pub tls_client_key: String,

    #[serde(rename = "spendguard_webhook_receiver_tls_ca_pem")]
    pub tls_ca_pem: String,

    #[serde(rename = "spendguard_webhook_receiver_fencing_scope_id")]
    pub fencing_scope_id: String,

    #[serde(rename = "spendguard_webhook_receiver_workload_instance_id")]
    pub workload_instance_id: String,

    #[serde(rename = "spendguard_webhook_receiver_fencing_initial_epoch")]
    pub fencing_initial_epoch: i64,

    /// HMAC shared secret for the mock-llm provider (POC).
    #[serde(rename = "spendguard_webhook_secret_mock_llm")]
    pub mock_llm_secret: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let cfg: Self = envy::from_env()?;
        if cfg.fencing_initial_epoch <= 0 {
            anyhow::bail!("fencing_initial_epoch must be > 0");
        }
        Ok(cfg)
    }
}
