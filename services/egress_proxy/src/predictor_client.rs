//! SLICE_10 Phase A — gRPC clients for output_predictor + run_cost_projector.
//!
//! The egress_proxy hot path runs THREE prediction services per decision:
//!
//!   1. tokenizer library (in-process, p99 ≤ 1ms, no RPC) — invoked inline
//!      in `decision::estimate_call_cost` via `Tokenizer::tokenize`.
//!   2. output_predictor gRPC (p99 ≤ 10ms hard cap) — Strategy A/B/C +
//!      selector + cold-start chain. Spec §11.1.
//!   3. run_cost_projector gRPC (p99 ≤ 5ms hard cap) — projection +
//!      RUN_* signal emission. Spec §12.1.
//!
//! Both gRPC clients are optional: when their endpoint URL env var is
//! unset (default in single-node demo deployments), the client is None and
//! `estimate_call_cost` falls back per spec §11 failure modes.
//!
//! Failure modes (per `predictor-architecture-spec-v1alpha1.md` §11 +
//! `audit-chain-prediction-extension-v1alpha1.md` §3.3 sentinels):
//!
//! | Client            | Failure                | Egress proxy behaviour                                        |
//! |-------------------|------------------------|---------------------------------------------------------------|
//! | output_predictor  | unreachable / timeout  | fall back to local Strategy A; `predicted_b/c_tokens = 0`     |
//! | run_cost_projector| unreachable / timeout  | pass-through (no RUN_* code); `run_predicted_remaining = -1`  |
//! | tokenizer library | panic                  | fail-closed at sidecar boot (Tier 2 panic invariant)          |
//!
//! Per SLICE_05/06/07 R1 lessons: this module is the integration point
//! and MUST NOT carry drop-handle fallbacks for production main.rs paths.
//! Configuration via env var: missing var ⇒ `Option::None` returned at
//! construction; runtime guards in `estimate_call_cost` consume `Option`s.

use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;
use tonic::transport::{Channel, Endpoint};
use tracing::{debug, warn};

use crate::proto::output_predictor::v1::{
    output_predictor_client::OutputPredictorClient as OutputPredictorProtoClient,
    PredictRequest, PredictResponse,
};
use crate::proto::run_cost_projector::v1::{
    run_cost_projector_client::RunCostProjectorClient as RunCostProjectorProtoClient,
    ProjectRequest, ProjectResponse, TerminateRunRequest, TerminateRunResponse,
};

/// Hot-path timeout for output_predictor.Predict.
///
/// Spec §11.1 budgets A only ≤ 1ms, A+B ≤ 5ms, A+B+C ≤ 15ms. The
/// egress_proxy hard caps the call at 10ms; if the predictor exceeds
/// that, we fall through to local Strategy A so the per-request decision
/// latency stays under the Contract §14 50ms p99 budget.
pub const OUTPUT_PREDICTOR_TIMEOUT_MS: u64 = 10;

/// Hot-path timeout for run_cost_projector.Project.
///
/// Spec §12.1 budgets p99 ≤ 5ms warm / 10ms cold. We cap at 5ms hard;
/// projector failure ⇒ no RUN_* code emitted (per-call decision unaffected).
pub const RUN_COST_PROJECTOR_TIMEOUT_MS: u64 = 5;

#[derive(Error, Debug)]
pub enum PredictorClientError {
    #[error("output_predictor endpoint parse: {0}")]
    EndpointParse(String),

    #[error("output_predictor connect: {0}")]
    Connect(String),

    #[error("output_predictor RPC: {0}")]
    Rpc(String),

    #[error("output_predictor timeout after {0}ms")]
    Timeout(u64),
}

/// SLICE_10 Phase A — thin tonic wrapper around the output_predictor service.
///
/// `Clone` is cheap (channel under Arc). One client instance per process is
/// the expected pattern — egress_proxy `main.rs` constructs once at boot,
/// then shares via `Arc<AppState>` to each request handler.
#[derive(Clone)]
pub struct OutputPredictorClient {
    inner: Arc<OutputPredictorProtoClient<Channel>>,
}

