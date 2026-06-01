//! Env-driven configuration for the tokenizer gRPC service.
//!
//! Mirrors the convention used by services/sidecar/src/config.rs and
//! services/ledger/src/config.rs — `envy` deserializes from
//! `SPENDGUARD_TOKENIZER_*` env vars.

use serde::Deserialize;

/// Configuration loaded from env at boot.
///
/// R2 M13: custom Debug impl masks `anthropic_api_key` + `gemini_api_key`
/// so structured logs / panic backtraces / error chains never leak the
/// raw keys. The startup log emits `*_api_key_present: bool` so
/// operators can verify configuration without leaking the secret.
#[derive(Clone, Deserialize)]
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

    /// Per-request synchronous encode timeout in milliseconds for the
    /// gRPC service form. Defaults to 30s so the 4 MiB accepted request
    /// cap has a compatible upper-bound budget; operators can lower this
    /// if they also lower the request cap or run tighter ingress limits.
    #[serde(default = "default_encode_timeout_ms")]
    pub encode_timeout_ms: u64,

    /// Maximum number of synchronous encode jobs allowed to run in the
    /// gRPC service form at once. The permit is held until the blocking
    /// encode closure actually returns, so request timeouts cannot create
    /// unbounded background CPU work.
    #[serde(default = "default_encode_max_concurrent")]
    pub encode_max_concurrent: usize,

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

    /// Optional Postgres URL for control_plane's durable tokenizer shadow
    /// tables: `tokenizer_sampling_rate_overrides`,
    /// `tokenizer_shadow_security_settings`, and
    /// `tokenizer_count_tokens_quota_usage`. When configured, the worker
    /// refreshes the current event's tenant/model overrides and claims
    /// count_tokens quota under that tenant's RLS context before provider
    /// raw-text egress. When unset, sampling overrides are absent and raw-text
    /// provider calls are default-denied.
    #[serde(default)]
    pub sampling_override_database_url: String,

    /// canonical_ingest gRPC URL for the signed `tokenizer_drift_alert`
    /// CloudEvent sink. Empty = use in-memory sink (demo only —
    /// production Helm profile requires this set).
    #[serde(default)]
    pub canonical_ingest_url: String,

    // ── HARDEN_03 / #168: AppendEventsRequest envelope fields ────────
    //
    // canonical_ingest rejects AppendEventsRequest envelopes without
    // producer_id + schema_bundle + non-ROUTE_UNSPECIFIED route. The
    // tokenizer shadow sink originally populated only producer_id and
    // left schema_bundle/route at defaults, so every drift alert was
    // rejected before admission. These fields mirror the stats_aggregator
    // envelope config and are required whenever canonical_ingest_url is
    // set.
    /// UUID string identifying the schema bundle tokenizer drift-alert
    /// CloudEvents conform to. Required when canonical_ingest_url is
    /// configured.
    #[serde(default)]
    pub schema_bundle_id: String,

    /// Hex-encoded SHA-256 of the schema bundle .tgz registered in
    /// canonical_ingest. Required when canonical_ingest_url is
    /// configured.
    #[serde(default)]
    pub schema_bundle_hash_hex: String,

    /// Canonical schema version written into SchemaBundleRef. Defaults
    /// to the repo-wide v1alpha1 schema bundle name.
    #[serde(default = "default_canonical_schema_version")]
    pub canonical_schema_version: String,

    /// Producer source URI written into the emitted CloudEvent. Defaults
    /// to `spendguard://tokenizer-service/<region>`. Surfaced for
    /// per-instance disambiguation in multi-region deploys.
    #[serde(default)]
    pub event_source_override: String,

    /// R2 B4 — paths for the outbound mTLS sink config to canonical_ingest.
    /// When all three are set the sink connects with mTLS; when all three
    /// are unset the sink falls back to plaintext (rejected by the Helm
    /// production profile). Partial config rejected at startup.
    #[serde(default)]
    pub sink_tls_cert_pem: Option<String>,
    #[serde(default)]
    pub sink_tls_key_pem: Option<String>,
    #[serde(default)]
    pub sink_tls_ca_pem: Option<String>,
    /// SNI domain to send in the TLS handshake. Defaults to
    /// `canonical-ingest.spendguard.internal` (matches the sidecar SNI).
    #[serde(default = "default_sink_sni")]
    pub sink_tls_sni: String,

    /// Deploy profile (demo|production). Used by signer_from_env to
    /// gate DisabledSigner. Also surfaced here so the worker's audit
    /// chain handling can fail-fast in production if the signer config
    /// is incomplete. Demo profile only accepts disabled signer.
    #[serde(default)]
    pub profile: String,
}

