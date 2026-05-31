//! `tokenizer_drift_alert` CloudEvent sink backed by canonical_ingest.
//!
//! Spec ref `tokenizer-service-spec-v1alpha1.md` §4 (the signed alert
//! event lands in the canonical audit chain via canonical_ingest's
//! `AppendEvents` RPC).
//!
//! ## Mode selection
//!
//! Production wiring uses the gRPC client to canonical_ingest. Demo
//! and tests use the in-memory sink from
//! [`super::worker::InMemoryDriftAlertSink`].
//!
//! ## R2 B4 — mTLS
//!
//! Production builds MUST configure mTLS — the sink mirrors the
//! sidecar pattern at `services/sidecar/src/clients/canonical_ingest.rs`.
//! When mTLS paths are absent (demo only) the channel falls back to
//! plaintext gRPC with a loud warn. The Helm production profile
//! rejects this fallback via the chart's required-input gate.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use tokio::sync::Mutex;
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, Identity};
use tracing::{info, warn};

use super::worker::DriftAlertSink;
use crate::proto::canonical_ingest::v1::{
    append_events_request::Route, canonical_ingest_client::CanonicalIngestClient,
    AppendEventsRequest,
};
use crate::proto::common::v1::{CloudEvent, SchemaBundleRef};

/// R2 B4 — paths to cert + key + CA + SNI domain for mTLS to
/// canonical_ingest. Matches the
/// `services/sidecar/src/clients/mtls.rs::MTlsPaths` shape so the same
/// cert-manager mount works for both consumers.
#[derive(Debug, Clone)]
pub struct SinkMTlsConfig {
    pub workload_cert_pem: PathBuf,
    pub workload_key_pem: PathBuf,
    pub trust_ca_pem: PathBuf,
    pub sni_domain: String,
}

/// Sink that forwards each event into canonical_ingest's AppendEvents
/// RPC. One event per call — SLICE_05 ships single-shot semantics for
/// simplicity; SLICE-extra can batch.
#[derive(Clone)]
pub struct CanonicalIngestDriftAlertSink {
    client: Arc<Mutex<CanonicalIngestClient<tonic::transport::Channel>>>,
    producer_id: String,
    schema_bundle_ref: SchemaBundleRef,
    signing_key_id: String,
}

impl std::fmt::Debug for CanonicalIngestDriftAlertSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanonicalIngestDriftAlertSink")
            .field("producer_id", &self.producer_id)
            .field("schema_bundle_id", &self.schema_bundle_ref.schema_bundle_id)
            .field("signing_key_id", &self.signing_key_id)
            .finish()
    }
}

impl CanonicalIngestDriftAlertSink {
    /// Connect to canonical_ingest via a tonic Channel.
    ///
    /// R2 B4: when `mtls` is `Some(_)` the channel uses mTLS with the
    /// supplied identity + trust roots; `None` produces a plaintext
    /// channel and emits a loud warn (rejected by the production
    /// Helm profile via the chart's required-input gate).
    pub async fn connect(
        endpoint: impl Into<String>,
        producer_id: impl Into<String>,
        schema_bundle_ref: SchemaBundleRef,
        signing_key_id: impl Into<String>,
        mtls: Option<SinkMTlsConfig>,
    ) -> Result<Self, anyhow::Error> {
        let endpoint: String = endpoint.into();
        info!(
            endpoint = %endpoint,
            mtls = mtls.is_some(),
            "connecting drift_alert sink to canonical_ingest"
        );

        let mut ep = Endpoint::from_shared(endpoint.clone())
            .map_err(|e| anyhow::anyhow!("invalid endpoint `{endpoint}`: {e}"))?
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(5))
            .keep_alive_timeout(Duration::from_secs(20))
            .keep_alive_while_idle(true);

        if let Some(cfg) = mtls {
            let tls = build_client_tls(&cfg).context("build canonical_ingest sink tls config")?;
            ep = ep
                .tls_config(tls)
                .map_err(|e| anyhow::anyhow!("apply tls config: {e}"))?;
        } else {
            warn!(
                "canonical_ingest sink connecting WITHOUT mTLS — \
                 demo only; production Helm profile rejects this via \
                 the providerSecretName / canonicalIngestUrl required-input gate."
            );
        }

