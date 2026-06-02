//! Env-driven configuration for the output_predictor gRPC service.
//!
//! Mirrors the convention used by services/tokenizer/src/config.rs and
//! services/sidecar/src/config.rs — `envy` deserializes from
//! `SPENDGUARD_OUTPUT_PREDICTOR_*` env vars.
//!
//! ## Secrets policy
//!
//! No vendor API keys in this service (Strategy C plugin endpoint is per
//! spec §5 a customer-side service the predictor *calls*, not an internal
//! service that consumes vendor keys). The custom Debug impl is kept to
//! mirror the tokenizer's R2 M13 pattern — sink_tls paths and database URL
//! are inputs that we deliberately do NOT exclude from Debug because the
//! values themselves are non-secret (paths + DSN without password); only
//! the URL inside DATABASE_URL would carry a secret, and we mask it via
//! a `_present: bool` projection to avoid accidental log spill.

use serde::Deserialize;

/// Configuration loaded from env at boot.
#[derive(Clone, Deserialize)]
pub struct Config {
    /// gRPC listen socket; `127.0.0.1:50054` (compose / dev) or
    /// `0.0.0.0:50054` (Helm chart in-cluster). The Helm chart also
    /// mounts a UDS at `uds_path` when set.
    pub listen_addr: String,

    /// Optional UDS path. When set, the server binds the UDS instead of
    /// TCP per the tokenizer SLICE_03 R3 N1+N2 pattern. UDS uses
    /// kernel-enforced trust; TCP requires mTLS (production profile).
    #[serde(default)]
    pub uds_path: Option<String>,

    // -- mTLS server bootstrap (mirrors services/tokenizer/src/config.rs
    //    SLICE_03 R3 + services/ledger/src/config.rs §12.1) --
    /// Path to the predictor's workload TLS cert PEM. When set together
    /// with `tls_key_pem` and `tls_ca_pem`, the gRPC server starts in
    /// mTLS mode and rejects clients whose cert chain doesn't validate.
    #[serde(default)]
    pub tls_cert_pem: Option<String>,
    #[serde(default)]
    pub tls_key_pem: Option<String>,
    #[serde(default)]
    pub tls_ca_pem: Option<String>,

    /// Prometheus /metrics + /healthz + /readyz listen socket. Empty
    /// string = disable metrics endpoint.
    #[serde(default = "default_metrics_addr")]
    pub metrics_addr: String,

    /// Multi-tenant region label (echoed into telemetry; no tenant-bound
    /// state per stats-aggregator-spec §9 isolation invariant).
    #[serde(default)]
    pub region: String,

    /// Deploy profile (demo|production). Used by Helm chart for
    /// production-fail-fast gates. Mirrors tokenizer convention.
    #[serde(default)]
    pub profile: String,

    /// Postgres URL for the read-only output_distribution_cache lookup
    /// (spec §4.2). Empty = run in skeleton mode (Strategy B always falls
    /// back to L1; demo-only — production Helm profile requires this set).
    #[serde(default)]
    pub database_url: String,

    /// In-memory cache TTL in seconds (spec §4.3 default 5 minutes = 300s).
    /// Lower = more SQL load + fresher data; higher = less SQL + more stale.
    #[serde(default = "default_cache_ttl_seconds")]
    pub cache_ttl_seconds: u64,

    /// Default model context window for unknown models (spec §3.3).
    /// 8000 tokens per OpenAI's old gpt-3 standard.
    #[serde(default = "default_unknown_model_context_window")]
    pub unknown_model_context_window: i64,

    /// Per-tenant Predict RPC token-bucket refill rate. 0 disables the
    /// limiter for emergency rollback; production default protects the
    /// shared predictor hot path without requiring control-plane wiring.
    #[serde(default = "default_predict_rate_limit_per_tenant_per_second")]
    pub predict_rate_limit_per_tenant_per_second: u32,

    /// Maximum number of tenant limiter buckets retained in memory.
    /// Bounded to keep hot-path state finite; metrics stay no-label.
    #[serde(default = "default_predict_rate_limit_tenant_capacity")]
    pub predict_rate_limit_tenant_capacity: usize,

    /// Path to the model_context_window.toml file. Defaults to the
    /// crate-relative `data/model_context_window.toml` packed with the
    /// binary. Production deployments may override to point at a Helm-
    /// mounted ConfigMap for out-of-band refresh.
    #[serde(default = "default_context_window_toml_path")]
    pub context_window_toml_path: String,

    // -- SLICE_07: Strategy C plugin wiring (per output-predictor-plugin-
    //    contract-v1alpha1.md). Empty values = skeleton mode: every
    //    Predict falls to B silently because the endpoint cache returns
    //    NotConfigured (per spec §11). Production Helm gate requires
    //    these set when chart.profile=production AND when at least one
    //    tenant has registered a plugin endpoint via control plane API.
    //
    /// Postgres URL for the read-only predictor_plugin_endpoints lookup
    /// (spec §8). Empty = skeleton mode (Strategy C always returns
    /// NotConfigured; fall to B). Distinct from `database_url` because
    /// the plugin endpoints live in the control_plane DB while the
    /// output_distribution_cache lives in the canonical_ingest DB —
    /// keeping the two pools separate avoids accidental cross-DB
    /// connection-string reuse.
    #[serde(default)]
    pub plugin_endpoint_database_url: String,

    /// Endpoint cache refresh TTL in seconds (spec §6.3 — 60s cap by
    /// SLICE_07 design; lower values = more SQL load + faster pick-up
    /// of control plane API mutations).
    #[serde(default = "default_plugin_endpoint_cache_ttl_seconds")]
    pub plugin_endpoint_cache_ttl_seconds: u64,

