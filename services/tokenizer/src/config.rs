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
    ///
    /// Round-2 fix B3.1: this field is now actually read by main.rs;
    /// previously declared but ignored.
    #[serde(default)]
    pub uds_path: Option<String>,

    // -- mTLS server bootstrap (round-2 fix B3.2; mirrors
    //    services/ledger/src/config.rs §12.1 pattern) ----------------
    /// Path to the tokenizer's workload TLS cert PEM. When set together
    /// with `tls_key_pem` and `tls_ca_pem`, the gRPC server starts in
    /// mTLS mode and rejects clients whose cert chain doesn't validate
    /// against the configured CA. Leave unset for plaintext (demo /
    /// compose dev only). Under chart.profile=production the Helm
    /// template fails fast if neither UDS nor mTLS is configured.
    #[serde(default)]
    pub tls_cert_pem: Option<String>,

    /// Path to the tokenizer's workload TLS private key PEM.
    #[serde(default)]
    pub tls_key_pem: Option<String>,

    /// Path to the trust root CA PEM the tokenizer uses to validate
    /// mTLS client certs (sidecar / shadow-worker workload certs).
    #[serde(default)]
    pub tls_ca_pem: Option<String>,

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

    // ── SLICE_05: Tier 1 shadow drift detection ────────────────────
    /// Master kill-switch for the shadow worker. When `false` the
    /// service still tokenises (Tier 2 hot path), but no shadow events
    /// are produced. Documented in §11 rollback plan.
    #[serde(default = "default_shadow_enabled")]
    pub shadow_enabled: bool,

    /// Default sample rate (per spec §4.1; default 1% = 0.01). Override
    /// per (tenant, model) via the control plane API (Phase F).
    #[serde(default = "default_shadow_sample_rate")]
    pub shadow_default_sample_rate: f64,

    /// Anthropic API key for `POST /v1/messages/count_tokens`. Empty =
    /// Anthropic shadow path disabled (worker still runs for Gemini).
    /// Read from a file path when prefixed with `file://` so K8s
    /// Secret mounts work cleanly (see Helm tokenizer.yaml).
    #[serde(default)]
    pub anthropic_api_key: String,

    /// Gemini API key for `POST /v1/models/{model}:countTokens`.
    /// Empty = Gemini shadow path disabled.
    #[serde(default)]
    pub gemini_api_key: String,

    /// Postgres URL for the shadow worker's `tokenizer_t1_samples`
    /// persistence. Empty = use in-memory persister (demo only —
    /// production Helm profile requires this set).
    #[serde(default)]
    pub database_url: String,

    /// canonical_ingest gRPC URL for the signed `tokenizer_drift_alert`
    /// CloudEvent sink. Empty = use in-memory sink (demo only —
    /// production Helm profile requires this set).
    #[serde(default)]
    pub canonical_ingest_url: String,

    /// Producer source URI written into the emitted CloudEvent. Defaults
    /// to `spendguard://tokenizer-service/<region>`. Surfaced for
    /// per-instance disambiguation in multi-region deploys.
    #[serde(default)]
    pub event_source_override: String,
}

fn default_metrics_addr() -> String {
    // Port 9099 — see deploy/demo/compose.yaml service port table:
    //   ledger=9092, canonical-ingest=9091, sidecar=9093,
    //   control-plane=9094, dashboard=9095, outbox=9096,
    //   ttl-sweeper=9097, webhook-receiver=9098, tokenizer=9099.
    //
    // Round-2 minor m4 (panel finding): port 9099 collides with the
    // usage_poller default metrics port in some demo topologies. The
    // collision is harmless when only one component runs per host
    // (compose / chart), but the cleaner fix is to migrate usage_poller
    // to a free port. Tracked as a separate GH issue (R2 minor m4)
    // since it spans a different service crate.
    "0.0.0.0:9099".to_string()
}

fn default_tier3_alert_threshold() -> f32 {
    0.001
}

fn default_shadow_enabled() -> bool {
    // SLICE_05: enabled by default — operators disable via env var per
    // §11 rollback plan. Helm production profile honours the value
    // explicitly so it lands in the chart's NOTES.txt.
    true
}

fn default_shadow_sample_rate() -> f64 {
    // Spec §4.1 — 1% default.
    0.01
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
