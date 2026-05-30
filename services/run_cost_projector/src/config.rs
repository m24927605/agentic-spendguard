//! Env-driven configuration for the run_cost_projector gRPC service.
//!
//! Mirrors the convention used by services/output_predictor/src/config.rs
//! and services/sidecar/src/config.rs — `envy` deserializes from
//! `SPENDGUARD_RUN_COST_PROJECTOR_*` env vars.
//!
//! ## Secrets policy
//!
//! No vendor API keys in this service. `database_url` is masked via
//! `_present: bool` projection in the custom Debug impl (same as
//! output_predictor SLICE_06 R2 M13 pattern) to avoid accidental log
//! spill of embedded credentials.

use serde::Deserialize;

/// Configuration loaded from env at boot.
#[derive(Clone, Deserialize)]
pub struct Config {
    /// gRPC listen socket; `127.0.0.1:50055` (compose / dev) or
    /// `0.0.0.0:50055` (Helm chart in-cluster). The Helm chart mounts a
    /// UDS at `uds_path` when set.
    pub listen_addr: String,

    /// Optional UDS path. When set the server binds the UDS instead of TCP
    /// per the tokenizer SLICE_03 R3 N1+N2 pattern. UDS uses kernel-enforced
    /// trust; TCP requires mTLS under production profile.
    #[serde(default)]
    pub uds_path: Option<String>,

    // -- mTLS server bootstrap (mirrors output_predictor SLICE_06).
    #[serde(default)]
    pub tls_cert_pem: Option<String>,
    #[serde(default)]
    pub tls_key_pem: Option<String>,
    #[serde(default)]
    pub tls_ca_pem: Option<String>,

    /// Prometheus /metrics + /healthz + /readyz + /livez listen socket.
    /// Empty = disable metrics endpoint.
    #[serde(default = "default_metrics_addr")]
    pub metrics_addr: String,

    /// Multi-tenant region label (echoed into telemetry).
    #[serde(default)]
    pub region: String,

    /// Deploy profile (demo|production). Used by Helm chart for
    /// production-fail-fast gates.
    #[serde(default)]
    pub profile: String,

    /// Postgres URL for the read-only `run_length_distribution_cache` lookup
    /// (Signal 1) AND audit_outbox replay (cold cache rebuild per spec §7.4).
    /// Empty = skeleton mode (Signal 1 always falls to cold-start default;
    /// no audit chain recovery). Production Helm profile requires this set.
    #[serde(default)]
    pub database_url: String,

    /// RunState cache TTL in seconds (spec §7.2 default 30 minutes = 1800s).
    /// Lower = more frequent recovery; higher = more cache memory.
    #[serde(default = "default_state_cache_ttl_seconds")]
    pub state_cache_ttl_seconds: u64,

    /// RunState cache capacity (spec §0.2 endurance test target = 10K).
    /// Bounded LRU eviction at this cap.
    #[serde(default = "default_state_cache_capacity")]
    pub state_cache_capacity: usize,

    /// Audit chain replay window in minutes for cold cache recovery
    /// (spec §7.4). Default 30 minutes — bounds replay cost on cold miss.
    /// MUST be small (NOT 30 days) — replay is hot-path on cold miss.
    #[serde(default = "default_replay_window_minutes")]
    pub replay_window_minutes: u32,

    /// Cold-start default predicted run length when no historical data
    /// (spec §3.2 = 10).
    #[serde(default = "default_cold_start_run_length")]
    pub cold_start_run_length: i32,

    /// Drift detection: consecutive step threshold (spec §4.2 = 3).
    #[serde(default = "default_drift_consecutive_threshold")]
    pub drift_consecutive_threshold: u32,

    /// Drift detection: per-step cost ratio threshold (2σ rule per spec §4.2).
    /// Expressed as multiplicative factor — drift if
    /// `|now/prior - 1| > drift_ratio_threshold`. Default 0.5 = 50% jump.
    #[serde(default = "default_drift_ratio_threshold")]
    pub drift_ratio_threshold: f64,
}

