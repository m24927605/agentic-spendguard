//! Wrapper around the `post_ledger_transaction` stored procedure.
//!
//! See `migrations/0012_post_ledger_transaction.sql`. The procedure does
//! all server-side derivation, fencing CAS, lock-order canonicalization,
//! per-unit balance check, and audit_outbox row insert in a single
//! Postgres transaction. We invoke it via `sqlx::query`.

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::error::{map_pg_error, DomainError};

pub struct PostTransactionInput<'a> {
    pub transaction: Value,
    pub entries: Value,
    /// Optional reservations to persist into the projection. Pass `Value::Null`
    /// for operations that don't create reservations (Release / Commit / etc.).
    pub reservations: Value,
    pub audit_outbox_row: Value,
    pub caller_lock_token: Option<&'a str>,
}

/// Invoke `post_ledger_transaction(...)`; returns the resulting
/// ledger_transaction_id. Stored proc is the SOLE authority on idempotent
/// replay: callers MUST NOT pre-check `ledger_transactions`.
pub async fn post(pool: &PgPool, input: PostTransactionInput<'_>) -> Result<Uuid, DomainError> {
    let row: (Uuid,) = sqlx::query_as(
        "SELECT post_ledger_transaction($1::JSONB, $2::JSONB, $3::JSONB, $4::JSONB, $5)",
    )
    .bind(input.transaction)
    .bind(input.entries)
    .bind(input.reservations)
    .bind(input.audit_outbox_row)
    .bind(input.caller_lock_token)
    .fetch_one(pool)
    .await
    .map_err(map_pg_error)?;

    Ok(row.0)
}

/// Look up an idempotent replay row.
///
/// Returned tuple: (ledger_transaction_id, operation_kind, audit_decision_event_id,
///                  posting_state, recorded_at).
/// Returns None if no match.
pub async fn lookup_idempotent(
    pool: &PgPool,
    tenant_id: Uuid,
    operation_kind: &str,
    idempotency_key: &str,
) -> Result<
    Option<(
        Uuid,
        String,
        Option<Uuid>,
        String,
        chrono::DateTime<chrono::Utc>,
    )>,
    DomainError,
> {
    let row = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            Option<Uuid>,
            String,
            chrono::DateTime<chrono::Utc>,
        ),
    >(
        "SELECT ledger_transaction_id, operation_kind, audit_decision_event_id,
                posting_state, recorded_at
         FROM ledger_transactions
         WHERE tenant_id = $1
           AND operation_kind = $2
           AND idempotency_key = $3",
    )
    .bind(tenant_id)
    .bind(operation_kind)
    .bind(idempotency_key)
    .fetch_optional(pool)
    .await
    .map_err(map_pg_error)?;
    Ok(row)
}
