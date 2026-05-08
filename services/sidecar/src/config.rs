use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// UDS path the in-process adapter connects to.
    /// Default matches `/var/run/spendguard/adapter.sock` per Sidecar §5.
    #[serde(default = "default_uds_path")]
    pub uds_path: String,

    /// Tenant id for this sidecar instance (per-pod identity).
    pub tenant_id: String,

    /// Workload instance id — unique per pod / VM. Sidecar §9 fencing
    /// scopes are per-(tenant, workload_instance_id).
    pub workload_instance_id: String,

    /// Region (e.g., "us-west-2"). Used for endpoint catalog filtering.
    pub region: String,

    /// Endpoint catalog manifest URL — pulled at startup + every 5 min.
    pub endpoint_catalog_manifest_url: String,

    /// Helm-pinned root CA bundle PEM contents (for verifying control
    /// plane manifests + mTLS chain). Sourced from the
    /// `spendguard-trust` Kubernetes Secret per Stage 2 §12.1.
    pub trust_root_ca_pem: String,

    /// Helm-pinned root SPKI hash (sha256 hex) — defense-in-depth pin
    /// against intermediate compromise.
    pub trust_root_spki_sha256_hex: String,

    /// One-time mTLS bootstrap token used to provision sidecar workload cert
    /// via cert-manager external issuer. Removed from disk after first
    /// successful issue.
    pub mtls_bootstrap_token: String,

    /// Capability level this sidecar advertises in adapter handshake.
    /// L0 / L1 / L2 / L3. POC default L3.
    #[serde(default = "default_capability_level")]
    pub capability_level: String,

    /// Enforcement strength advertised to contracts.
    /// advisory_sdk / semantic_adapter / egress_proxy_hard_block / provider_key_gateway.
    /// POC default semantic_adapter.
    #[serde(default = "default_enforcement_strength")]
    pub enforcement_strength: String,

    /// Manifest pull cadence in seconds. MUST be < the manifest's
    /// `valid_until - issued_at` window (publisher default 600s) so the
    /// sidecar always has a fresh manifest before expiry. Default 60s.
    #[serde(default = "default_manifest_pull_seconds")]
    pub manifest_pull_seconds: u64,

    /// Critical revocation max stale (Sidecar §7) — enforcement routes
    /// fail_closed if last_verified_critical_version_age exceeds this.
    /// Default 300s (5 min).
    #[serde(default = "default_critical_max_stale_seconds")]
    pub critical_max_stale_seconds: u64,

    /// Drain window in seconds. Aligns with K8s
    /// `terminationGracePeriodSeconds`; default 60s for K8s SaaS mode.
    #[serde(default = "default_drain_window_seconds")]
    pub drain_window_seconds: u64,

    /// Decision-boundary p99 budget in milliseconds (Contract §14).
    /// Warm SLO 50ms.
    #[serde(default = "default_decision_p99_ms")]
    pub decision_p99_ms: u64,

    /// Optional metrics bind address (Prometheus).
    #[serde(default = "default_metrics_addr")]
    pub metrics_addr: String,

    /// Health probe bind (kubelet readiness/liveness).
    #[serde(default = "default_health_addr")]
    pub health_addr: String,

    // -- Bundle bootstrap (Stage 2 §5 + §9.4) -----------------------------
    /// Local path containing pre-pulled `contract_bundle/<id>.tgz` +
    /// `schema_bundle/<id>.tgz` artifacts. POC default
    /// `/var/lib/spendguard/bundles`. The Helm chart's init container
    /// populates this from the Bundle Registry (cosign verified upstream).
    #[serde(default = "default_bundle_root")]
    pub bundle_root: String,

    /// Contract bundle id + sha256 hex hash to load at startup.
    pub contract_bundle_id: String,
    pub contract_bundle_hash_hex: String,

    /// Schema bundle id + canonical schema version (Trace §12).
    pub schema_bundle_id: String,
    #[serde(default = "default_canonical_schema_version")]
    pub schema_bundle_canonical_version: String,

    // -- Fencing bootstrap (Sidecar §9 + Stage 2 §4.4) --------------------
    /// Pre-provisioned fencing scope id the operator created in the
    /// ledger for this (tenant, workload_instance_id). Phase 2 adds a
    /// proper Ledger.AcquireFencingScope RPC.
    pub fencing_scope_id: String,

    /// Caller's expected current_epoch for the scope above (MUST equal
    /// `fencing_scopes.current_epoch`).
    pub fencing_initial_epoch: u64,

    /// Local TTL after which the sidecar refuses new decisions (defense
    /// in depth; ledger is authoritative).
    #[serde(default = "default_fencing_ttl_seconds")]
    pub fencing_ttl_seconds: i64,

    // -- Idempotency cache (Stage 2 §4.3 + Contract §6) -------------------
    /// Decision idempotency cache size (in-memory LRU). Retries within
    /// this many entries return the cached DecisionResponse instead of
    /// minting a new decision_id and a duplicate ledger transaction.
    #[serde(default = "default_idempotency_cache_size")]
    pub idempotency_cache_size: usize,

    /// Idempotency cache TTL — entries older than this are evicted.
    #[serde(default = "default_idempotency_cache_ttl_secs")]
    pub idempotency_cache_ttl_secs: i64,
}

fn default_uds_path() -> String {
    "/var/run/spendguard/adapter.sock".to_string()
}
fn default_capability_level() -> String {
    "L3_POLICY_HOOK".to_string()
}
fn default_enforcement_strength() -> String {
    "semantic_adapter".to_string()
}
fn default_manifest_pull_seconds() -> u64 {
    60
}
fn default_critical_max_stale_seconds() -> u64 {
    300
}
fn default_drain_window_seconds() -> u64 {
    60
}
fn default_decision_p99_ms() -> u64 {
    50
}
fn default_metrics_addr() -> String {
    "127.0.0.1:9090".to_string()
}
fn default_health_addr() -> String {
    "127.0.0.1:8080".to_string()
}
fn default_bundle_root() -> String {
    "/var/lib/spendguard/bundles".to_string()
}
fn default_canonical_schema_version() -> String {
    "spendguard.v1alpha1".to_string()
}
fn default_fencing_ttl_seconds() -> i64 {
    120
}
fn default_idempotency_cache_size() -> usize {
    8192
}
fn default_idempotency_cache_ttl_secs() -> i64 {
    600
}

impl Config {
    pub fn from_env() -> Result<Self, envy::Error> {
        envy::prefixed("SPENDGUARD_SIDECAR_").from_env::<Config>()
    }
}
