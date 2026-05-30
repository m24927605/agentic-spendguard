//! gRPC `RunCostProjector` service implementation.
//!
//! Phase A: skeleton with handler stubs returning `Unimplemented`.
//! Phase B: state_cache + recovery wired.
//! Phase C: signal_1 + signal_2 + signal_3 wired into layering.
//! Phase D: orchestration + validation + final response shape.
//!
//! Spec ref `run-cost-projector-spec-v1alpha1.md` §2.

use std::sync::Arc;

use tonic::{Request, Response, Status};
use tracing::{info, warn};

use crate::proto::run_cost_projector::v1::{
    run_cost_projector_server::RunCostProjector, ProjectRequest, ProjectResponse,
    TerminateRunRequest, TerminateRunResponse,
};

/// gRPC service handle.
///
/// Phase A: empty struct. Phases B-D add cache + recovery + DB pool +
/// signal computation parameters.
pub struct RunCostProjectorSvc {
    /// Phase A placeholder so the struct is non-empty (avoid trait-impl
    /// shape churn between phases). Replaced in Phase B with the real
    /// `RunStateCache` Arc.
    #[allow(dead_code)]
    pub(crate) inner: Arc<()>,
}

impl Default for RunCostProjectorSvc {
    fn default() -> Self {
        Self {
            inner: Arc::new(()),
        }
    }
}

impl RunCostProjectorSvc {
    pub fn new() -> Self {
        Self::default()
    }
}

#[tonic::async_trait]
impl RunCostProjector for RunCostProjectorSvc {
    async fn project(
        &self,
        request: Request<ProjectRequest>,
    ) -> Result<Response<ProjectResponse>, Status> {
        let req = request.into_inner();
        // Phase A: trace + return Unimplemented so callers know the wiring
        // is not yet activated. Phase D installs the real orchestration.
        warn!(
            tenant_id = %req.tenant_id,
            run_id = %req.run_id,
            agent_id = %req.agent_id,
            "Project RPC invoked but SLICE_09 Phase A skeleton — real layering wires in Phase D"
        );
        Err(Status::unimplemented(
            "run_cost_projector Project handler is a SLICE_09 Phase A skeleton; \
             Phase D wires Signal 1/2/3 layering",
        ))
    }

    async fn terminate_run(
        &self,
        request: Request<TerminateRunRequest>,
    ) -> Result<Response<TerminateRunResponse>, Status> {
        let req = request.into_inner();
        info!(
            tenant_id = %req.tenant_id,
            run_id = %req.run_id,
            reason = %req.reason,
            "TerminateRun received (Phase A skeleton — idempotent no-op until Phase D)"
        );
        Ok(Response::new(TerminateRunResponse {
            removed_from_cache: false,
        }))
    }
}
