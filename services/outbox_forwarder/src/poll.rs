//! Polling SQL — deterministic ordering for decision-before-outcome
//! within identical recorded_at (Codex r1 P2.5 fix).

use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// Upper bound on how long the claim transaction may wait on a row lock /
/// run its SELECT. The claim tx only holds the `FOR UPDATE SKIP LOCKED`
/// locks for the duration of this single SELECT (the tx is committed before
/// any network call), so a small timeout is sufficient and prevents a
/// pathological pile-up from ever holding the pending-forwarder index hot.
const CLAIM_LOCK_TIMEOUT_MS: i32 = 2_000;
const CLAIM_STATEMENT_TIMEOUT_MS: i32 = 5_000;

#[derive(Debug, Clone)]
pub struct OutboxRow {
    pub recorded_month: NaiveDate,
    pub audit_outbox_id: Uuid,
    pub audit_decision_event_id: Uuid,
    pub decision_id: Uuid,
    pub tenant_id: Uuid,
    pub event_type: String,
    pub cloudevent_payload: Value,
    pub cloudevent_payload_signature: Vec<u8>,
    pub producer_sequence: i64,
    pub recorded_at: DateTime<Utc>,
}

/// Claim a batch of pending rows under `FOR UPDATE SKIP LOCKED`, then commit
/// the claim transaction immediately so the row locks are released before any
/// downstream `canonical_ingest` network call.
///
/// Correctness no longer depends solely on leader election: the
/// `SKIP LOCKED` clause guarantees two pollers running concurrently (e.g. a
/// lease-handoff window, or `leader_election_mode = disabled`) never select
/// the same pending row in overlapping SELECTs, and `forward::apply_updates`
/// re-asserts `pending_forward = TRUE` so any residual race after the lock is
/// released degrades to an idempotent no-op (canonical_ingest already dedups
/// by event_id). We deliberately do NOT hold the lock-holding tx across the
/// AppendEvents RPC — holding row locks over a network call lengthens
/// lock-hold time and risks pool starvation when canonical_ingest is slow.
/// Mirrors services/control_plane/src/audit_forwarder.rs.
pub async fn fetch_pending(pool: &PgPool, batch_size: i64) -> Result<Vec<OutboxRow>, sqlx::Error> {
    let mut claim_tx = pool.begin().await?;

    // Bound the claim window defensively. The claim tx only spans this single
    // SELECT, but a tight lock/statement timeout ensures a stuck claim can
    // never sit on the pending-forwarder index indefinitely.
    sqlx::query(&format!("SET LOCAL lock_timeout = {CLAIM_LOCK_TIMEOUT_MS}"))
        .execute(&mut *claim_tx)
        .await?;
    sqlx::query(&format!(
        "SET LOCAL statement_timeout = {CLAIM_STATEMENT_TIMEOUT_MS}"
    ))
    .execute(&mut *claim_tx)
    .await?;

    let rows = sqlx::query(
        "SELECT recorded_month, audit_outbox_id, audit_decision_event_id, \
                decision_id, tenant_id, event_type, \
                cloudevent_payload, cloudevent_payload_signature, \
                producer_sequence, recorded_at \
           FROM audit_outbox \
          WHERE pending_forward = TRUE \
          ORDER BY recorded_month, \
                   recorded_at, \
                   (CASE event_type \
                        WHEN 'spendguard.audit.decision' THEN 0 \
                        ELSE 1 END), \
                   producer_sequence, \
                   audit_outbox_id \
          LIMIT $1 \
          FOR UPDATE SKIP LOCKED",
    )
    .bind(batch_size)
    .fetch_all(&mut *claim_tx)
    .await?;

    // Release the row locks before the network round-trip in forward_batch.
    claim_tx.commit().await?;

    Ok(rows
        .into_iter()
        .map(|r| OutboxRow {
            recorded_month: r.get("recorded_month"),
            audit_outbox_id: r.get("audit_outbox_id"),
            audit_decision_event_id: r.get("audit_decision_event_id"),
            decision_id: r.get("decision_id"),
            tenant_id: r.get("tenant_id"),
            event_type: r.get("event_type"),
            cloudevent_payload: r.get("cloudevent_payload"),
            cloudevent_payload_signature: r.get("cloudevent_payload_signature"),
            producer_sequence: r.get("producer_sequence"),
            recorded_at: r.get("recorded_at"),
        })
        .collect())
}
