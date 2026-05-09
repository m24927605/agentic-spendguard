use sqlx::PgPool;
use std::sync::Arc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::{
    config::Config,
    handlers,
    metrics::IngestMetrics,
    proto::canonical_ingest::v1::{
        canonical_ingest_server::CanonicalIngest, AppendEventsRequest, AppendEventsResponse,
        AuditChainEvent, QueryAuditChainRequest, VerifySchemaBundleRequest,
        VerifySchemaBundleResponse,
    },
};

pub struct CanonicalIngestService {
    pool: PgPool,
    cfg: Config,
    /// Phase 5 GA hardening S8: producer signature verifier. Built at
    /// startup; `None` only when strict_signatures=false AND no trust
    /// store dir configured (non-strict pure POC mode).
    verifier: Option<Arc<dyn spendguard_signing::Verifier>>,
    metrics: IngestMetrics,
}

impl CanonicalIngestService {
    pub fn new(
        pool: PgPool,
        cfg: Config,
        verifier: Option<Arc<dyn spendguard_signing::Verifier>>,
        metrics: IngestMetrics,
    ) -> Self {
        Self {
            pool,
            cfg,
            verifier,
            metrics,
        }
    }
}

#[tonic::async_trait]
impl CanonicalIngest for CanonicalIngestService {
    async fn append_events(
        &self,
        req: Request<AppendEventsRequest>,
    ) -> Result<Response<AppendEventsResponse>, Status> {
        let resp = handlers::append_events::handle(
            &self.pool,
            &self.cfg,
            self.verifier.as_deref(),
            &self.metrics,
            req.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
    }

    async fn verify_schema_bundle(
        &self,
        req: Request<VerifySchemaBundleRequest>,
    ) -> Result<Response<VerifySchemaBundleResponse>, Status> {
        let resp = handlers::verify_schema_bundle::handle(&self.pool, req.into_inner()).await?;
        Ok(Response::new(resp))
    }

    type QueryAuditChainStream = ReceiverStream<Result<AuditChainEvent, Status>>;

    async fn query_audit_chain(
        &self,
        req: Request<QueryAuditChainRequest>,
    ) -> Result<Response<Self::QueryAuditChainStream>, Status> {
        handlers::query_audit_chain::handle(self.pool.clone(), req.into_inner()).await
    }
}