    /// Plugin client TLS cert PEM path. SpendGuard presents this cert
    /// to the customer plugin endpoint per spec §3.1 (mTLS-only auth).
    /// HARDEN_08 adds `plugin_client_svid_dir`, which supersedes these
    /// legacy deploy-wide paths when Strategy C is enabled.
    /// All three legacy paths (cert / key / ca) must be set together;
    /// partial config fails at boot.
    #[serde(default)]
    pub plugin_client_cert_pem: Option<String>,
    #[serde(default)]
    pub plugin_client_key_pem: Option<String>,
    #[serde(default)]
    pub plugin_trust_ca_pem: Option<String>,

    /// Directory containing per-tenant SVID material subdirectories:
    /// `<plugin_client_svid_dir>/<client_cert_id>/{tls.crt,tls.key,ca.crt}`.
    /// `client_cert_id` comes from predictor_plugin_endpoints and is
    /// path-sanitized before use.
    #[serde(default)]
    pub plugin_client_svid_dir: Option<String>,
}

impl std::fmt::Debug for Config {
    /// Mirrors tokenizer R2 M13 pattern. database_url is masked via a
    /// `_present: bool` projection to avoid leaking embedded credentials
    /// into structured logs / panic backtraces.
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
            .field("cache_ttl_seconds", &self.cache_ttl_seconds)
            .field(
                "unknown_model_context_window",
                &self.unknown_model_context_window,
            )
            .field(
                "predict_rate_limit_per_tenant_per_second",
                &self.predict_rate_limit_per_tenant_per_second,
            )
            .field(
                "predict_rate_limit_tenant_capacity",
                &self.predict_rate_limit_tenant_capacity,
            )
            .field("context_window_toml_path", &self.context_window_toml_path)
            .field(
                "plugin_endpoint_database_url_present",
                &!self.plugin_endpoint_database_url.is_empty(),
            )
            .field(
                "plugin_endpoint_cache_ttl_seconds",
                &self.plugin_endpoint_cache_ttl_seconds,
            )
            .field("plugin_client_cert_pem", &self.plugin_client_cert_pem)
            .field("plugin_client_key_pem", &self.plugin_client_key_pem)
            .field("plugin_trust_ca_pem", &self.plugin_trust_ca_pem)
            .field("plugin_client_svid_dir", &self.plugin_client_svid_dir)
            .finish()
    }
}

fn default_metrics_addr() -> String {
    // Port 9100 — output_predictor in the demo compose port table:
    //   ledger=9092, canonical-ingest=9091, sidecar=9093,
    //   control-plane=9094, dashboard=9095, outbox=9096,
    //   ttl-sweeper=9097, webhook-receiver=9098, tokenizer=9099,
    //   output-predictor=9100, stats-aggregator=9101.
    "0.0.0.0:9100".to_string()
}

fn default_cache_ttl_seconds() -> u64 {
    // Spec §4.3 — 5 minutes.
    300
}

fn default_unknown_model_context_window() -> i64 {
    // Spec §3.3 — conservative 8000 tokens for unknown models.
    8000
}

fn default_predict_rate_limit_per_tenant_per_second() -> u32 {
    crate::server::DEFAULT_PREDICT_RATE_LIMIT_PER_TENANT_PER_SECOND
}

fn default_predict_rate_limit_tenant_capacity() -> usize {
    crate::server::DEFAULT_PREDICT_RATE_LIMIT_TENANT_CAPACITY
}

fn default_context_window_toml_path() -> String {
    // Relative to the binary's working directory in compose; Helm chart
    // mounts a ConfigMap at /etc/spendguard/output_predictor/
    // model_context_window.toml when override.
    "data/model_context_window.toml".to_string()
}

fn default_plugin_endpoint_cache_ttl_seconds() -> u64 {
    // R2 M2 (Security F4): cache TTL tightened from 60s → 5s.
    // Spec §6.3 health-check cadence is 30s; cache TTL must be ≤
    // that. The 5s value bounds the control-plane mutation →
    // predictor observation window at 5s, documented in spec §11
    // as the eventual-consistency operator contract. Tighter
    // consistency requires a cache_revision_at column on the
    // registry table (tracked as a follow-up GH issue).
    5
}

impl Config {
    /// Load from `SPENDGUARD_OUTPUT_PREDICTOR_*` env vars.
    pub fn from_env() -> Result<Self, envy::Error> {
        envy::prefixed("SPENDGUARD_OUTPUT_PREDICTOR_").from_env()
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
                "127.0.0.1:50054".to_string(),
            )])
            .expect("config loads");
        assert_eq!(cfg.listen_addr, "127.0.0.1:50054");
        assert!(cfg.uds_path.is_none());
        assert_eq!(cfg.metrics_addr, "0.0.0.0:9100");
        assert_eq!(cfg.cache_ttl_seconds, 300);
        assert_eq!(cfg.unknown_model_context_window, 8000);
        assert_eq!(cfg.predict_rate_limit_per_tenant_per_second, 1000);
        assert_eq!(cfg.predict_rate_limit_tenant_capacity, 4096);
    }

    #[test]
    fn debug_format_masks_database_url() {
        let cfg = envy::prefixed("TEST_CFG_")
            .from_iter::<_, Config>(vec![
                (
                    "TEST_CFG_LISTEN_ADDR".to_string(),
                    "127.0.0.1:50054".to_string(),
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
                "127.0.0.1:50054".to_string(),
            )])
            .expect("config loads");
        let dbg = format!("{cfg:?}");
        assert!(dbg.contains("database_url_present: false"));
    }
}
