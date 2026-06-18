//! ReplayAuditFromCursor query (per Stage 2 §4.7).

use base64::Engine as _;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::error::{map_pg_error, DomainError};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AuditOutboxRow {
    pub audit_outbox_id: Uuid,
    pub audit_decision_event_id: Uuid,
    pub decision_id: Uuid,
    pub ledger_transaction_id: Uuid,
    pub event_type: String,
    pub cloudevent_payload: serde_json::Value,
    pub cloudevent_payload_signature: Vec<u8>,
    pub ledger_fencing_epoch: i64,
    pub workload_instance_id: String,
    pub pending_forward: bool,
    pub forwarded_at: Option<chrono::DateTime<chrono::Utc>>,
    pub recorded_at: chrono::DateTime<chrono::Utc>,
    pub producer_sequence: i64,
}

/// Verify caller fencing epoch matches current; return rows after cursor.
///
/// Per Sidecar §9 + Stage 2 §4.7: replay only allowed when caller holds the
/// current fencing scope lease. Stale callers receive FENCING_EPOCH_STALE.
///
/// Caller MAY supply `fencing_scope_id` to disambiguate when the workload
/// owns multiple scopes (recommended for production). When omitted, server
/// requires that exactly one live scope matches (tenant, workload); ANY
/// multi-scope match is rejected with FENCING_EPOCH_STALE because the
/// replay cursor belongs to a specific scope and cannot be safely shared.
///
/// Returned rows have `ledger_fencing_epoch <= caller_epoch`. With our
/// fencing CAS no row can ever be written at a future epoch; the constraint
/// is defense-in-depth against schema drift / direct-SQL writes.
pub async fn replay_audit_from_cursor(
    pool: &PgPool,
    tenant_id: Uuid,
    workload_instance_id: &str,
    fencing_epoch: i64,
    fencing_scope_id: Option<Uuid>,
    producer_sequence_after: i64,
    limit: u32,
) -> Result<Vec<AuditOutboxRow>, DomainError> {
    // 1) Resolve scope; verify caller's epoch.
    if let Some(scope_id) = fencing_scope_id {
        let row = sqlx::query_as::<_, (i64,)>(
            "SELECT current_epoch
               FROM fencing_scopes
              WHERE fencing_scope_id = $1
                AND tenant_id = $2
                AND active_owner_instance_id = $3
                AND (ttl_expires_at IS NULL OR ttl_expires_at > now())",
        )
        .bind(scope_id)
        .bind(tenant_id)
        .bind(workload_instance_id)
        .fetch_optional(pool)
        .await
        .map_err(map_pg_error)?;
        match row {
            Some((e,)) if e == fencing_epoch => {}
            Some((e,)) => {
                return Err(DomainError::FencingEpochStale(format!(
                    "caller={}, scope={} current={}",
                    fencing_epoch, scope_id, e
                )))
            }
            None => {
                return Err(DomainError::FencingEpochStale(format!(
                    "no live fencing scope {} owned by workload {}",
                    scope_id, workload_instance_id
                )))
            }
        }
    } else {
        let rows: Vec<(Uuid, i64)> = sqlx::query_as(
            "SELECT fencing_scope_id, current_epoch
               FROM fencing_scopes
              WHERE tenant_id = $1
                AND active_owner_instance_id = $2
                AND scope_type IN ('control_plane_writer', 'budget_window', 'reservation')
                AND (ttl_expires_at IS NULL OR ttl_expires_at > now())",
        )
        .bind(tenant_id)
        .bind(workload_instance_id)
        .fetch_all(pool)
        .await
        .map_err(map_pg_error)?;

        if rows.is_empty() {
            return Err(DomainError::FencingEpochStale(format!(
                "no live fencing scope owned by workload {}",
                workload_instance_id
            )));
        }
        // Multi-scope workloads MUST supply fencing_scope_id to disambiguate,
        // regardless of whether epochs happen to coincide today: the cursor
        // belongs to a specific scope and replays must be scoped accordingly.
        if rows.len() > 1 {
            return Err(DomainError::FencingEpochStale(format!(
                "workload {} owns {} live scopes; caller MUST supply \
                 fencing_scope_id to disambiguate",
                workload_instance_id,
                rows.len()
            )));
        }
        let scope_epoch = rows[0].1;
        if scope_epoch != fencing_epoch {
            return Err(DomainError::FencingEpochStale(format!(
                "caller={}, current={}",
                fencing_epoch, scope_epoch
            )));
        }
    }

    // 2) Fetch rows; filter by epoch defense-in-depth.
    let rows = sqlx::query_as::<_, AuditOutboxRow>(
        "SELECT audit_outbox_id, audit_decision_event_id, decision_id,
                ledger_transaction_id, event_type, cloudevent_payload,
                cloudevent_payload_signature, ledger_fencing_epoch,
                workload_instance_id, pending_forward, forwarded_at,
                recorded_at, producer_sequence
           FROM audit_outbox
          WHERE tenant_id = $1
            AND workload_instance_id = $2
            AND ledger_fencing_epoch <= $3
            AND producer_sequence > $4
          ORDER BY producer_sequence ASC
          LIMIT $5",
    )
    .bind(tenant_id)
    .bind(workload_instance_id)
    .bind(fencing_epoch)
    .bind(producer_sequence_after)
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .map_err(map_pg_error)?;

    Ok(rows)
}

