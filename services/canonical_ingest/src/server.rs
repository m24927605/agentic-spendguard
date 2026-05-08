use sqlx::PgPool;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::{
    config::Config,
    handlers,
    proto::canonical_ingest::v1::{
        canonical_ingest_server::CanonicalIngest, AppendEventsRequest, AppendEventsResponse,
        AuditChainEvent, QueryAuditChainRequest, VerifySchemaBundleRequest,
        VerifySchemaBundleResponse,
    },
};

pub struct CanonicalIngestService {
    pool: PgPool,
    cfg: Config,
}

impl CanonicalIngestService {
    pub fn new(pool: PgPool, cfg: Config) -> Self {
        Self { pool, cfg }
    }
}

#[tonic::async_trait]
impl CanonicalIngest for CanonicalIngestService {
    async fn append_events(
        &self,
        req: Request<AppendEventsRequest>,
    ) -> Result<Response<AppendEventsResponse>, Status> {
        let resp = handlers::append_events::handle(&self.pool, &self.cfg, req.into_inner()).await?;
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