impl OutputPredictorClient {
    /// Connect over plaintext gRPC (no TLS).
    ///
    /// This is the demo/on-node deployment path. Cross-node production
    /// deployments should use `connect_mtls` once the SLICE_07 cert-pinning
    /// pattern lands for egress_proxy.
    pub async fn connect(endpoint_url: String) -> Result<Self, PredictorClientError> {
        let endpoint = Endpoint::from_shared(endpoint_url.clone())
            .map_err(|e| PredictorClientError::EndpointParse(e.to_string()))?
            // Per-RPC timeout is enforced on each call_with_timeout; the
            // tonic-level transport timeout is a safety cap.
            .timeout(Duration::from_millis(OUTPUT_PREDICTOR_TIMEOUT_MS * 5))
            .connect_timeout(Duration::from_secs(5))
            .keep_alive_timeout(Duration::from_secs(20))
            .keep_alive_while_idle(true);
        let channel = endpoint
            .connect()
            .await
            .map_err(|e| PredictorClientError::Connect(format!("{endpoint_url}: {e}")))?;
        Ok(Self {
            inner: Arc::new(OutputPredictorProtoClient::new(channel)),
        })
    }

    /// Predict with a hard timeout (10ms by default, spec §11.1).
    ///
    /// On timeout/network/validation error, returns `Err`; caller is
    /// responsible for falling back to local Strategy A.
    pub async fn predict_with_timeout(
        &self,
        req: PredictRequest,
        timeout: Duration,
    ) -> Result<PredictResponse, PredictorClientError> {
        let mut client = (*self.inner).clone();
        let fut = client.predict(req);
        match tokio::time::timeout(timeout, fut).await {
            Ok(Ok(resp)) => Ok(resp.into_inner()),
            Ok(Err(status)) => {
                debug!(code = ?status.code(), msg = %status.message(), "output_predictor.Predict error");
                Err(PredictorClientError::Rpc(format!(
                    "{}: {}",
                    status.code(),
                    status.message()
                )))
            }
            Err(_elapsed) => {
                warn!(
                    timeout_ms = timeout.as_millis() as u64,
                    "output_predictor.Predict timeout"
                );
                Err(PredictorClientError::Timeout(timeout.as_millis() as u64))
            }
        }
    }
}

/// SLICE_10 Phase A — thin tonic wrapper around the run_cost_projector
/// service.
///
/// Mirrors `OutputPredictorClient` shape; differences:
///   * Default hard cap is 5ms (spec §12.1 warm cache).
///   * Failure mode is pass-through (no RUN_* code), not fall-back-to-A.
#[derive(Clone)]
pub struct RunCostProjectorClient {
    inner: Arc<RunCostProjectorProtoClient<Channel>>,
}

impl RunCostProjectorClient {
    pub async fn connect(endpoint_url: String) -> Result<Self, PredictorClientError> {
        let endpoint = Endpoint::from_shared(endpoint_url.clone())
            .map_err(|e| PredictorClientError::EndpointParse(e.to_string()))?
            .timeout(Duration::from_millis(RUN_COST_PROJECTOR_TIMEOUT_MS * 5))
            .connect_timeout(Duration::from_secs(5))
            .keep_alive_timeout(Duration::from_secs(20))
            .keep_alive_while_idle(true);
        let channel = endpoint
            .connect()
            .await
            .map_err(|e| PredictorClientError::Connect(format!("{endpoint_url}: {e}")))?;
        Ok(Self {
            inner: Arc::new(RunCostProjectorProtoClient::new(channel)),
        })
    }

    pub async fn project_with_timeout(
        &self,
        req: ProjectRequest,
        timeout: Duration,
    ) -> Result<ProjectResponse, PredictorClientError> {
        let mut client = (*self.inner).clone();
        let fut = client.project(req);
        match tokio::time::timeout(timeout, fut).await {
            Ok(Ok(resp)) => Ok(resp.into_inner()),
            Ok(Err(status)) => {
                debug!(code = ?status.code(), msg = %status.message(), "run_cost_projector.Project error");
                Err(PredictorClientError::Rpc(format!(
                    "{}: {}",
                    status.code(),
                    status.message()
                )))
            }
            Err(_elapsed) => {
                warn!(
                    timeout_ms = timeout.as_millis() as u64,
                    "run_cost_projector.Project timeout"
                );
                Err(PredictorClientError::Timeout(timeout.as_millis() as u64))
            }
        }
    }

