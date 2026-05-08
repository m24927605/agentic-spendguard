//! Wrapper around the `post_release_transaction` stored procedure
//! (Phase 2B Step 7.5).
//!
//! See `migrations/0015_post_release_transaction.sql`. Like Step 7's
//! commit_estimated SP, this proc is the SOLE authority on durable
//! release-lifecycle writes; callers MUST NOT pre-validate or pre-derive
//! entries. Server-derived from decision_id (no caller pricing or
//! source_ledger_transaction_id).

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::error::{map_pg_error, DomainError};

pub struct PostReleaseInput<'a> {
    pub transaction: Value,
    pub reservation_set_id: Uuid,
    pub reason: &'a str,
    pub audit_outbox_row: Value,
}

pub async fn post(
    pool: &PgPool,
    input: PostReleaseInput<'_>,
) -> Result<Uuid, DomainError> {
    let row: (Uuid,) = sqlx::query_as(
        "SELECT post_release_transaction(\
            $1::JSONB, $2::UUID, $3::TEXT, $4::JSONB)",
    )
    .bind(input.transaction)
    .bind(input.reservation_set_id)
    .bind(input.reason)
    .bind(input.audit_outbox_row)
    .fetch_one(pool)
    .await
    .map_err(map_pg_error)?;

    Ok(row.0)
}
