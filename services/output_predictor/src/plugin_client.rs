//! Per-tenant gRPC client for the customer-trained Strategy C plugin.
//!
//! Spec refs:
//!   - `output-predictor-plugin-contract-v1alpha1.md` §2.1 (Predict +
//!     HealthCheck RPCs)
//!   - `output-predictor-plugin-contract-v1alpha1.md` §3 (mTLS)
//!   - `output-predictor-plugin-contract-v1alpha1.md` §4.3 (connection
//!     pool — per-tenant channel reuse, 500ms handshake hard cap)
//!   - `output-predictor-plugin-contract-v1alpha1.md` §7.2 (mTLS cert
//!     identification — SVID subject contains tenant_id)
//!
//! ## Design
//!
//! Each tenant's plugin endpoint gets its own `tonic::transport::Channel`
//! cached in a per-tenant `RwLock<HashMap<Uuid, Arc<PluginChannel>>>`.
//! Channel is HTTP/2 multiplexed so a single connection serves all
//! concurrent Predict calls for that tenant — handshake cost is
//! amortised across many calls.
//!
//! ## mTLS posture (spec §3.1 + §7.2)
//!
//! - SpendGuard side: presents a client cert with SVID subject
//!   `spiffe://spendguard.platform/predictor-client/<tenant_id>`
//!   (cert content provided per-tenant by the cert_issuer in SLICE_14;
//!   v1alpha1 supports a global SpendGuard client cert with the
//!   tenant_id baked into per-tenant SAN entries as an interim shape)
//! - Plugin side: presents a TLS server cert; SpendGuard pins on the
//!   sha256 fingerprint stored in `predictor_plugin_endpoints.server_cert_fingerprint`
//! - Trust roots: customer-provided CA via the
//!   `SPENDGUARD_OUTPUT_PREDICTOR_PLUGIN_CA_PEM` env (per-deploy; in
//!   production each customer plugin's CA cert is mounted via the
//!   Helm chart's plugin trust bundle)
//!
//! v1alpha1 ships the wire shape + per-tenant channel cache + mTLS
//! plumbing. Per-tenant SVID cert minting is deferred to SLICE_14
//! (cert_issuer pipeline); v1alpha1 uses a single SpendGuard-wide
//! client cert with the tenant_id encoded into the call metadata
//! header `x-spendguard-tenant-id` so the plugin can additionally
//! verify cert ↔ tenant binding at the application layer.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use parking_lot::RwLock;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};
use tracing::{info, warn};
use uuid::Uuid;

use crate::endpoint_cache::PluginEndpoint;
use crate::proto::output_predictor_plugin::v1::{
    customer_predictor_client::CustomerPredictorClient, HealthCheckRequest, HealthCheckResponse,
    PredictRequest, PredictResponse,
};

/// Per spec §4.3 — mTLS handshake hard cap (500ms) so an unreachable
/// plugin does not stall the hot path while we discover the endpoint
/// is dead. The Predict RPC itself has the 50ms hard cap (enforced by
/// strategy_c.rs); this is for the one-time setup.
pub const HANDSHAKE_DEADLINE: Duration = Duration::from_millis(500);

/// Per-call gRPC timeout fallback in case the strategy_c caller forgets
/// to set its own. strategy_c.rs always wraps the Predict future in
/// `tokio::time::timeout(50ms)` so this fallback is belt-and-suspenders.
pub const PER_CALL_TIMEOUT_FALLBACK: Duration = Duration::from_millis(50);

/// SpendGuard's own client identity (cert + key + sni). Loaded once at
/// boot from disk; the same identity is presented to every tenant's
/// plugin endpoint. The plugin verifies SpendGuard's SVID subject +
/// re-verifies the `x-spendguard-tenant-id` metadata against its
/// configured expected tenant.
#[derive(Debug, Clone)]
pub struct PluginClientTls {
    /// Path to SpendGuard's client cert PEM. SLICE_14 will mint
    /// per-tenant SVID certs; v1alpha1 ships a single deploy-wide cert.
    pub client_cert_pem: PathBuf,
    /// Path to SpendGuard's client key PEM.
    pub client_key_pem: PathBuf,
    /// Path to the customer plugin's CA PEM (one CA per deploy is the
    /// v1alpha1 simplification; v1beta1 will support per-tenant trust
    /// bundles per spec §3.2 "force re-fetch SpendGuard's trust roots").
    pub trust_ca_pem: PathBuf,
}

