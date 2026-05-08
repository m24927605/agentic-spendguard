//! Construction of `Replay` (minimal replay response, per Ledger §7).
//!
//! Minimal replay contains NO PII, NO full prompts, NO encryption keys —
//! only a status code, the ledger_transaction_id, the operation kind, the
//! audit_decision_event_id anchor, the recorded timestamp, and a compact
//! operation_id.

use prost_types::Timestamp;

use crate::proto::common::v1::{replay::StatusCode, Replay};

pub fn from_db_row(
    ledger_transaction_id: String,
    operation_kind: &str,
    audit_decision_event_id: String,
    operation_id: String,
    decision_id: String,
    projection_ids: Vec<String>,
    ttl_expires_at: Option<Timestamp>,
    recorded_at: chrono::DateTime<chrono::Utc>,
    posting_state: &str,
) -> Replay {
    let status = match posting_state {
        "posted" => StatusCode::Posted,
        "voided" => StatusCode::Voided,
        "pending" => StatusCode::Pending,
        _ => StatusCode::Unspecified,
    };
    Replay {
        ledger_transaction_id,
        operation_kind: operation_kind.to_string(),
        audit_decision_event_id,
        recorded_at: Some(Timestamp {
            seconds: recorded_at.timestamp(),
            nanos: recorded_at.timestamp_subsec_nanos() as i32,
        }),
        operation_id,
        status_code: status as i32,
        decision_id,
        projection_ids,
        ttl_expires_at,
    }
}