impl std::fmt::Debug for Config {
    /// Mirror output_predictor's R2 M13 secret-masking pattern.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("listen_addr", &self.listen_addr)
            .field("uds_path", &self.uds_path)
            .field("tls_cert_pem", &self.tls_cert_pem)
            .field("tls_key_pem", &self.tls_key_pem)
            .field("tls_ca_pem", &self.tls_ca_pem)
            .field("metrics_addr", &self.metrics_addr)
            .field("region", &self.region)
            .field("profile", &self.profile)
            .field("database_url_present", &!self.database_url.is_empty())
            .field("state_cache_ttl_seconds", &self.state_cache_ttl_seconds)
            .field("state_cache_capacity", &self.state_cache_capacity)
            .field("replay_window_minutes", &self.replay_window_minutes)
            .field("cold_start_run_length", &self.cold_start_run_length)
            .field("drift_consecutive_threshold", &self.drift_consecutive_threshold)
            .field("drift_ratio_threshold", &self.drift_ratio_threshold)
            .finish()
    }
}

fn default_metrics_addr() -> String {
    // Port 9102 — run_cost_projector slot in the demo compose port table:
    //   ledger=9092, canonical-ingest=9091, sidecar=9093, control-plane=9094,
    //   dashboard=9095, outbox=9096, ttl-sweeper=9097, webhook-receiver=9098,
    //   tokenizer=9099, output-predictor=9100, stats-aggregator=9101,
    //   run-cost-projector=9102.
    "0.0.0.0:9102".to_string()
}

fn default_state_cache_ttl_seconds() -> u64 {
    // Spec §7.2 — 30 minutes default.
    1800
}

fn default_state_cache_capacity() -> usize {
    // Spec §0.2 endurance test = 10K concurrent runs without memory leak.
    10_000
}

fn default_replay_window_minutes() -> u32 {
    // Spec §7.4 — bounded replay window (30 min). NOT 30 days.
    30
}

fn default_cold_start_run_length() -> i32 {
    // Spec §3.2 — cold-start default 10 steps.
    10
}

fn default_drift_consecutive_threshold() -> u32 {
    // Spec §4.2 — 3 consecutive steps above threshold trigger drift.
    3
}

fn default_drift_ratio_threshold() -> f64 {
    // Spec §4.2 — 2σ rule; SLICE_09 uses 0.5 (50% jump) as a conservative
    // multiplicative threshold. Per-call cost variance in practice is high
    // enough that strict 2σ on per-step samples is noisy; the layering
    // logic + 3-consecutive-step requirement (§4.2) suppresses false
    // positives. Tunable per tenant in future via control-plane API.
    0.5
}

impl Config {
    pub fn from_env() -> Result<Self, envy::Error> {
        envy::prefixed("SPENDGUARD_RUN_COST_PROJECTOR_").from_env()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_load_with_minimum_env() {
        let cfg = envy::prefixed("TEST_CFG_")
            .from_iter::<_, Config>(vec![(
                "TEST_CFG_LISTEN_ADDR".to_string(),
                "127.0.0.1:50055".to_string(),
            )])
            .expect("config loads");
        assert_eq!(cfg.listen_addr, "127.0.0.1:50055");
        assert!(cfg.uds_path.is_none());
        assert_eq!(cfg.metrics_addr, "0.0.0.0:9102");
        assert_eq!(cfg.state_cache_ttl_seconds, 1800);
        assert_eq!(cfg.state_cache_capacity, 10_000);
        assert_eq!(cfg.replay_window_minutes, 30);
        assert_eq!(cfg.cold_start_run_length, 10);
        assert_eq!(cfg.drift_consecutive_threshold, 3);
        assert!((cfg.drift_ratio_threshold - 0.5).abs() < 1e-9);
    }

    #[test]
    fn debug_format_masks_database_url() {
        let cfg = envy::prefixed("TEST_CFG_")
            .from_iter::<_, Config>(vec![
                (
                    "TEST_CFG_LISTEN_ADDR".to_string(),
                    "127.0.0.1:50055".to_string(),
                ),
                (
                    "TEST_CFG_DATABASE_URL".to_string(),
                    "postgres://user:secret-pass-DO-NOT-LEAK@host/db".to_string(),
                ),
            ])
            .expect("config loads");
        let dbg = format!("{cfg:?}");
        assert!(
            !dbg.contains("secret-pass-DO-NOT-LEAK"),
            "database_url password leaked into Debug: {dbg}"
        );
        assert!(dbg.contains("database_url_present: true"));
    }

    #[test]
    fn debug_format_reports_missing_database_url() {
        let cfg = envy::prefixed("TEST_CFG_")
            .from_iter::<_, Config>(vec![(
                "TEST_CFG_LISTEN_ADDR".to_string(),
                "127.0.0.1:50055".to_string(),
            )])
            .expect("config loads");
        let dbg = format!("{cfg:?}");
        assert!(dbg.contains("database_url_present: false"));
    }
}
