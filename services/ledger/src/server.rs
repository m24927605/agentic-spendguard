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
    metrics::{Handler, LedgerMetrics, Outcome},
    proto::ledger::v1::{
        ledger_server::Ledger, AcquireFencingLeaseRequest, AcquireFencingLeaseResponse,
        CommitEstimatedRequest, CommitEstimatedResponse,
        CompensateRequest, CompensateResponse, DisputeAdjustmentRequest,
        DisputeAdjustmentResponse, GetApprovalForResumeRequest,
        GetApprovalForResumeResponse, InvoiceReconcileRequest, InvoiceReconcileResponse,
        MarkApprovalBundledRequest, MarkApprovalBundledResponse, ProviderReportRequest,
        ProviderReportResponse, QueryBudgetStateRequest, QueryBudgetStateResponse,
        QueryDecisionOutcomeRequest, QueryDecisionOutcomeResponse,
        QueryReservationContextRequest, QueryReservationContextResponse,
        RecordDeniedDecisionRequest, RecordDeniedDecisionResponse, RefundCreditRequest,
        RefundCreditResponse, ReleaseRequest, ReleaseResponse, ReplayAuditEvent,
        ReplayAuditFromCursorRequest, ReserveSetRequest, ReserveSetResponse,
    },
};

pub struct LedgerService {
    pub pool: PgPool,
    /// Phase 5 GA hardening S6: producer signer for server-minted
    /// audit rows. Currently used only by InvoiceReconcile (which
    /// synthesizes a decision row that has no client-side originator).
    pub signer: std::sync::Arc<dyn spendguard_signing::Signer>,
    /// Followup #11: Prometheus counters per gRPC method.
    pub metrics: LedgerMetrics,
}

impl LedgerService {
    pub fn new(pool: PgPool, signer: std::sync::Arc<dyn spendguard_signing::Signer>) -> Self {
        Self {
            pool,
            signer,
            metrics: LedgerMetrics::new(),
        }
    }

    pub fn with_metrics(
        pool: PgPool,
        signer: std::sync::Arc<dyn spendguard_signing::Signer>,
        metrics: LedgerMetrics,
    ) -> Self {
        Self {
            pool,
            signer,
            metrics,
        }
    }
}

/// Helper that increments the right (handler, outcome) bucket based on
/// the result returned by the wrapped handler call.
fn record_outcome<T>(metrics: &LedgerMetrics, handler: Handler, r: &Result<T, Status>) {
    let outcome = if r.is_ok() { Outcome::Ok } else { Outcome::Err };
    metrics.inc_handler(handler, outcome);
}

#[tonic::async_trait]
impl Ledger for LedgerService {
    async fn reserve_set(
        &self,
        req: Request<ReserveSetRequest>,
    ) -> Result<Response<ReserveSetResponse>, Status> {
        let result = handlers::reserve_set::handle(&self.pool, req.into_inner()).await;
        record_outcome(&self.metrics, Handler::ReserveSet, &result);
        result.map(Response::new)
    }

    async fn release(
        &self,
        req: Request<ReleaseRequest>,
    ) -> Result<Response<ReleaseResponse>, Status> {
        // Step 7.5 implemented: handler returns ReleaseSuccess / Replay /
        // typed Error for sidecar-originated release lane.
        let result = handlers::release::handle(&self.pool, req.into_inner()).await;
        record_outcome(&self.metrics, Handler::Release, &result);
        result.map(Response::new)
    }

    async fn record_denied_decision(
        &self,
        req: Request<RecordDeniedDecisionRequest>,
    ) -> Result<Response<RecordDeniedDecisionResponse>, Status> {
        let result = handlers::record_denied_decision::handle(&self.pool, req.into_inner()).await;
        record_outcome(&self.metrics, Handler::RecordDeniedDecision, &result);
        result.map(Response::new)
    }

    async fn acquire_fencing_lease(
        &self,
        req: Request<AcquireFencingLeaseRequest>,
    ) -> Result<Response<AcquireFencingLeaseResponse>, Status> {
        let result =
            handlers::acquire_fencing_lease::handle(&self.pool, req.into_inner()).await;
        record_outcome(&self.metrics, Handler::AcquireFencingLease, &result);
        result.map(Response::new)
    }

    async fn commit_estimated(
        &self,
        req: Request<CommitEstimatedRequest>,
    ) -> Result<Response<CommitEstimatedResponse>, Status> {
        let result = handlers::commit_estimated::handle(&self.pool, req.into_inner()).await;
        record_outcome(&self.metrics, Handler::CommitEstimated, &result);
        result.map(Response::new)
    }