    /// Lifecycle: TerminateRun on run.end. Idempotent per spec §2.1.
    /// Not on the hot path; uses a more generous 1s timeout.
    pub async fn terminate_run(
        &self,
        req: TerminateRunRequest,
    ) -> Result<TerminateRunResponse, PredictorClientError> {
        let mut client = (*self.inner).clone();
        let fut = client.terminate_run(req);
        match tokio::time::timeout(Duration::from_secs(1), fut).await {
            Ok(Ok(resp)) => Ok(resp.into_inner()),
            Ok(Err(status)) => Err(PredictorClientError::Rpc(format!(
                "{}: {}",
                status.code(),
                status.message()
            ))),
            Err(_) => Err(PredictorClientError::Timeout(1000)),
        }
    }
}

/// SLICE_10 Phase A — boot-time configuration for the two prediction
/// gRPC clients.
///
/// Both clients are optional. When the env var is unset, the client is
/// `None` and `estimate_call_cost` falls back per spec §11 failure modes.
/// This keeps demo-mode deployments (sidecar + egress_proxy on one node,
/// no predictor sidecars) functional with degraded prediction quality.
#[derive(Debug, Clone, Default)]
pub struct PredictorClientConfig {
    pub output_predictor_endpoint: Option<String>,
    pub run_cost_projector_endpoint: Option<String>,
}

impl PredictorClientConfig {
    /// Read env vars:
    ///   SPENDGUARD_PROXY_OUTPUT_PREDICTOR_ENDPOINT — e.g. http://predictor:50051
    ///   SPENDGUARD_PROXY_RUN_COST_PROJECTOR_ENDPOINT — e.g. http://projector:50052
    pub fn from_env() -> Self {
        Self {
            output_predictor_endpoint: std::env::var(
                "SPENDGUARD_PROXY_OUTPUT_PREDICTOR_ENDPOINT",
            )
            .ok()
            .filter(|s| !s.is_empty()),
            run_cost_projector_endpoint: std::env::var(
                "SPENDGUARD_PROXY_RUN_COST_PROJECTOR_ENDPOINT",
            )
            .ok()
            .filter(|s| !s.is_empty()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_is_all_none() {
        let cfg = PredictorClientConfig::default();
        assert!(cfg.output_predictor_endpoint.is_none());
        assert!(cfg.run_cost_projector_endpoint.is_none());
    }

    #[test]
    fn config_from_env_reads_both() {
        // Restore env after the test to avoid pollution.
        let orig_op = std::env::var("SPENDGUARD_PROXY_OUTPUT_PREDICTOR_ENDPOINT").ok();
        let orig_rp = std::env::var("SPENDGUARD_PROXY_RUN_COST_PROJECTOR_ENDPOINT").ok();
        std::env::set_var(
            "SPENDGUARD_PROXY_OUTPUT_PREDICTOR_ENDPOINT",
            "http://predictor:50051",
        );
        std::env::set_var(
            "SPENDGUARD_PROXY_RUN_COST_PROJECTOR_ENDPOINT",
            "http://projector:50052",
        );
        let cfg = PredictorClientConfig::from_env();
        assert_eq!(
            cfg.output_predictor_endpoint.as_deref(),
            Some("http://predictor:50051")
        );
        assert_eq!(
            cfg.run_cost_projector_endpoint.as_deref(),
            Some("http://projector:50052")
        );
        // Restore.
        match orig_op {
            Some(v) => std::env::set_var("SPENDGUARD_PROXY_OUTPUT_PREDICTOR_ENDPOINT", v),
            None => std::env::remove_var("SPENDGUARD_PROXY_OUTPUT_PREDICTOR_ENDPOINT"),
        }
        match orig_rp {
            Some(v) => std::env::set_var("SPENDGUARD_PROXY_RUN_COST_PROJECTOR_ENDPOINT", v),
            None => std::env::remove_var("SPENDGUARD_PROXY_RUN_COST_PROJECTOR_ENDPOINT"),
        }
    }

    #[test]
    fn empty_env_treated_as_unset() {
        let orig = std::env::var("SPENDGUARD_PROXY_OUTPUT_PREDICTOR_ENDPOINT").ok();
        std::env::set_var("SPENDGUARD_PROXY_OUTPUT_PREDICTOR_ENDPOINT", "");
        let cfg = PredictorClientConfig::from_env();
        assert!(
            cfg.output_predictor_endpoint.is_none(),
            "empty string treated as unset"
        );
        match orig {
            Some(v) => std::env::set_var("SPENDGUARD_PROXY_OUTPUT_PREDICTOR_ENDPOINT", v),
            None => std::env::remove_var("SPENDGUARD_PROXY_OUTPUT_PREDICTOR_ENDPOINT"),
        }
    }
}