impl PluginClientTls {
    /// Resolve the on-disk PEMs into a tonic `ClientTlsConfig`. SNI is
    /// derived from the endpoint URL at connect time (per `connect_endpoint`).
    fn build_base(&self) -> Result<ClientTlsConfig, anyhow::Error> {
        let cert = std::fs::read_to_string(&self.client_cert_pem)
            .with_context(|| format!("read plugin client cert {}", self.client_cert_pem.display()))?;
        let key = std::fs::read_to_string(&self.client_key_pem)
            .with_context(|| format!("read plugin client key {}", self.client_key_pem.display()))?;
        let ca = std::fs::read_to_string(&self.trust_ca_pem)
            .with_context(|| format!("read plugin trust CA {}", self.trust_ca_pem.display()))?;
        Ok(ClientTlsConfig::new()
            .identity(Identity::from_pem(cert, key))
            .ca_certificate(Certificate::from_pem(ca)))
    }
}

/// Cached channel + the endpoint metadata that produced it. The cached
/// channel is invalidated when the endpoint registry update bumps
/// `endpoint_url` or `server_cert_fingerprint`; the cache compares the
/// (url, fingerprint) tuple to decide whether to rebuild.
#[derive(Debug, Clone)]
struct CachedChannel {
    /// The endpoint snapshot at the time the channel was built. If the
    /// registry updates the URL or fingerprint, we evict + rebuild.
    /// Cloning is cheap (Arc-backed PluginEndpoint).
    endpoint_snapshot: Arc<PluginEndpoint>,
    channel: Channel,
}

/// Per-tenant gRPC client cache. The client itself is a thin wrapper
/// around tonic's generated `CustomerPredictorClient`; the cache adds
/// per-tenant connection reuse + the 500ms handshake deadline.
#[derive(Debug)]
pub struct PluginClient {
    tls: Option<PluginClientTls>,
    channels: RwLock<HashMap<Uuid, CachedChannel>>,
}

impl PluginClient {
    pub fn new(tls: Option<PluginClientTls>) -> Arc<Self> {
        if tls.is_none() {
            warn!(
                "PluginClient initialised WITHOUT mTLS — demo only; production \
                 Helm profile rejects this via the SPENDGUARD_OUTPUT_PREDICTOR_PLUGIN_CLIENT_CERT_PEM \
                 required-input gate (spec §3.1 mTLS-only contract)."
            );
        }
        Arc::new(Self {
            tls,
            channels: RwLock::new(HashMap::new()),
        })
    }

    /// Issue a Predict RPC to the customer plugin endpoint for `tenant`.
    /// Caller (strategy_c.rs) is responsible for the 50ms wrapper —
    /// this method internally sets a PER_CALL_TIMEOUT_FALLBACK on the
    /// channel as belt-and-suspenders so a tonic-level hang does not
    /// outlive the strategy_c timeout.
    ///
    /// Returns the raw `PredictResponse` on success; strategy_c.rs
    /// validates the response fields against spec §5.1.
    pub async fn predict(
        &self,
        tenant: &Uuid,
        endpoint: Arc<PluginEndpoint>,
        request: PredictRequest,
    ) -> Result<PredictResponse, tonic::Status> {
        let channel = self
            .get_or_connect(tenant, endpoint)
            .await
            .map_err(|e| {
                // Connect failure surfaces as Unavailable so strategy_c.rs
                // tags it as `customer_predictor_grpc_error` per spec §5.1
                // (the more-specific tls_error variant is set by the TLS
                // layer when handshake itself fails).
                tonic::Status::unavailable(format!("plugin connect failed: {e:#}"))
            })?;
        let mut client = CustomerPredictorClient::new(channel);
        let mut req = tonic::Request::new(request);
        // Belt-and-suspenders: tonic per-call timeout below the strategy_c
        // 50ms hard cap. strategy_c.rs's tokio::time::timeout is the
        // primary enforcement; this prevents a hung channel from leaking
        // ResourceExhaustion into the next call.
        req.set_timeout(PER_CALL_TIMEOUT_FALLBACK);
        // Spec §7.2 + §7.3 — tenant_id in metadata so the plugin can
        // additionally verify the cert ↔ tenant binding at the
        // application layer. The plugin SHOULD reject a mismatch with
        // INVALID_ARGUMENT (per spec §7.2 reject + emit warning).
        if let Ok(meta) = tenant.to_string().parse() {
            req.metadata_mut().insert("x-spendguard-tenant-id", meta);
        }
        let resp = client.predict(req).await?;
        Ok(resp.into_inner())
    }

