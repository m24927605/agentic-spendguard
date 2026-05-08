//! Canonical Ingest gRPC client wrapper.
//!
//! Sidecar uses CI directly only for non-audit observability events
//! (SPAN_DELTA / TOOL_CALL_POST etc.). Audit events flow via
//! ledger.audit_outbox (Stage 2 §4 transactional outbox); ledger's
//! outbox forwarder pushes them to CI.

use std::sync::Arc;

use tonic::transport::{Channel, Endpoint};

use crate::{
    clients::mtls::{build_client_tls, MTlsPaths},
    domain::error::DomainError,
    proto::canonical_ingest::v1::{
        canonical_ingest_client::CanonicalIngestClient as CIProtoClient, AppendEventsRequest,
        AppendEventsResponse,
    },
};

#[derive(Clone)]
pub struct CanonicalIngestClient {
    inner: Arc<CIProtoClient<Channel>>,
}

impl CanonicalIngestClient {
    pub async fn connect(
        endpoint_url: String,
        sni: &str,
        mtls: &MTlsPaths,
    ) -> Result<Self, DomainError> {
        let tls = build_client_tls(mtls, sni).map_err(|e| {
            DomainError::CanonicalIngestClient(format!("build tls: {e}"))
        })?;
        let endpoint = Endpoint::from_shared(endpoint_url.clone())
            .map_err(|e| DomainError::CanonicalIngestClient(format!("endpoint parse: {e}")))?
            .tls_config(tls)
            .map_err(|e| DomainError::CanonicalIngestClient(format!("apply tls: {e}")))?
            .timeout(std::time::Duration::from_secs(5))
            .connect_timeout(std::time::Duration::from_secs(5))
            .keep_alive_timeout(std::time::Duration::from_secs(20))
            .keep_alive_while_idle(true);
        let channel = endpoint.connect().await.map_err(|e| {
            DomainError::CanonicalIngestClient(format!("connect {endpoint_url}: {e}"))
        })?;
        Ok(Self {
            inner: Arc::new(CIProtoClient::new(channel)),
        })
    }

    pub async fn append_events(
        &self,
        req: AppendEventsRequest,
    ) -> Result<AppendEventsResponse, DomainError> {
        let mut client = (*self.inner).clone();
        let resp = client
            .append_events(req)
            .await
            .map_err(|e| DomainError::CanonicalIngestClient(format!("AppendEvents: {e}")))?;
        Ok(resp.into_inner())
    }
}

pub fn default_sni(_endpoint_url: &str) -> &'static str {
    "canonical-ingest.spendguard.internal"
}