impl std::fmt::Debug for Config {
    /// R2 M13: mask API keys in Debug output so structured logs / panic
    /// backtraces / error chains never spill secrets. Reports key
    /// presence as a boolean so operators can verify config without
    /// the raw value.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("listen_addr", &self.listen_addr)
            .field("uds_path", &self.uds_path)
            .field("tls_cert_pem", &self.tls_cert_pem)
            .field("tls_key_pem", &self.tls_key_pem)
            .field("tls_ca_pem", &self.tls_ca_pem)
            .field("metrics_addr", &self.metrics_addr)
            .field("encode_timeout_ms", &self.encode_timeout_ms)
            .field("encode_max_concurrent", &self.encode_max_concurrent)
            .field("tier3_alert_threshold", &self.tier3_alert_threshold)
            .field("region", &self.region)
            .field("shadow_enabled", &self.shadow_enabled)
            .field(
                "shadow_default_sample_rate",
                &self.shadow_default_sample_rate,
            )
            .field(
                "anthropic_api_key_present",
                &!self.anthropic_api_key.is_empty(),
            )
            .field("gemini_api_key_present", &!self.gemini_api_key.is_empty())
            .field("database_url_present", &!self.database_url.is_empty())
            .field(
                "sampling_override_database_url_present",
                &!self.sampling_override_database_url.is_empty(),
            )
            .field("canonical_ingest_url", &self.canonical_ingest_url)
            .field("schema_bundle_id", &self.schema_bundle_id)
            .field(
                "schema_bundle_hash_hex_present",
                &!self.schema_bundle_hash_hex.is_empty(),
            )
            .field("canonical_schema_version", &self.canonical_schema_version)
            .field("event_source_override", &self.event_source_override)
            .field("sink_tls_cert_pem", &self.sink_tls_cert_pem)
            .field("sink_tls_key_pem", &self.sink_tls_key_pem)
            .field("sink_tls_ca_pem", &self.sink_tls_ca_pem)
            .field("sink_tls_sni", &self.sink_tls_sni)
            .field("profile", &self.profile)
            .finish()
    }
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

fn default_encode_timeout_ms() -> u64 {
    30_000
}

fn default_encode_max_concurrent() -> usize {
    32
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

fn default_sink_sni() -> String {
    "canonical-ingest.spendguard.internal".to_string()
}

fn default_canonical_schema_version() -> String {
    "spendguard.v1alpha1".to_string()
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
        let cfg = envy::prefixed("TEST_CFG_")
            .from_iter::<_, Config>(vec![(
                "TEST_CFG_LISTEN_ADDR".to_string(),
                "127.0.0.1:50053".to_string(),
            )])
            .expect("config loads");
        assert_eq!(cfg.listen_addr, "127.0.0.1:50053");
        assert!(cfg.uds_path.is_none());
        assert_eq!(cfg.metrics_addr, "0.0.0.0:9099");
        assert_eq!(cfg.encode_timeout_ms, 30_000);
        assert_eq!(cfg.encode_max_concurrent, 32);
        assert!((cfg.tier3_alert_threshold - 0.001).abs() < 1e-6);
    }

    #[test]
    fn tier3_threshold_overridable() {
        let cfg = envy::prefixed("TEST_CFG_")
            .from_iter::<_, Config>(vec![
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

    #[test]
    fn encode_timeout_overridable() {
        let cfg = envy::prefixed("TEST_CFG_")
            .from_iter::<_, Config>(vec![
                (
                    "TEST_CFG_LISTEN_ADDR".to_string(),
                    "127.0.0.1:50053".to_string(),
                ),
                ("TEST_CFG_ENCODE_TIMEOUT_MS".to_string(), "7500".to_string()),
            ])
            .expect("config loads");
        assert_eq!(cfg.encode_timeout_ms, 7500);
    }

    #[test]
    fn encode_max_concurrent_overridable() {
        let cfg = envy::prefixed("TEST_CFG_")
            .from_iter::<_, Config>(vec![
                (
                    "TEST_CFG_LISTEN_ADDR".to_string(),
                    "127.0.0.1:50053".to_string(),
                ),
                (
                    "TEST_CFG_ENCODE_MAX_CONCURRENT".to_string(),
                    "8".to_string(),
                ),
            ])
            .expect("config loads");
        assert_eq!(cfg.encode_max_concurrent, 8);
    }

    /// R2 M13: Debug must not spill the raw API keys.
    #[test]
    fn debug_format_masks_api_keys() {
        let cfg = envy::prefixed("TEST_CFG_")
            .from_iter::<_, Config>(vec![
                (
                    "TEST_CFG_LISTEN_ADDR".to_string(),
                    "127.0.0.1:50053".to_string(),
                ),
                (
                    "TEST_CFG_ANTHROPIC_API_KEY".to_string(),
                    "sk-ant-extremely-secret-token-DO-NOT-LEAK".to_string(),
                ),
                (
                    "TEST_CFG_GEMINI_API_KEY".to_string(),
                    "AIza-extremely-secret-DO-NOT-LEAK".to_string(),
                ),
            ])
            .expect("config loads");
        let dbg = format!("{cfg:?}");
        assert!(
            !dbg.contains("sk-ant-extremely-secret"),
            "Anthropic key leaked into Debug output: {dbg}"
        );
        assert!(
            !dbg.contains("AIza-extremely-secret"),
            "Gemini key leaked into Debug output: {dbg}"
        );
        // Both presence booleans rendered.
        assert!(dbg.contains("anthropic_api_key_present: true"));
        assert!(dbg.contains("gemini_api_key_present: true"));
    }

    #[test]
    fn debug_format_reports_missing_keys_as_false() {
        let cfg = envy::prefixed("TEST_CFG_")
            .from_iter::<_, Config>(vec![(
                "TEST_CFG_LISTEN_ADDR".to_string(),
                "127.0.0.1:50053".to_string(),
            )])
            .expect("config loads");
        let dbg = format!("{cfg:?}");
        assert!(dbg.contains("anthropic_api_key_present: false"));
        assert!(dbg.contains("gemini_api_key_present: false"));
    }
}
