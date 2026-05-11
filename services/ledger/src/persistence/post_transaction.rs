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

// ---------------------------------------------------------------------------
// Phase 3 wedge: post_denied_decision_transaction
// ---------------------------------------------------------------------------

pub struct PostDeniedInput {
    pub transaction: Value,
    pub audit_outbox_row: Value,
}

/// Invoke `post_denied_decision_transaction(...)`; returns the resulting
/// ledger_transaction_id. SP is the sole authority on idempotent replay.
pub async fn post_denied(pool: &PgPool, input: PostDeniedInput) -> Result<Uuid, DomainError> {
    let row: (Uuid,) = sqlx::query_as(
        "SELECT post_denied_decision_transaction($1::JSONB, $2::JSONB)",
    )
    .bind(input.transaction)
    .bind(input.audit_outbox_row)
    .fetch_one(pool)
    .await
    .map_err(map_pg_error)?;
    Ok(row.0)
}

// ---------------------------------------------------------------------------
// Round-2 #9 producer SP: post_approval_required_decision
// ---------------------------------------------------------------------------

pub struct PostApprovalRequiredInput {
    pub transaction: Value,
    pub audit_outbox_row: Value,
    pub decision_context: Value,
    pub requested_effect: Value,
    pub approval_ttl_seconds: i32,
}

pub struct PostApprovalRequiredOutput {
    pub ledger_transaction_id: Uuid,
    pub approval_id: Uuid,
    pub was_first_insert: bool,
}

/// Invoke `post_approval_required_decision(...)` SP (migration 0037).
/// Wraps `post_denied_decision_transaction` + writes an
/// `approval_requests` row atomically. Idempotent on
/// (tenant_id, decision_id) — replays return the existing
/// approval_id with `was_first_insert = false`.
pub async fn post_approval_required(
    pool: &PgPool,
    input: PostApprovalRequiredInput,
) -> Result<PostApprovalRequiredOutput, DomainError> {
    let row: (Uuid, Uuid, bool) = sqlx::query_as(
        "SELECT ledger_transaction_id, approval_id, was_first_insert
         FROM post_approval_required_decision(
             $1::JSONB, $2::JSONB, $3::JSONB, $4::JSONB, $5::INT
         )",
    )
    .bind(input.transaction)
    .bind(input.audit_outbox_row)
    .bind(input.decision_context)
    .bind(input.requested_effect)
    .bind(input.approval_ttl_seconds)
    .fetch_one(pool)
    .await
    .map_err(map_pg_error)?;
    Ok(PostApprovalRequiredOutput {
        ledger_transaction_id: row.0,
        approval_id: row.1,
        was_first_insert: row.2,
    })
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
