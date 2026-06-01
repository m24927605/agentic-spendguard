//! Run Cost Projector gRPC client wrapper.
//!
//! Spec ref `run-cost-projector-spec-v1alpha1.md` §2.1.
//!
//! ## Why a thin wrapper
//!
//! Sidecar's decision/transaction.rs needs to call Project right after
//! output_predictor.Predict (which currently runs server-side inside
//! sidecar via the legacy in-process estimator; SLICE_10 wires the real
//! output_predictor client). The wrapper hides tonic's Channel + Endpoint
//! construction and surfaces a single async function per RPC.
//!
//! ## Failure mode
//!
//! Spec §10: "projector RPC unreachable from sidecar → Sidecar conservative
//! fall-through: no RUN_* emitted; reservation 仍正確 (用 A); emit metric
//! `projector_unreachable`."
//!
//! The wrapper returns Result<ProjectResponse, DomainError>; the caller
//! (decision/transaction.rs) is responsible for catching the Err and
//! converting it into the spec §10 fall-through (defaults: no code emitted,
//! audit row carries -1 sentinel for remaining_steps per audit-chain-
//! extension §3.3).

use std::sync::Arc;

use tonic::transport::{Channel, Endpoint};

use crate::{
    clients::mtls::{build_client_tls, MTlsPaths},
    domain::error::DomainError,
    proto::run_cost_projector::v1::{
        run_cost_projector_client::RunCostProjectorClient as RunCostProjectorProtoClient,
        ProjectRequest, ProjectResponse, TerminateRunRequest, TerminateRunResponse,
    },
};

#[derive(Clone)]
pub struct RunCostProjectorClient {
    inner: Arc<RunCostProjectorProtoClient<Channel>>,
}

impl RunCostProjectorClient {
    /// Connect over mTLS to a TCP endpoint. Mirrors LedgerClient pattern;
    /// callers in production point at a Helm-deployed Service.
    pub async fn connect(
        endpoint_url: String,
        sni: &str,
        mtls: &MTlsPaths,
        timeout_ms: u64,
    ) -> Result<Self, DomainError> {
        let mut endpoint = Endpoint::from_shared(endpoint_url.clone())
            .map_err(|e| DomainError::LedgerClient(format!("projector endpoint parse: {e}")))?;
        if endpoint_url.starts_with("https://") {
            let tls = build_client_tls(mtls, sni)
                .map_err(|e| DomainError::LedgerClient(format!("projector build tls: {e}")))?;
            endpoint = endpoint
                .tls_config(tls)
                .map_err(|e| DomainError::LedgerClient(format!("projector apply tls: {e}")))?;
        } else {
            tracing::warn!(
                endpoint = %endpoint_url,
                "run_cost_projector client using plaintext; only valid for demo/test"
            );
        }
        let timeout_ms = timeout_ms.max(1);
        let endpoint = endpoint
            // Keep the cap configurable. Production defaults stay inside
            // the sidecar decision budget; local smoke gates may override.
            .timeout(std::time::Duration::from_millis(timeout_ms))
            .connect_timeout(std::time::Duration::from_secs(5))
            .keep_alive_timeout(std::time::Duration::from_secs(20))
            .keep_alive_while_idle(true);
        let channel = endpoint.connect().await.map_err(|e| {
            DomainError::LedgerClient(format!("projector connect {endpoint_url}: {e}"))
        })?;
        Ok(Self {
            inner: Arc::new(RunCostProjectorProtoClient::new(channel)),
        })
    }

    /// Project: hot-path RPC. Sidecar calls per decision (LLM_CALL_PRE).
    /// On failure (timeout, network, validation), caller falls through
    /// per spec §10.
    pub async fn project(
        &self,
        req: ProjectRequest,
    ) -> Result<ProjectResponse, DomainError> {
        let mut client = (*self.inner).clone();
        let resp = client.project(req).await.map_err(|e| {
            DomainError::LedgerClient(format!("RunCostProjector.Project: {e}"))
        })?;
        Ok(resp.into_inner())
    }

    /// TerminateRun: lifecycle RPC. Sidecar emits on RUN_END event.
    /// Idempotent per spec §2.1.
    pub async fn terminate_run(
        &self,
        req: TerminateRunRequest,
    ) -> Result<TerminateRunResponse, DomainError> {
        let mut client = (*self.inner).clone();
        let resp = client.terminate_run(req).await.map_err(|e| {
            DomainError::LedgerClient(format!("RunCostProjector.TerminateRun: {e}"))
        })?;
        Ok(resp.into_inner())
    }
}
