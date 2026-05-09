//! Polling SQL — deterministic ordering for decision-before-outcome
//! within identical recorded_at (Codex r1 P2.5 fix).

use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

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

pub async fn fetch_pending(pool: &PgPool, batch_size: i64) -> Result<Vec<OutboxRow>, sqlx::Error> {
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
          LIMIT $1",
    )
    .bind(batch_size)
    .fetch_all(pool)
    .await?;

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
