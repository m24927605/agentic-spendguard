//! `Ledger` gRPC service implementation.
//!
//! Maps proto-defined RPCs to domain handlers. Phase 2B Step 1 implements
//! ReserveSet end-to-end; Release / Commit* / Refund / Dispute / Compensate
//! are stubbed (return Unimplemented). Replay + QueryDecisionOutcome are
//! fully implemented because they're required by sidecar crash recovery
//! testing (per Stage 2 §4.5).

use sqlx::PgPool;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::{
    handlers,
    proto::ledger::v1::{
        ledger_server::Ledger, CommitEstimatedRequest, CommitEstimatedResponse,
        CompensateRequest, CompensateResponse, DisputeAdjustmentRequest,
        DisputeAdjustmentResponse, InvoiceReconcileRequest, InvoiceReconcileResponse,
        ProviderReportRequest, ProviderReportResponse, QueryBudgetStateRequest,
        QueryBudgetStateResponse, QueryDecisionOutcomeRequest, QueryDecisionOutcomeResponse,
        QueryReservationContextRequest, QueryReservationContextResponse, RefundCreditRequest,
        RefundCreditResponse, ReleaseRequest, ReleaseResponse, ReplayAuditEvent,
        ReplayAuditFromCursorRequest, ReserveSetRequest, ReserveSetResponse,
    },
};

pub struct LedgerService {
    pool: PgPool,
}

impl LedgerService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[tonic::async_trait]
impl Ledger for LedgerService {
    async fn reserve_set(
        &self,
        req: Request<ReserveSetRequest>,
    ) -> Result<Response<ReserveSetResponse>, Status> {
        let resp = handlers::reserve_set::handle(&self.pool, req.into_inner()).await?;
        Ok(Response::new(resp))
    }

    async fn release(
        &self,
        req: Request<ReleaseRequest>,
    ) -> Result<Response<ReleaseResponse>, Status> {
        // Step 7.5 implemented: handler returns ReleaseSuccess / Replay /
        // typed Error for sidecar-originated release lane.
        let resp = handlers::release::handle(&self.pool, req.into_inner()).await?;
        Ok(Response::new(resp))
    }

    async fn commit_estimated(
        &self,
        req: Request<CommitEstimatedRequest>,
    ) -> Result<Response<CommitEstimatedResponse>, Status> {
        let resp = handlers::commit_estimated::handle(&self.pool, req.into_inner()).await?;
        Ok(Response::new(resp))
    }

    async fn provider_report(
        &self,
        req: Request<ProviderReportRequest>,
    ) -> Result<Response<ProviderReportResponse>, Status> {
        let resp = handlers::provider_report::handle(&self.pool, req.into_inner()).await?;
        Ok(Response::new(resp))
    }

    async fn invoice_reconcile(
        &self,
        req: Request<InvoiceReconcileRequest>,
    ) -> Result<Response<InvoiceReconcileResponse>, Status> {
        let resp = handlers::invoice_reconcile::handle(&self.pool, req.into_inner()).await?;
        Ok(Response::new(resp))
    }

    async fn refund_credit(
        &self,
        _req: Request<RefundCreditRequest>,
    ) -> Result<Response<RefundCreditResponse>, Status> {
        Err(Status::unimplemented("RefundCredit: vertical slice expansion in progress"))
    }

    async fn dispute_adjustment(
        &self,
        _req: Request<DisputeAdjustmentRequest>,
    ) -> Result<Response<DisputeAdjustmentResponse>, Status> {
        Err(Status::unimplemented("DisputeAdjustment: vertical slice expansion in progress"))
    }

    async fn compensate(
        &self,
        _req: Request<CompensateRequest>,
    ) -> Result<Response<CompensateResponse>, Status> {
        Err(Status::unimplemented("Compensate: vertical slice expansion in progress"))
    }

    async fn query_budget_state(
        &self,
        req: Request<QueryBudgetStateRequest>,
    ) -> Result<Response<QueryBudgetStateResponse>, Status> {
        let resp = handlers::query_budget_state::handle(&self.pool, req.into_inner()).await?;
        Ok(Response::new(resp))
    }

    async fn query_reservation_context(
        &self,
        req: Request<QueryReservationContextRequest>,
    ) -> Result<Response<QueryReservationContextResponse>, Status> {
        let resp = handlers::query_reservation_context::handle(&self.pool, req.into_inner()).await?;
        Ok(Response::new(resp))
    }

    type ReplayAuditFromCursorStream = ReceiverStream<Result<ReplayAuditEvent, Status>>;

    async fn replay_audit_from_cursor(
        &self,
        req: Request<ReplayAuditFromCursorRequest>,
    ) -> Result<Response<Self::ReplayAuditFromCursorStream>, Status> {
        handlers::replay::replay_stream(self.pool.clone(), req.into_inner()).await
    }

    async fn query_decision_outcome(
        &self,
        req: Request<QueryDecisionOutcomeRequest>,
    ) -> Result<Response<QueryDecisionOutcomeResponse>, Status> {
        let resp = handlers::replay::query_decision_outcome(&self.pool, req.into_inner()).await?;
        Ok(Response::new(resp))
    }
}
