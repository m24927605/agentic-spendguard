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

use std::sync::Arc;

use tokio::sync::Mutex;
use tonic::transport::Channel;
use tracing::{info, warn};

use super::worker::DriftAlertSink;
use crate::proto::canonical_ingest::v1::{
    canonical_ingest_client::CanonicalIngestClient, AppendEventsRequest,
};
use crate::proto::common::v1::CloudEvent;

/// Sink that forwards each event into canonical_ingest's AppendEvents
/// RPC. One event per call — SLICE_05 ships single-shot semantics for
/// simplicity; SLICE-extra can batch.
#[derive(Clone)]
pub struct CanonicalIngestDriftAlertSink {
    client: Arc<Mutex<CanonicalIngestClient<Channel>>>,
    producer_id: String,
}

impl std::fmt::Debug for CanonicalIngestDriftAlertSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanonicalIngestDriftAlertSink")
            .field("producer_id", &self.producer_id)
            .finish()
    }
}

impl CanonicalIngestDriftAlertSink {
    /// Connect to canonical_ingest via a tonic Channel. Callers pass
    /// the producer_id (e.g. `tokenizer-service:region-us-west2`) that
    /// will be echoed back in AppendEvents requests.
    pub async fn connect(
        endpoint: impl Into<String>,
        producer_id: impl Into<String>,
    ) -> Result<Self, anyhow::Error> {
        let endpoint: String = endpoint.into();
        info!(endpoint = %endpoint, "connecting drift_alert sink to canonical_ingest");
        let channel = Channel::from_shared(endpoint.clone())
            .map_err(|e| anyhow::anyhow!("invalid endpoint `{endpoint}`: {e}"))?
            .connect()
            .await
            .map_err(|e| anyhow::anyhow!("connect canonical_ingest `{endpoint}`: {e}"))?;
        Ok(Self {
            client: Arc::new(Mutex::new(CanonicalIngestClient::new(channel))),
            producer_id: producer_id.into(),
        })
    }
}

#[async_trait::async_trait]
impl DriftAlertSink for CanonicalIngestDriftAlertSink {
    async fn emit(&self, event: CloudEvent) -> Result<(), anyhow::Error> {
        let req = AppendEventsRequest {
            producer_id: self.producer_id.clone(),
            batch_max_producer_sequence: 0,
            // Per-event signature is on `event.producer_signature`; we
            // leave batch_signature empty (allowed in-cluster on mTLS
            // per the canonical_ingest.proto comment).
            batch_signature: bytes::Bytes::new(),
            signing_key_id: String::new(),
            schema_bundle: None,
            events: vec![event],
            route: 0, // ROUTE_UNSPECIFIED — canonical_ingest's
                      // observability path is appropriate for alerts.
        };
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
