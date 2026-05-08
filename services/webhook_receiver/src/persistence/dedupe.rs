//! webhook_dedupe queries (services/ledger/migrations/0017_webhook_dedupe.sql).
//!
//! Co-located in the ledger DB but NOT atomic with ledger transactions.
//! Best-effort replay cache for idempotent provider event delivery.

use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct DedupeRow {
    pub canonical_hash: Vec<u8>,
    pub ledger_transaction_id: Uuid,
}

pub async fn lookup(
    pool: &PgPool,
    provider: &str,
    event_kind: &str,
    provider_account: &str,
    provider_event_id: &str,
) -> Result<Option<DedupeRow>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT canonical_hash, ledger_transaction_id \
           FROM webhook_dedupe \
          WHERE provider = $1 AND event_kind = $2 \
            AND provider_account = $3 AND provider_event_id = $4",
    )
    .bind(provider)
    .bind(event_kind)
    .bind(provider_account)
    .bind(provider_event_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| DedupeRow {
        canonical_hash: r.get("canonical_hash"),
        ledger_transaction_id: r.get("ledger_transaction_id"),
    }))
}

/// Insert dedupe row. Returns true if inserted, false on conflict.
#[allow(clippy::too_many_arguments)]
pub async fn insert(
    pool: &PgPool,
    provider: &str,
    event_kind: &str,
    provider_account: &str,
    provider_event_id: &str,
    canonical_hash: &[u8],
    ledger_transaction_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO webhook_dedupe \
            (provider, event_kind, provider_account, provider_event_id, \
             canonical_hash, ledger_transaction_id) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         ON CONFLICT (provider, event_kind, provider_account, provider_event_id) \
         DO NOTHING",
    )
    .bind(provider)
    .bind(event_kind)
    .bind(provider_account)
    .bind(provider_event_id)
    .bind(canonical_hash)
    .bind(ledger_transaction_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Look up an existing ledger_transactions row by idempotency key.
/// Used for the rare race where dedupe row hasn't been inserted yet but
/// ledger has already collapsed via its own idempotency replay.
pub async fn lookup_ledger_tx(
    pool: &PgPool,
    tenant_id: Uuid,
    operation_kind: &str,
    idempotency_key: &str,
) -> Result<Option<(Uuid, Vec<u8>)>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT ledger_transaction_id, request_hash \
           FROM ledger_transactions \
          WHERE tenant_id = $1 \
            AND operation_kind = $2 \
            AND idempotency_key = $3",
    )
    .bind(tenant_id)
    .bind(operation_kind)
    .bind(idempotency_key)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| {
        let id: Uuid = r.get("ledger_transaction_id");
        let hash: Vec<u8> = r.get("request_hash");
        (id, hash)
    }))
}
