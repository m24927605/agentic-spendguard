//! Ledger gRPC client wrapper.
//!
//! Holds a tonic `LedgerClient<Channel>` over mTLS; reconnects on channel
//! drop. POC scope: ReserveSet + Release + ReplayAuditFromCursor +
//! QueryDecisionOutcome. Other RPCs (Commit*/Refund/Dispute/Compensate/
//! NormalizeCost) are exposed via passthroughs but not used by the sidecar
//! quickstart shadow flow.

use std::sync::Arc;

use tonic::transport::{Channel, ClientTlsConfig, Endpoint};

use crate::{
    clients::mtls::{build_client_tls, MTlsPaths},
    domain::error::DomainError,
    proto::ledger::v1::{
        ledger_client::LedgerClient as LedgerProtoClient, CommitEstimatedRequest,
        CommitEstimatedResponse, QueryDecisionOutcomeRequest, QueryDecisionOutcomeResponse,
        QueryReservationContextRequest, QueryReservationContextResponse, ReleaseRequest,
        ReleaseResponse, ReplayAuditFromCursorRequest, ReserveSetRequest, ReserveSetResponse,
    },
};

#[derive(Clone)]
pub struct LedgerClient {
    inner: Arc<LedgerProtoClient<Channel>>,
}

impl LedgerClient {
    pub async fn connect(
        endpoint_url: String,
        sni: &str,
        mtls: &MTlsPaths,
    ) -> Result<Self, DomainError> {
        let tls = build_client_tls(mtls, sni).map_err(|e| {
            DomainError::LedgerClient(format!("build tls: {e}"))
        })?;
        let endpoint = Endpoint::from_shared(endpoint_url.clone())
            .map_err(|e| DomainError::LedgerClient(format!("endpoint parse: {e}")))?
            .tls_config(tls)
            .map_err(|e| DomainError::LedgerClient(format!("apply tls: {e}")))?
            .timeout(std::time::Duration::from_secs(5))
            .connect_timeout(std::time::Duration::from_secs(5))
            .keep_alive_timeout(std::time::Duration::from_secs(20))
            .keep_alive_while_idle(true);
        let channel = endpoint
            .connect()
            .await
            .map_err(|e| DomainError::LedgerClient(format!("connect {endpoint_url}: {e}")))?;
        Ok(Self {
            inner: Arc::new(LedgerProtoClient::new(channel)),
        })
    }

    pub async fn reserve_set(
        &self,
        req: ReserveSetRequest,
    ) -> Result<ReserveSetResponse, DomainError> {
        let mut client = (*self.inner).clone();
        let resp = client.reserve_set(req).await.map_err(|e| {
            DomainError::LedgerClient(format!("ReserveSet: {e}"))
        })?;
        Ok(resp.into_inner())
    }

    pub async fn release(&self, req: ReleaseRequest) -> Result<ReleaseResponse, DomainError> {
        let mut client = (*self.inner).clone();
        let resp = client
            .release(req)
            .await
            .map_err(|e| DomainError::LedgerClient(format!("Release: {e}")))?;
        Ok(resp.into_inner())
    }

    pub async fn commit_estimated(
        &self,
        req: CommitEstimatedRequest,
    ) -> Result<CommitEstimatedResponse, DomainError> {
        let mut client = (*self.inner).clone();
        let resp = client
            .commit_estimated(req)
            .await
            .map_err(|e| DomainError::LedgerClient(format!("CommitEstimated: {e}")))?;
        Ok(resp.into_inner())
    }

    pub async fn query_reservation_context(
        &self,
        req: QueryReservationContextRequest,
    ) -> Result<QueryReservationContextResponse, DomainError> {
        let mut client = (*self.inner).clone();
        let resp = client
            .query_reservation_context(req)
            .await
            .map_err(|e| {
                DomainError::LedgerClient(format!("QueryReservationContext: {e}"))
            })?;
        Ok(resp.into_inner())
    }

    pub async fn query_decision_outcome(
        &self,
        req: QueryDecisionOutcomeRequest,
    ) -> Result<QueryDecisionOutcomeResponse, DomainError> {
        let mut client = (*self.inner).clone();
        let resp = client
            .query_decision_outcome(req)
            .await
            .map_err(|e| DomainError::LedgerClient(format!("QueryDecisionOutcome: {e}")))?;
        Ok(resp.into_inner())
    }

    pub async fn replay_audit_from_cursor(
        &self,
        req: ReplayAuditFromCursorRequest,
    ) -> Result<tonic::Streaming<crate::proto::ledger::v1::ReplayAuditEvent>, DomainError>
    {
        let mut client = (*self.inner).clone();
        let resp = client
            .replay_audit_from_cursor(req)
            .await
            .map_err(|e| DomainError::LedgerClient(format!("ReplayAuditFromCursor: {e}")))?;
        Ok(resp.into_inner())
    }
}

/// POC default to TLS over the regional ledger endpoint announced via
/// the endpoint catalog. SNI domain matches the issued cert SAN.
pub fn default_sni(_endpoint_url: &str) -> &'static str {
    // For POC the cert-manager external issuer issues SAN "ledger.spendguard.internal";
    // production should derive SNI from the catalog endpoint's host.
    "ledger.spendguard.internal"
}

// Silence unused warning until ClientTlsConfig is consumed by callers.
#[allow(dead_code)]
fn _unused(_t: ClientTlsConfig) {}