/// Query terminal stage of a decision_id (per Stage 2 §4 recovery state machine).
pub async fn query_decision_outcome(
    pool: &PgPool,
    tenant_id: Uuid,
    decision_id: Uuid,
) -> Result<DecisionOutcome, DomainError> {
    let rows = sqlx::query_as::<
        _,
        (
            String,
            Uuid,
            Uuid,
            chrono::DateTime<chrono::Utc>,
            String,
            Uuid,
            Option<serde_json::Value>,
            serde_json::Value,
        ),
    >(
        "SELECT ao.event_type, ao.audit_decision_event_id, ao.ledger_transaction_id,
                ao.recorded_at, lt.operation_kind, lt.decision_id,
                lt.minimal_replay_response, ao.cloudevent_payload
           FROM audit_outbox ao
           JOIN ledger_transactions lt
             ON lt.ledger_transaction_id = ao.ledger_transaction_id
          WHERE ao.tenant_id = $1
            AND ao.decision_id = $2
          ORDER BY ao.recorded_at ASC",
    )
    .bind(tenant_id)
    .bind(decision_id)
    .fetch_all(pool)
    .await
    .map_err(map_pg_error)?;

    Ok(reduce_decision_outcome_rows(rows))
}

/// One audit_outbox+ledger_transactions row joined for a decision_id, in
/// `ORDER BY recorded_at ASC` order. Tuple shape mirrors the query above.
type DecisionOutcomeRow = (
    String,                          // event_type
    Uuid,                            // audit_decision_event_id
    Uuid,                            // ledger_transaction_id
    chrono::DateTime<chrono::Utc>,   // recorded_at
    String,                          // operation_kind
    Uuid,                            // decision_id (original)
    Option<serde_json::Value>,       // minimal_replay_response
    serde_json::Value,               // cloudevent_payload
);

