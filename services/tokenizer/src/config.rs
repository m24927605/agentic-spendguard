//! Env-driven configuration for the tokenizer gRPC service.
//!
//! Mirrors the convention used by services/sidecar/src/config.rs and
//! services/ledger/src/config.rs — `envy` deserializes from
//! `SPENDGUARD_TOKENIZER_*` env vars.

use serde::Deserialize;

/// Configuration loaded from env at boot.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// gRPC listen socket; either `127.0.0.1:50053` (compose / dev)
    /// or `0.0.0.0:50053` (Helm chart in-cluster). The Helm chart
    /// also mounts a UDS at `uds_path` when present.
    pub listen_addr: String,

    /// Optional UDS path. When set, the server binds the UDS in
    /// parallel to `listen_addr` so on-node sidecars can reach the
    /// tokenizer service without an L4 hop (spec §10.1 latency
    /// budget).
    #[serde(default)]
    pub uds_path: Option<String>,

    /// Prometheus /metrics listen socket. Empty string = disable
    /// metrics endpoint (test envs).
    #[serde(default = "default_metrics_addr")]
    pub metrics_addr: String,

    /// Threshold for emitting `tokenizer_tier3_hit_alert` events.
    /// Default 0.001 (0.1%) per spec §5.3 health invariant.
    /// SLICE_03: the value is plumbed but the alert wire-up itself
    /// lives in SLICE-extra (control plane integration).
    #[serde(default = "default_tier3_alert_threshold")]
    pub tier3_alert_threshold: f32,

    /// Multi-tenant region label (echoed into telemetry; no
    /// tenant-bound state per spec §9 review question 6).
    #[serde(default)]
    pub region: String,
}

fn default_metrics_addr() -> String {
    // Port 9099 — see deploy/demo/compose.yaml service port table:
    //   ledger=9092, canonical-ingest=9091, sidecar=9093,
    //   control-plane=9094, dashboard=9095, outbox=9096,
    //   ttl-sweeper=9097, webhook-receiver=9098, tokenizer=9099.
    "0.0.0.0:9099".to_string()
}

fn default_tier3_alert_threshold() -> f32 {
    0.001
}

impl Config {
    /// Load from `SPENDGUARD_TOKENIZER_*` env vars.
    pub fn from_env() -> Result<Self, envy::Error> {
        envy::prefixed("SPENDGUARD_TOKENIZER_").from_env()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_load_with_minimum_env() {
        // Force load with only the required field.
        let cfg = envy::prefixed("TEST_CFG_").from_iter::<_, Config>(vec![(
            "TEST_CFG_LISTEN_ADDR".to_string(),
            "127.0.0.1:50053".to_string(),
        )])
        .expect("config loads");
        assert_eq!(cfg.listen_addr, "127.0.0.1:50053");
        assert!(cfg.uds_path.is_none());
        assert_eq!(cfg.metrics_addr, "0.0.0.0:9099");
        assert!((cfg.tier3_alert_threshold - 0.001).abs() < 1e-6);
    }

    #[test]
    fn tier3_threshold_overridable() {
        let cfg = envy::prefixed("TEST_CFG_").from_iter::<_, Config>(vec![
            (
                "TEST_CFG_LISTEN_ADDR".to_string(),
                "127.0.0.1:50053".to_string(),
            ),
            (
                "TEST_CFG_TIER3_ALERT_THRESHOLD".to_string(),
                "0.005".to_string(),
            ),
        ])
        .expect("config loads");
        assert!((cfg.tier3_alert_threshold - 0.005).abs() < 1e-6);
    }
}
