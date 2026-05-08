//! Wrapper around the `post_provider_reported_transaction` stored
//! procedure (Phase 2B Step 8).
//!
//! See `migrations/0014_post_provider_reported_transaction.sql`. Like
//! `post_commit_estimated_transaction`, this proc is the SOLE authority
//! on the provider_report state transition + audit + projection update.

use num_bigint::BigInt;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::error::{map_pg_error, DomainError};

pub struct PostProviderReportedInput<'a> {
    pub transaction: Value,
    pub reservation_id: Uuid,
    pub provider_amount: &'a BigInt,
    pub pricing: Value,
    pub audit_outbox_row: Value,
}

pub async fn post(
    pool: &PgPool,
    input: PostProviderReportedInput<'_>,
) -> Result<Uuid, DomainError> {
    let row: (Uuid,) = sqlx::query_as(
        "SELECT post_provider_reported_transaction(\
            $1::JSONB, $2::UUID, $3::NUMERIC, $4::JSONB, $5::JSONB)",
    )
    .bind(input.transaction)
    .bind(input.reservation_id)
    .bind(input.provider_amount.to_string())
    .bind(input.pricing)
    .bind(input.audit_outbox_row)
    .fetch_one(pool)
    .await
    .map_err(map_pg_error)?;

    Ok(row.0)
}
