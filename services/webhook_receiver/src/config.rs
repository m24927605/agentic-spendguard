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

    /// Round-2 #11: Prometheus /metrics endpoint bind addr. Defaults
    /// to `0.0.0.0:9098` per the round-2 port table. Empty disables.
    #[serde(default = "default_metrics_addr")]
    #[serde(rename = "spendguard_webhook_receiver_metrics_addr")]
    pub metrics_addr: String,
}

fn default_metrics_addr() -> String {
    "0.0.0.0:9098".to_string()
}

/// Minimum acceptable length (bytes) for any per-provider HMAC secret. An
/// empty or trivially short secret keys `HmacSha256::new_from_slice` with a
/// near-zero-entropy key, making webhook signatures forgeable. We fail closed
/// at startup rather than serve a forgeable endpoint. 16 bytes (128 bits) is
/// the floor for an HMAC-SHA256 key.
const MIN_HMAC_SECRET_LEN: usize = 16;

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let cfg: Self = envy::from_env()?;
        if cfg.fencing_initial_epoch <= 0 {
            anyhow::bail!("fencing_initial_epoch must be > 0");
        }
        // Reject empty/short per-provider HMAC secrets so a misconfiguration
        // crashes startup instead of serving a forgeable webhook endpoint.
        cfg.validate_provider_secret("mock_llm", &cfg.mock_llm_secret)?;
        Ok(cfg)
    }

    /// Fail closed if a provider HMAC secret is missing or below the minimum
    /// HMAC-SHA256 key length. Add a call here for every future per-provider
    /// secret field.
    fn validate_provider_secret(&self, name: &str, secret: &str) -> anyhow::Result<()> {
        if secret.len() < MIN_HMAC_SECRET_LEN {
            anyhow::bail!(
                "{} HMAC secret must be at least {} bytes (got {}); refusing to \
                 start with an empty or trivially short, forgeable secret",
                name,
                MIN_HMAC_SECRET_LEN,
                secret.len(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_secret(len: usize) -> String {
        "a".repeat(len)
    }

    fn dummy_config(secret: &str) -> Config {
        Config {
            bind_addr: String::new(),
            health_addr: String::new(),
            database_url: String::new(),
            ledger_url: String::new(),
            tenant_id: String::new(),
            tls_server_cert: String::new(),
            tls_server_key: String::new(),
            tls_client_cert: String::new(),
            tls_client_key: String::new(),
            tls_ca_pem: String::new(),
            fencing_scope_id: String::new(),
            workload_instance_id: String::new(),
            fencing_initial_epoch: 1,
            mock_llm_secret: secret.to_string(),
            metrics_addr: default_metrics_addr(),
        }
    }

    #[test]
    fn empty_secret_rejected() {
        let cfg = dummy_config("");
        assert!(cfg
            .validate_provider_secret("mock_llm", &cfg.mock_llm_secret)
            .is_err());
    }

    #[test]
    fn short_secret_rejected() {
        let cfg = dummy_config(&base_secret(MIN_HMAC_SECRET_LEN - 1));
        assert!(cfg
            .validate_provider_secret("mock_llm", &cfg.mock_llm_secret)
            .is_err());
    }

    #[test]
    fn min_length_secret_accepted() {
        let cfg = dummy_config(&base_secret(MIN_HMAC_SECRET_LEN));
        assert!(cfg
            .validate_provider_secret("mock_llm", &cfg.mock_llm_secret)
            .is_ok());
    }
}
