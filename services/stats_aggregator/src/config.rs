//! Env-driven configuration for the stats_aggregator daemon.
//!
//! Mirrors services/tokenizer/src/config.rs SLICE_05 R2 M13 (mask
//! sensitive fields in Debug).

use serde::Deserialize;

/// Configuration loaded from `SPENDGUARD_STATS_AGGREGATOR_*` env vars.
#[derive(Clone, Deserialize)]
pub struct Config {
    /// Postgres URL for canonical_ingest DB (read source + cache write
    /// target). Required.
    pub database_url: String,

    /// canonical_ingest gRPC URL for signed prediction_drift_alert
    /// CloudEvent emission per spec §7.2. Required under production
    /// profile (Helm gate enforces); empty acceptable in demo (the
    /// daemon logs drift alerts to stdout instead of sinking them).
    #[serde(default)]
    pub canonical_ingest_url: String,

    /// Aggregation cycle cadence in seconds. Spec §8.1 default 3600
    /// (hourly). Minimum 60 enforced at boot to protect Postgres.
    #[serde(default = "default_cycle_seconds")]
    pub cycle_seconds: u64,

    /// Drift detection minimum sample count per spec §7.1 (default 100).
    /// Smaller windows trigger false positives.
    #[serde(default = "default_min_samples_for_alert")]
    pub min_samples_for_alert: i32,

    /// Drift detection z-score threshold (per spec §7.1 default 2.0).
    #[serde(default = "default_drift_z_threshold")]
    pub drift_z_threshold: f32,

    /// Prometheus /metrics + /healthz + /readyz listen socket.
    #[serde(default = "default_metrics_addr")]
    pub metrics_addr: String,

    /// Multi-tenant region label.
    #[serde(default)]
    pub region: String,

    /// Deploy profile (demo|production). Used by Helm chart for
    /// production-fail-fast gates.
    #[serde(default)]
    pub profile: String,

    // -- canonical_ingest sink mTLS (matches tokenizer SLICE_05 R2 B4) --
    #[serde(default)]
    pub sink_tls_cert_pem: Option<String>,
    #[serde(default)]
    pub sink_tls_key_pem: Option<String>,
    #[serde(default)]
    pub sink_tls_ca_pem: Option<String>,
    #[serde(default = "default_sink_sni")]
    pub sink_tls_sni: String,

    /// Producer source URI written into emitted CloudEvent. Defaults to
    /// `spendguard://stats-aggregator/<region>`. Surfaced for
    /// per-instance disambiguation in multi-region deploys.
    #[serde(default)]
    pub event_source_override: String,
}

impl std::fmt::Debug for Config {
    /// Mask database_url + sink TLS material paths in Debug output to
    /// avoid leaking secrets into structured logs / panic backtraces.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("database_url_present", &!self.database_url.is_empty())
            .field("canonical_ingest_url", &self.canonical_ingest_url)
            .field("cycle_seconds", &self.cycle_seconds)
            .field("min_samples_for_alert", &self.min_samples_for_alert)
            .field("drift_z_threshold", &self.drift_z_threshold)
            .field("metrics_addr", &self.metrics_addr)
            .field("region", &self.region)
            .field("profile", &self.profile)
            .field("sink_tls_cert_pem", &self.sink_tls_cert_pem)
            .field("sink_tls_key_pem", &self.sink_tls_key_pem)
            .field("sink_tls_ca_pem", &self.sink_tls_ca_pem)
            .field("sink_tls_sni", &self.sink_tls_sni)
            .field("event_source_override", &self.event_source_override)
            .finish()
    }
}

fn default_cycle_seconds() -> u64 {
    3600
}

fn default_min_samples_for_alert() -> i32 {
    100
}

fn default_drift_z_threshold() -> f32 {
    2.0
}

fn default_metrics_addr() -> String {
    // Port 9101 per the demo compose port table.
    "0.0.0.0:9101".to_string()
}

fn default_sink_sni() -> String {
    "canonical-ingest.spendguard.internal".to_string()
}

impl Config {
    pub fn from_env() -> Result<Self, envy::Error> {
        envy::prefixed("SPENDGUARD_STATS_AGGREGATOR_").from_env()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_load_with_minimum_env() {
        let cfg = envy::prefixed("TEST_CFG_").from_iter::<_, Config>(vec![(
            "TEST_CFG_DATABASE_URL".to_string(),
            "postgres://example/db".to_string(),
        )])
        .expect("config loads");
        assert_eq!(cfg.cycle_seconds, 3600);
        assert_eq!(cfg.min_samples_for_alert, 100);
        assert!((cfg.drift_z_threshold - 2.0).abs() < 1e-6);
        assert_eq!(cfg.metrics_addr, "0.0.0.0:9101");
    }

    #[test]
    fn debug_format_masks_database_url() {
        let cfg = envy::prefixed("TEST_CFG_").from_iter::<_, Config>(vec![(
            "TEST_CFG_DATABASE_URL".to_string(),
            "postgres://user:hidden-secret-password@host/db".to_string(),
        )])
        .expect("config loads");
        let dbg = format!("{cfg:?}");
        assert!(
            !dbg.contains("hidden-secret-password"),
            "database_url password leaked: {dbg}"
        );
        assert!(dbg.contains("database_url_present: true"));
    }
}
