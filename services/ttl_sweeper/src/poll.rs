//! Polling query for expired reservations (Codex TTL r2 §Δ2).
//!
//! NO `FOR UPDATE` — Codex TTL r1 P1.1 fix: holding a row lock across
//! the ledger gRPC call would deadlock the SP, which itself acquires
//! `FOR UPDATE` on the same row at step 6. Instead rely on SP's CAS
//! (current_state='reserved') for race correctness.

use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ExpiredRow {
    pub reservation_id: Uuid,
    pub tenant_id: Uuid,
    pub source_ledger_transaction_id: Uuid,
    pub original_decision_id: Uuid,
    pub ttl_expires_at: chrono::DateTime<chrono::Utc>,
}

pub async fn fetch_expired(
    pool: &PgPool,
    tenant_id: Uuid,
    batch_size: i64,
) -> Result<Vec<ExpiredRow>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT r.reservation_id,
                r.tenant_id,
                r.source_ledger_transaction_id,
                r.ttl_expires_at,
                lt.decision_id AS original_decision_id
           FROM reservations r
           JOIN ledger_transactions lt
             ON lt.ledger_transaction_id = r.source_ledger_transaction_id
            AND lt.tenant_id = r.tenant_id
          WHERE r.tenant_id = $1
            AND r.current_state = 'reserved'
            AND r.ttl_expires_at <= NOW()
          ORDER BY r.ttl_expires_at
          LIMIT $2",
    )
    .bind(tenant_id)
    .bind(batch_size)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ExpiredRow {
            reservation_id: r.get("reservation_id"),
            tenant_id: r.get("tenant_id"),
            source_ledger_transaction_id: r.get("source_ledger_transaction_id"),
            original_decision_id: r.get("original_decision_id"),
            ttl_expires_at: r.get("ttl_expires_at"),
        })
        .collect())
}