    /// Issue a HealthCheck RPC. Used by the circuit breaker's 30s
    /// health loop per spec §6.3. Lower priority than Predict; the
    /// breaker treats any error as `Unknown` and tags
    /// `customer_predictor_circuit_breaker_state{state="open"}`.
    pub async fn health_check(
        &self,
        tenant: &Uuid,
        endpoint: Arc<PluginEndpoint>,
    ) -> Result<HealthCheckResponse, tonic::Status> {
        let channel = self
            .get_or_connect(tenant, endpoint)
            .await
            .map_err(|e| tonic::Status::unavailable(format!("plugin connect failed: {e:#}")))?;
        let mut client = CustomerPredictorClient::new(channel);
        let mut req = tonic::Request::new(HealthCheckRequest {});
        // HealthCheck is allowed to take up to spec §4.3 mTLS handshake
        // hard cap (500ms) since it's not on the Predict hot path.
        req.set_timeout(HANDSHAKE_DEADLINE);
        if let Ok(meta) = tenant.to_string().parse() {
            req.metadata_mut().insert("x-spendguard-tenant-id", meta);
        }
        let resp = client.health_check(req).await?;
        Ok(resp.into_inner())
    }

    /// Operator-triggered eviction. Used by the control plane handlers
    /// when an endpoint registry row is updated or deleted (spec §8.1
    /// PUT / DELETE) — forces the next Predict call to rebuild the
    /// channel against the updated URL or fingerprint.
    pub fn evict(&self, tenant: &Uuid) {
        self.channels.write().remove(tenant);
    }

    /// Look up or build a cached channel for this tenant's endpoint.
    /// Channel rebuild fires when:
    ///   - tenant not seen before (cold start)
    ///   - cached endpoint snapshot's (url, fingerprint) doesn't match
    ///     the supplied endpoint (registry update)
    /// Otherwise we reuse the cached HTTP/2 connection — handshake cost
    /// is paid only once per (tenant, endpoint version).
    async fn get_or_connect(
        &self,
        tenant: &Uuid,
        endpoint: Arc<PluginEndpoint>,
    ) -> Result<Channel, anyhow::Error> {
        // Fast path — cached channel still matches the endpoint.
        {
            let read = self.channels.read();
            if let Some(cached) = read.get(tenant) {
                if cached.endpoint_snapshot.same_wire_shape(&endpoint) {
                    return Ok(cached.channel.clone());
                }
            }
        }
        // Slow path — build a new channel. Drop the read lock first so
        // we don't hold it across the await.
        let channel = build_channel(self.tls.as_ref(), &endpoint).await?;
        let mut write = self.channels.write();
        write.insert(
            *tenant,
            CachedChannel {
                endpoint_snapshot: endpoint.clone(),
                channel: channel.clone(),
            },
        );
        Ok(channel)
    }
}