    async fn provider_report(
        &self,
        req: Request<ProviderReportRequest>,
    ) -> Result<Response<ProviderReportResponse>, Status> {
        let result = handlers::provider_report::handle(&self.pool, req.into_inner()).await;
        record_outcome(&self.metrics, Handler::ProviderReport, &result);
        result.map(Response::new)
    }

    async fn invoice_reconcile(
        &self,
        req: Request<InvoiceReconcileRequest>,
    ) -> Result<Response<InvoiceReconcileResponse>, Status> {
        let result =
            handlers::invoice_reconcile::handle(&self.pool, &*self.signer, req.into_inner())
                .await;
        record_outcome(&self.metrics, Handler::InvoiceReconcile, &result);
        result.map(Response::new)
    }

    async fn refund_credit(
        &self,
        _req: Request<RefundCreditRequest>,
    ) -> Result<Response<RefundCreditResponse>, Status> {
        self.metrics.inc_handler(Handler::RefundCredit, Outcome::Err);
        Err(Status::unimplemented("RefundCredit: vertical slice expansion in progress"))
    }

    async fn dispute_adjustment(
        &self,
        _req: Request<DisputeAdjustmentRequest>,
    ) -> Result<Response<DisputeAdjustmentResponse>, Status> {
        self.metrics.inc_handler(Handler::DisputeAdjustment, Outcome::Err);
        Err(Status::unimplemented("DisputeAdjustment: vertical slice expansion in progress"))
    }

    async fn compensate(
        &self,
        _req: Request<CompensateRequest>,
    ) -> Result<Response<CompensateResponse>, Status> {
        self.metrics.inc_handler(Handler::Compensate, Outcome::Err);
        Err(Status::unimplemented("Compensate: vertical slice expansion in progress"))
    }

    async fn query_budget_state(
        &self,
        req: Request<QueryBudgetStateRequest>,
    ) -> Result<Response<QueryBudgetStateResponse>, Status> {
        let result = handlers::query_budget_state::handle(&self.pool, req.into_inner()).await;
        record_outcome(&self.metrics, Handler::QueryBudgetState, &result);
        result.map(Response::new)
    }

    async fn query_reservation_context(
        &self,
        req: Request<QueryReservationContextRequest>,
    ) -> Result<Response<QueryReservationContextResponse>, Status> {
        let result = handlers::query_reservation_context::handle(&self.pool, req.into_inner()).await;
        record_outcome(&self.metrics, Handler::QueryReservationContext, &result);
        result.map(Response::new)
    }

    type ReplayAuditFromCursorStream = ReceiverStream<Result<ReplayAuditEvent, Status>>;

    async fn replay_audit_from_cursor(
        &self,
        req: Request<ReplayAuditFromCursorRequest>,
    ) -> Result<Response<Self::ReplayAuditFromCursorStream>, Status> {
        let result =
            handlers::replay::replay_stream(self.pool.clone(), req.into_inner()).await;
        record_outcome(&self.metrics, Handler::ReplayAuditFromCursor, &result);
        result
    }

    async fn query_decision_outcome(
        &self,
        req: Request<QueryDecisionOutcomeRequest>,
    ) -> Result<Response<QueryDecisionOutcomeResponse>, Status> {
        let result = handlers::replay::query_decision_outcome(&self.pool, req.into_inner()).await;
        record_outcome(&self.metrics, Handler::QueryDecisionOutcome, &result);
        result.map(Response::new)
    }

    async fn get_approval_for_resume(
        &self,
        req: Request<GetApprovalForResumeRequest>,
    ) -> Result<Response<GetApprovalForResumeResponse>, Status> {
        let result =
            handlers::get_approval_for_resume::handle(&self.pool, req.into_inner()).await;
        record_outcome(&self.metrics, Handler::GetApprovalForResume, &result);
        result.map(Response::new)
    }

    async fn mark_approval_bundled(
        &self,
        req: Request<MarkApprovalBundledRequest>,
    ) -> Result<Response<MarkApprovalBundledResponse>, Status> {
        let result =
            handlers::mark_approval_bundled::handle(&self.pool, req.into_inner()).await;
        record_outcome(&self.metrics, Handler::MarkApprovalBundled, &result);
        result.map(Response::new)
    }
}
