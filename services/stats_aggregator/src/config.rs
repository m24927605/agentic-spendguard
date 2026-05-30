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

    // ── R2 B5: AppendEventsRequest envelope fields ────────────────────
    //
    // canonical_ingest's append handler validates producer_id +
    // schema_bundle + route fields on every AppendEventsRequest. The
    // R1 shape sent `..Default::default()` on these → empty producer_id
    // → "producer_id required"; missing schema_bundle → "schema_bundle
    // required"; ROUTE_UNSPECIFIED → "route is unspecified". Every
    // drift_alert emit was rejected.
    //
    // Mirrors the outbox_forwarder env contract
    // (services/outbox_forwarder/src/config.rs + outbox-forwarder.yaml).

    /// UUID string identifying the schema bundle stats_aggregator's
    /// outgoing drift_alert CloudEvents conform to. Same format as
    /// outboxForwarder.schemaBundleId in the Helm chart. Required at
    /// runtime when the CanonicalIngest sink is configured (production
    /// profile) — empty value triggers a sink-construction failure with
    /// a clear error.
    #[serde(default)]
    pub schema_bundle_id: String,

    /// Hex-encoded SHA-256 of the schema bundle .tgz. Must byte-exact
    /// match the canonical_ingest service's registered hash — mismatch
    /// → canonical_ingest rejects every event with SchemaBundleUnknown.
    /// Required at runtime when the sink is configured.
    #[serde(default)]
    pub schema_bundle_hash_hex: String,

    /// Canonical schema version string written into the
    /// SchemaBundleRef.canonical_schema_version field. Defaults to
    /// `spendguard.v1alpha1` matching the outbox_forwarder pattern.
    #[serde(default = "default_canonical_schema_version")]
    pub canonical_schema_version: String,
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
            .field("schema_bundle_id", &self.schema_bundle_id)
            .field("schema_bundle_hash_hex_present", &!self.schema_bundle_hash_hex.is_empty())
            .field("canonical_schema_version", &self.canonical_schema_version)
            .finish()
    }
}

fn default_canonical_schema_version() -> String {
    "spendguard.v1alpha1".to_string()
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
