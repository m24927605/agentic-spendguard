//! Wrapper around the `post_commit_estimated_transaction` stored procedure
//! (Phase 2B Step 7).
//!
//! See `migrations/0013_post_commit_estimated_transaction.sql`. Like
//! `post_ledger_transaction`, this proc is the SOLE authority on durable
//! commit-lifecycle writes; callers MUST NOT pre-validate or pre-derive
//! entries. The handler passes only identifiers + the requested
//! estimated_amount + sanity-check pricing.

use num_bigint::BigInt;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::error::{map_pg_error, DomainError};

pub struct PostCommitEstimatedInput<'a> {
    pub transaction: Value,
    pub reservation_id: Uuid,
    pub estimated_amount: &'a BigInt,
    pub pricing: Value,
    pub audit_outbox_row: Value,
}

pub async fn post(
    pool: &PgPool,
    input: PostCommitEstimatedInput<'_>,
) -> Result<Uuid, DomainError> {
    let row: (Uuid,) = sqlx::query_as(
        "SELECT post_commit_estimated_transaction(\
            $1::JSONB, $2::UUID, $3::NUMERIC, $4::JSONB, $5::JSONB)",
    )
    .bind(input.transaction)
    .bind(input.reservation_id)
    .bind(input.estimated_amount.to_string())
    .bind(input.pricing)
    .bind(input.audit_outbox_row)
    .fetch_one(pool)
    .await
    .map_err(map_pg_error)?;

    Ok(row.0)
}