/// Reduce the joined decision rows to a `DecisionOutcome`.
///
/// The response's `ledger_transaction_id` is the DECISION anchor (the
/// reserve/deny tx), per `QueryDecisionOutcomeResponse.ledger_transaction_id`
/// docs and `ReservationContext.decision_id`. It MUST come from the
/// `spendguard.audit.decision` row — NOT the last-sorted row, which for a
/// committed/released decision is the audit.outcome's DIFFERENT
/// commit/release tx. This matches the sibling
/// `query_decision_outcome_by_idempotency_key`, which returns the decision
/// row's tx. Pure function so the contract can be unit-tested without a DB.
fn reduce_decision_outcome_rows(rows: Vec<DecisionOutcomeRow>) -> DecisionOutcome {
    let mut decision_event = None;
    let mut outcome_event = None;
    let mut last_updated = None;
    let mut tx_id = None;
    let mut replay = ReplayMetadata::default();

    for (kind, audit_id, ltx, recorded, operation_kind, original_decision_id, minimal, payload) in
        rows
    {
        last_updated = Some(recorded);
        match kind.as_str() {
            "spendguard.audit.decision" => {
                decision_event = Some(audit_id);
                // Anchor on the decision tx; do NOT let a later audit.outcome
                // row overwrite it.
                tx_id = Some(ltx);
                replay = replay_metadata(
                    operation_kind,
                    original_decision_id,
                    ltx,
                    minimal.unwrap_or(serde_json::Value::Null),
                    payload,
                );
            }
            "spendguard.audit.outcome" => {
                outcome_event = Some(audit_id);
                // Fallback only: if the decision row is absent (schema drift /
                // partial chain) but an outcome row exists, still surface a tx
                // anchor so callers aren't left with NULL. The decision arm
                // takes precedence whenever both are present, regardless of
                // row order.
                if tx_id.is_none() {
                    tx_id = Some(ltx);
                }
            }
            _ => {}
        }
    }

    let stage = if outcome_event.is_some() {
        Stage::AuditOutcomeRecorded
    } else if decision_event.is_some() {
        Stage::AuditDecisionRecorded
    } else {
        Stage::NotFound
    };

    DecisionOutcome {
        stage,
        ledger_transaction_id: tx_id,
        audit_decision_event_id: decision_event,
        audit_outcome_event_id: outcome_event,
        last_updated_at: last_updated,
        replay,
    }
}

/// Query the original reserve/deny decision for an adapter idempotency key.
///
/// The sidecar uses this before calling the mutating run-cost projector so a
/// retry after process/cache loss replays the durable first decision instead
/// of recomputing under a changed budget or contract snapshot.
pub async fn query_decision_outcome_by_idempotency_key(
    pool: &PgPool,
    tenant_id: Uuid,
    idempotency_key: &str,
) -> Result<DecisionOutcome, DomainError> {
    let row = sqlx::query_as::<
        _,
        (
            String,
            Uuid,
            Uuid,
            Uuid,
            chrono::DateTime<chrono::Utc>,
            Option<serde_json::Value>,
            serde_json::Value,
        ),
    >(
        "SELECT lt.operation_kind, lt.ledger_transaction_id,
                lt.audit_decision_event_id, lt.decision_id, lt.recorded_at,
                lt.minimal_replay_response, ao.cloudevent_payload
           FROM ledger_transactions lt
           JOIN audit_outbox ao
             ON ao.ledger_transaction_id = lt.ledger_transaction_id
            AND ao.event_type = 'spendguard.audit.decision'
          WHERE lt.tenant_id = $1
            AND lt.idempotency_key = $2
            AND lt.operation_kind IN ('reserve', 'denied_decision')
          ORDER BY lt.recorded_at ASC
          LIMIT 1",
    )
    .bind(tenant_id)
    .bind(idempotency_key)
    .fetch_optional(pool)
    .await
    .map_err(map_pg_error)?;

    let Some((operation_kind, tx_id, audit_id, decision_id, recorded_at, minimal, payload)) = row
    else {
        return Ok(DecisionOutcome::not_found());
    };

    Ok(DecisionOutcome {
        stage: Stage::AuditDecisionRecorded,
        ledger_transaction_id: Some(tx_id),
        audit_decision_event_id: Some(audit_id),
        audit_outcome_event_id: None,
        last_updated_at: Some(recorded_at),
        replay: replay_metadata(
            operation_kind,
            decision_id,
            tx_id,
            minimal.unwrap_or(serde_json::Value::Null),
            payload,
        ),
    })
}

