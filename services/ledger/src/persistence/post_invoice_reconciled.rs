//! Wrapper around the `post_invoice_reconciled_transaction` stored
//! procedure (Phase 2B Step 9).
//!
//! See `migrations/0016_post_invoice_reconciled_transaction.sql`. The
//! SP is the SOLE authority on the invoice_reconcile state transition +
//! dual audit (decision + outcome) + projection update.

use num_bigint::BigInt;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::error::{map_pg_error, DomainError};

pub struct PostInvoiceReconciledInput<'a> {
    pub transaction: Value,
    pub reservation_id: Uuid,
    pub invoice_amount: &'a BigInt,
    pub pricing: Value,
    pub audit_decision_outbox_row: Value,
    pub audit_outcome_outbox_row: Value,
    pub outcome_producer_seq: i64,
}

pub async fn post(
    pool: &PgPool,
    input: PostInvoiceReconciledInput<'_>,
) -> Result<Uuid, DomainError> {
    let row: (Uuid,) = sqlx::query_as(
        "SELECT post_invoice_reconciled_transaction(\
            $1::JSONB, $2::UUID, $3::NUMERIC, $4::JSONB, $5::JSONB, $6::JSONB, $7::BIGINT)",
    )
    .bind(input.transaction)
    .bind(input.reservation_id)
    .bind(input.invoice_amount.to_string())
    .bind(input.pricing)
    .bind(input.audit_decision_outbox_row)
    .bind(input.audit_outcome_outbox_row)
    .bind(input.outcome_producer_seq)
    .fetch_one(pool)
    .await
    .map_err(map_pg_error)?;

    Ok(row.0)
}