        let channel = ep
            .connect()
            .await
            .map_err(|e| anyhow::anyhow!("connect canonical_ingest `{endpoint}`: {e}"))?;

        Ok(Self {
            client: Arc::new(Mutex::new(CanonicalIngestClient::new(channel))),
            producer_id: producer_id.into(),
            schema_bundle_ref,
            signing_key_id: signing_key_id.into(),
        })
    }
}

pub(crate) fn build_append_events_request(
    producer_id: String,
    signing_key_id: String,
    schema_bundle_ref: SchemaBundleRef,
    event: CloudEvent,
) -> AppendEventsRequest {
    AppendEventsRequest {
        producer_id,
        batch_max_producer_sequence: 0,
        // Per-event signature is on `event.producer_signature`; batch
        // signatures remain optional for in-cluster mTLS producers.
        batch_signature: bytes::Bytes::new(),
        signing_key_id,
        schema_bundle: Some(schema_bundle_ref),
        events: vec![event],
        route: Route::Observability as i32,
    }
}

/// R2 B4 helper — load on-disk PEMs into a tonic `ClientTlsConfig` with
/// the supplied SNI. Mirrors `services/sidecar/src/clients/mtls.rs`.
fn build_client_tls(cfg: &SinkMTlsConfig) -> Result<ClientTlsConfig, anyhow::Error> {
    let cert_pem = std::fs::read_to_string(&cfg.workload_cert_pem)
        .with_context(|| format!("read workload cert {}", cfg.workload_cert_pem.display()))?;
    let key_pem = std::fs::read_to_string(&cfg.workload_key_pem)
        .with_context(|| format!("read workload key {}", cfg.workload_key_pem.display()))?;
    let ca_pem = std::fs::read_to_string(&cfg.trust_ca_pem)
        .with_context(|| format!("read trust CA {}", cfg.trust_ca_pem.display()))?;
    let identity = Identity::from_pem(cert_pem, key_pem);
    let ca = Certificate::from_pem(ca_pem);
    Ok(ClientTlsConfig::new()
        .identity(identity)
        .ca_certificate(ca)
        .domain_name(cfg.sni_domain.clone()))
}

#[async_trait::async_trait]
impl DriftAlertSink for CanonicalIngestDriftAlertSink {
    async fn emit(&self, event: CloudEvent) -> Result<(), anyhow::Error> {
        let req = build_append_events_request(
            self.producer_id.clone(),
            self.signing_key_id.clone(),
            self.schema_bundle_ref.clone(),
            event,
        );
        let mut client = self.client.lock().await;
        match client.append_events(req).await {
            Ok(resp) => {
                let resp = resp.into_inner();
                for r in &resp.results {
                    if let Some(err) = r.error.as_ref() {
                        if !err.message.is_empty() {
                            warn!(event_id = %r.event_id, err = %err.message,
                                  "canonical_ingest rejected drift_alert");
                        }
                    }
                }
                Ok(())
            }
            Err(e) => Err(anyhow::anyhow!("AppendEvents drift_alert: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_request_carries_required_observability_envelope() {
        let bundle = SchemaBundleRef {
            schema_bundle_id: "018fa6b3-0f59-7a4d-bbd2-4be8f4f14a10".to_string(),
            schema_bundle_hash: bytes::Bytes::from_static(&[0xab; 32]),
            canonical_schema_version: "spendguard.v1alpha1".to_string(),
        };
        let event = CloudEvent {
            id: "evt-1".to_string(),
            r#type: "spendguard.audit.tokenizer_drift_alert.v1alpha1".to_string(),
            ..Default::default()
        };

        let req = build_append_events_request(
            "tokenizer-service:us".to_string(),
            "tokenizer-key-v1".to_string(),
            bundle.clone(),
            event,
        );

        assert_eq!(req.producer_id, "tokenizer-service:us");
        assert_eq!(req.signing_key_id, "tokenizer-key-v1");
        assert_eq!(req.schema_bundle, Some(bundle));
        assert_eq!(req.route, Route::Observability as i32);
        assert_eq!(req.events.len(), 1);
        assert_eq!(
            req.events[0].r#type,
            "spendguard.audit.tokenizer_drift_alert.v1alpha1"
        );
    }
}