fn replay_metadata(
    operation_kind: String,
    decision_id: Uuid,
    ledger_transaction_id: Uuid,
    minimal: serde_json::Value,
    cloudevent_payload: serde_json::Value,
) -> ReplayMetadata {
    let data = cloudevent_data_json(&cloudevent_payload);
    let reason_codes = string_array(data.as_ref(), "reason_codes");
    let matched_rule_ids = string_array(data.as_ref(), "matched_rules");
    let request_fingerprint_hex = data
        .as_ref()
        .and_then(|v| v.get("idempotency_request_fingerprint"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let run_code_triggered = reason_codes
        .iter()
        .find(|code| code.starts_with("RUN_"))
        .cloned()
        .unwrap_or_default();

    let mut final_decision = data
        .as_ref()
        .and_then(|v| v.get("final_decision"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if final_decision.is_empty() && operation_kind == "reserve" {
        final_decision = "CONTINUE".to_string();
    }

    let projection_ids = minimal
        .get("reservation_ids")
        .and_then(|v| v.as_array())
        .map(|ids| {
            ids.iter()
                .filter_map(|id| id.as_str().map(ToString::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let operation_id = if operation_kind == "reserve" {
        derive_reservation_set_id(&decision_id).to_string()
    } else {
        ledger_transaction_id.to_string()
    };

    ReplayMetadata {
        decision_id: Some(decision_id),
        operation_kind,
        operation_id,
        projection_ids,
        ttl_expires_at: minimal
            .get("ttl_expires_at")
            .and_then(|v| v.as_str())
            .and_then(parse_rfc3339_timestamp),
        final_decision,
        matched_rule_ids,
        reason_codes,
        run_code_triggered,
        request_fingerprint_hex,
    }
}

fn cloudevent_data_json(payload: &serde_json::Value) -> Option<serde_json::Value> {
    let raw = payload.get("data_b64")?.as_str()?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(raw).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn string_array(payload: Option<&serde_json::Value>, field: &str) -> Vec<String> {
    payload
        .and_then(|v| v.get(field))
        .and_then(|v| v.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_rfc3339_timestamp(raw: &str) -> Option<prost_types::Timestamp> {
    let parsed = chrono::DateTime::parse_from_rfc3339(raw)
        .ok()?
        .with_timezone(&chrono::Utc);
    Some(prost_types::Timestamp {
        seconds: parsed.timestamp(),
        nanos: parsed.timestamp_subsec_nanos() as i32,
    })
}

fn derive_reservation_set_id(decision_id: &Uuid) -> Uuid {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(decision_id.as_bytes());
    h.update(b":reservation_set");
    let bytes: [u8; 32] = h.finalize().into();
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&bytes[..16]);
    buf[6] = (buf[6] & 0x0f) | 0x40;
    buf[8] = (buf[8] & 0x3f) | 0x80;
    Uuid::from_bytes(buf)
}

#[derive(Debug, Clone)]
pub struct DecisionOutcome {
    pub stage: Stage,
    pub ledger_transaction_id: Option<Uuid>,
    pub audit_decision_event_id: Option<Uuid>,
    pub audit_outcome_event_id: Option<Uuid>,
    pub last_updated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub replay: ReplayMetadata,
}

impl DecisionOutcome {
    fn not_found() -> Self {
        Self {
            stage: Stage::NotFound,
            ledger_transaction_id: None,
            audit_decision_event_id: None,
            audit_outcome_event_id: None,
            last_updated_at: None,
            replay: ReplayMetadata::default(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ReplayMetadata {
    pub decision_id: Option<Uuid>,
    pub operation_kind: String,
    pub operation_id: String,
    pub projection_ids: Vec<String>,
    pub ttl_expires_at: Option<prost_types::Timestamp>,
    pub final_decision: String,
    pub matched_rule_ids: Vec<String>,
    pub reason_codes: Vec<String>,
    pub run_code_triggered: String,
    pub request_fingerprint_hex: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    NotFound,
    AuditDecisionRecorded,
    AuditOutcomeRecorded,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn replay_metadata_extracts_reserve_projection_fields() {
        let decision_id = Uuid::new_v4();
        let tx_id = Uuid::new_v4();
        let reservation_id = Uuid::new_v4().to_string();
        let payload_data = json!({
            "matched_rules": [],
            "reason_codes": [],
            "idempotency_request_fingerprint": "fp-reserve",
        });
        let cloudevent_payload = json!({
            "data_b64": base64::engine::general_purpose::STANDARD
                .encode(serde_json::to_vec(&payload_data).unwrap()),
        });
        let metadata = replay_metadata(
            "reserve".to_string(),
            decision_id,
            tx_id,
            json!({
                "reservation_ids": [reservation_id],
                "ttl_expires_at": "2026-05-31T00:00:00Z",
            }),
            cloudevent_payload,
        );

        assert_eq!(metadata.decision_id, Some(decision_id));
        assert_eq!(metadata.operation_kind, "reserve");
        assert_eq!(metadata.final_decision, "CONTINUE");
        assert_eq!(metadata.request_fingerprint_hex, "fp-reserve");
        assert_eq!(metadata.projection_ids.len(), 1);
        assert!(metadata.ttl_expires_at.is_some());
        assert_eq!(
            metadata.operation_id,
            derive_reservation_set_id(&decision_id).to_string()
        );
    }

    fn decision_row(
        kind: &str,
        ltx: Uuid,
        recorded_secs: i64,
        decision_id: Uuid,
    ) -> DecisionOutcomeRow {
        (
            kind.to_string(),
            Uuid::new_v4(),
            ltx,
            chrono::DateTime::from_timestamp(recorded_secs, 0).unwrap(),
            "reserve".to_string(),
            decision_id,
            Some(json!({})),
            json!({ "data_b64": "" }),
        )
    }

    #[test]
    fn decision_outcome_anchors_on_decision_tx_not_outcome_tx() {
        // A decision that has progressed to outcome has TWO different ledger
        // transactions: the reserve (decision) tx and the commit/release
        // (outcome) tx. ORDER BY recorded_at ASC puts decision first, outcome
        // second. The response's ledger_transaction_id must be the DECISION
        // tx, not the last-sorted outcome tx.
        let decision_id = Uuid::new_v4();
        let decision_tx = Uuid::new_v4();
        let outcome_tx = Uuid::new_v4();
        assert_ne!(decision_tx, outcome_tx);

        let rows = vec![
            decision_row("spendguard.audit.decision", decision_tx, 100, decision_id),
            decision_row("spendguard.audit.outcome", outcome_tx, 200, decision_id),
        ];

        let outcome = reduce_decision_outcome_rows(rows);
        assert_eq!(outcome.stage, Stage::AuditOutcomeRecorded);
        assert_eq!(
            outcome.ledger_transaction_id,
            Some(decision_tx),
            "ledger_transaction_id must be the decision tx, not the outcome tx"
        );
    }

    #[test]
    fn decision_outcome_decision_wins_even_when_outcome_sorts_first() {
        // Defense in depth: even if recorded_at ordering placed the outcome
        // row before the decision row, the decision tx must still win.
        let decision_id = Uuid::new_v4();
        let decision_tx = Uuid::new_v4();
        let outcome_tx = Uuid::new_v4();

        let rows = vec![
            decision_row("spendguard.audit.outcome", outcome_tx, 100, decision_id),
            decision_row("spendguard.audit.decision", decision_tx, 200, decision_id),
        ];

        let outcome = reduce_decision_outcome_rows(rows);
        assert_eq!(outcome.ledger_transaction_id, Some(decision_tx));
    }

    #[test]
    fn decision_outcome_falls_back_to_outcome_tx_when_no_decision_row() {
        // Partial chain (decision row absent): still surface a tx anchor.
        let decision_id = Uuid::new_v4();
        let outcome_tx = Uuid::new_v4();
        let rows = vec![decision_row(
            "spendguard.audit.outcome",
            outcome_tx,
            100,
            decision_id,
        )];

        let outcome = reduce_decision_outcome_rows(rows);
        assert_eq!(outcome.stage, Stage::AuditOutcomeRecorded);
        assert_eq!(outcome.ledger_transaction_id, Some(outcome_tx));
    }

    #[test]
    fn replay_metadata_extracts_denied_run_code() {
        let payload_data = json!({
            "final_decision": "STOP",
            "matched_rules": ["projection-stop"],
            "reason_codes": ["RUN_BUDGET_PROJECTION_EXCEEDED"],
            "idempotency_request_fingerprint": "fp-denied",
        });
        let metadata = replay_metadata(
            "denied_decision".to_string(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::Value::Null,
            json!({
                "data_b64": base64::engine::general_purpose::STANDARD
                    .encode(serde_json::to_vec(&payload_data).unwrap()),
            }),
        );

        assert_eq!(metadata.final_decision, "STOP");
        assert_eq!(
            metadata.run_code_triggered,
            "RUN_BUDGET_PROJECTION_EXCEEDED"
        );
        assert_eq!(metadata.request_fingerprint_hex, "fp-denied");
        assert_eq!(metadata.matched_rule_ids, vec!["projection-stop"]);
    }
}