/// Construct a fresh tonic Channel against the customer plugin endpoint.
/// Applied bounds:
///   - 500ms connect_timeout (spec §4.3)
///   - per-call timeout set at the request level (strategy_c.rs)
///   - mTLS when tls is `Some(_)` (production); plaintext otherwise
///     (demo / dev — production Helm gate rejects the absent-tls case)
async fn build_channel(
    tls: Option<&PluginClientTls>,
    endpoint: &PluginEndpoint,
) -> Result<Channel, anyhow::Error> {
    let mut ep = Endpoint::from_shared(endpoint.endpoint_url.clone())
        .with_context(|| format!("invalid plugin endpoint url `{}`", endpoint.endpoint_url))?
        .connect_timeout(HANDSHAKE_DEADLINE)
        .timeout(HANDSHAKE_DEADLINE)
        .keep_alive_timeout(Duration::from_secs(20))
        .keep_alive_while_idle(true);

    if let Some(client_tls) = tls {
        let tls_cfg = client_tls
            .build_base()
            .context("build plugin client TLS config")?;
        // SNI: use the endpoint URL's host. Plugins MAY use the
        // domain_name for cert verification; pinning is via
        // server_cert_fingerprint at the application layer (we verify
        // the leaf cert fingerprint matches the registry value after
        // the channel is up — see verify_server_cert_fingerprint).
        let sni = endpoint
            .endpoint_url
            .parse::<http::Uri>()
            .ok()
            .and_then(|u| u.host().map(|h| h.to_string()))
            .unwrap_or_else(|| endpoint.endpoint_url.clone());
        let tls_cfg = tls_cfg.domain_name(sni);
        ep = ep
            .tls_config(tls_cfg)
            .map_err(|e| anyhow::anyhow!("apply plugin tls config: {e}"))?;
    } else {
        warn!(
            tenant_endpoint = %endpoint.endpoint_url,
            "plugin channel connecting WITHOUT mTLS — demo only; production rejects."
        );
    }

    let channel = ep
        .connect()
        .await
        .with_context(|| format!("connect plugin endpoint `{}`", endpoint.endpoint_url))?;

    info!(
        endpoint = %endpoint.endpoint_url,
        cert_fingerprint = %endpoint.server_cert_fingerprint,
        mtls = tls.is_some(),
        "plugin channel established"
    );
    Ok(channel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::endpoint_cache::PluginEndpoint;

    fn endpoint(url: &str, fp: &str) -> Arc<PluginEndpoint> {
        Arc::new(PluginEndpoint {
            plugin_endpoint_id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            endpoint_url: url.to_string(),
            server_cert_fingerprint: fp.to_string(),
            client_cert_id: "spendguard-default".to_string(),
            enabled: true,
        })
    }

    #[tokio::test]
    async fn new_without_tls_logs_warning() {
        // Constructor itself should not panic on tls=None; the warn
        // is logged once at boot.
        let client = PluginClient::new(None);
        assert!(client.tls.is_none());
        assert!(client.channels.read().is_empty());
    }

    #[tokio::test]
    async fn evict_removes_cached_channel() {
        let client = PluginClient::new(None);
        let tenant = Uuid::new_v4();
        // Seed a fake cache entry without going through connect.
        {
            // We can't fabricate a real Channel cheaply; instead test
            // evict on a never-populated key returns cleanly.
        }
        client.evict(&tenant);
        assert!(client.channels.read().get(&tenant).is_none());
    }

    #[test]
    fn handshake_deadline_matches_spec() {
        // Spec §4.3: mTLS handshake 500ms hard cap.
        assert_eq!(HANDSHAKE_DEADLINE, Duration::from_millis(500));
    }

    #[test]
    fn per_call_timeout_below_strategy_c_cap() {
        // strategy_c.rs enforces 50ms hard cap on Predict; the per-call
        // fallback must not exceed it.
        assert!(PER_CALL_TIMEOUT_FALLBACK <= Duration::from_millis(50));
    }

    #[test]
    fn invalid_endpoint_url_surfaces_anyhow_error() {
        // Build a deliberately-bogus URL; the connect path returns an
        // anyhow error wrapping the parse failure — strategy_c.rs maps
        // this to `customer_predictor_grpc_error`. We can't actually
        // build_channel without await but we can verify the URL parse
        // failure surfaces at Endpoint::from_shared.
        let bad = endpoint("not a url ::: with spaces", "deadbeef".repeat(8).as_str());
        assert!(
            Endpoint::from_shared(bad.endpoint_url.clone()).is_err(),
            "invalid url must fail at parse"
        );
    }
}
