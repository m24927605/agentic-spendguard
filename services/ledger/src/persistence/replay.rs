//! ReplayAuditFromCursor query (per Stage 2 §4.7).

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
    let rows = sqlx::query_as::<_, (String, Uuid, Uuid, chrono::DateTime<chrono::Utc>)>(
        "SELECT event_type, audit_decision_event_id, ledger_transaction_id, recorded_at
           FROM audit_outbox
          WHERE tenant_id = $1
            AND decision_id = $2
          ORDER BY recorded_at ASC",
    )
    .bind(tenant_id)
    .bind(decision_id)
    .fetch_all(pool)
    .await
    .map_err(map_pg_error)?;

    let mut decision_event = None;
    let mut outcome_event = None;
    let mut last_updated = None;
    let mut tx_id = None;

    for (kind, audit_id, ltx, recorded) in rows {
        last_updated = Some(recorded);
        tx_id = Some(ltx);
        match kind.as_str() {
            "spendguard.audit.decision" => decision_event = Some(audit_id),
            "spendguard.audit.outcome" => outcome_event = Some(audit_id),
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

    Ok(DecisionOutcome {
        stage,
        ledger_transaction_id: tx_id,
        audit_decision_event_id: decision_event,
        audit_outcome_event_id: outcome_event,
        last_updated_at: last_updated,
    })
}

#[derive(Debug, Clone)]
pub struct DecisionOutcome {
    pub stage: Stage,
    pub ledger_transaction_id: Option<Uuid>,
    pub audit_decision_event_id: Option<Uuid>,
    pub audit_outcome_event_id: Option<Uuid>,
    pub last_updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    NotFound,
    AuditDecisionRecorded,
    AuditOutcomeRecorded,
}
