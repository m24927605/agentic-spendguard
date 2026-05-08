//! `CanonicalIngest::VerifySchemaBundle` handler.

use sqlx::PgPool;
use tonic::Status;
use uuid::Uuid;

use crate::{
    domain::error::DomainError,
    persistence::schema_bundle,
    proto::{
        canonical_ingest::v1::{
            verify_schema_bundle_response::Status as VerifyStatus, VerifySchemaBundleRequest,
            VerifySchemaBundleResponse,
        },
        common::v1::Error as ProtoError,
    },
};

pub async fn handle(
    pool: &PgPool,
    req: VerifySchemaBundleRequest,
) -> Result<VerifySchemaBundleResponse, Status> {
    let bundle = req
        .schema_bundle
        .ok_or_else(|| Status::invalid_argument("schema_bundle required"))?;
    let bundle_id = Uuid::parse_str(&bundle.schema_bundle_id)
        .map_err(|e| Status::invalid_argument(format!("schema_bundle_id: {}", e)))?;

    match schema_bundle::lookup(pool, bundle_id, &bundle.schema_bundle_hash).await {
        Ok(Some(_)) => Ok(VerifySchemaBundleResponse {
            status: VerifyStatus::Known as i32,
            error: None,
        }),
        Ok(None) => Ok(VerifySchemaBundleResponse {
            status: VerifyStatus::Unknown as i32,
            error: None,
        }),
        Err(DomainError::SchemaBundleHashMismatch(msg)) => Ok(VerifySchemaBundleResponse {
            status: VerifyStatus::HashMismatch as i32,
            error: Some(ProtoError {
                code: 0,
                message: msg,
                details: Default::default(),
            }),
        }),
        Err(e) => Err(e.to_status()),
    }
}
