//! Recovery: `ReplayAuditFromCursor` + `QueryDecisionOutcome`.

use base64::Engine as _;
use prost_types::Timestamp;
use sqlx::PgPool;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Response, Status};
use tracing::{instrument, warn};
use uuid::Uuid;

use crate::{
    domain::error::DomainError,
    persistence::replay::{self, AuditOutboxRow, Stage},
    proto::{
        common::v1::CloudEvent,
        ledger::v1::{
            query_decision_outcome_response::Stage as ProtoStage,
            replay_audit_event::ForwardingState, QueryDecisionOutcomeRequest,
            QueryDecisionOutcomeResponse, ReplayAuditEvent, ReplayAuditFromCursorRequest,
        },
    },
};

#[instrument(skip(pool, req), fields(
    tenant = %req.tenant_id, workload = %req.workload_instance_id,
    after = req.producer_sequence_after, limit = req.limit
))]
pub async fn replay_stream(
    pool: PgPool,
    req: ReplayAuditFromCursorRequest,
) -> Result<Response<ReceiverStream<Result<ReplayAuditEvent, Status>>>, Status> {
    let tenant_id = Uuid::parse_str(&req.tenant_id)
        .map_err(|e| Status::invalid_argument(format!("tenant_id: {}", e)))?;
    let fencing_scope_id = if req.fencing_scope_id.is_empty() {
        None
    } else {
        Some(
            Uuid::parse_str(&req.fencing_scope_id).map_err(|e| {
                Status::invalid_argument(format!("fencing_scope_id: {}", e))
            })?,
        )
    };

    let (tx, rx) = mpsc::channel::<Result<ReplayAuditEvent, Status>>(64);

    tokio::spawn(async move {
        let rows = replay::replay_audit_from_cursor(
            &pool,
            tenant_id,
            &req.workload_instance_id,
            req.fencing_epoch as i64,
            fencing_scope_id,
            req.producer_sequence_after as i64,
            req.limit.max(1),
        )
        .await;

        match rows {
            Err(e) => {
                let _ = tx.send(Err(e.to_status())).await;
            }
            Ok(rows) => {
                for row in rows {
                    let cloudevent = match decode_cloudevent_from_jsonb(&row) {
                        Ok(ev) => ev,
                        Err(e) => {
                            warn!(
                                audit_id = %row.audit_decision_event_id,
                                err = %e,
                                "skipping row with undecodable CloudEvent payload"
                            );
                            continue;
                        }
                    };
                    let recorded_at = Timestamp {
                        seconds: row.recorded_at.timestamp(),
                        nanos: row.recorded_at.timestamp_subsec_nanos() as i32,
                    };
                    let forwarding_state = if row.pending_forward {
                        ForwardingState::Pending
                    } else {
                        ForwardingState::Forwarded
                    };
                    let event = ReplayAuditEvent {
                        event: Some(cloudevent),
                        producer_sequence: row.producer_sequence as u64,
                        recorded_at: Some(recorded_at),
                        ledger_transaction_id: row.ledger_transaction_id.to_string(),
                        forwarding_state: forwarding_state as i32,
                    };
                    if tx.send(Ok(event)).await.is_err() {
                        return; // client disconnected
                    }
                }
            }
        }
    });

    Ok(Response::new(ReceiverStream::new(rx)))
}

#[instrument(skip(pool, req), fields(tenant = %req.tenant_id, decision_id = %req.decision_id))]
pub async fn query_decision_outcome(
    pool: &PgPool,
    req: QueryDecisionOutcomeRequest,
) -> Result<QueryDecisionOutcomeResponse, Status> {
    let tenant_id = Uuid::parse_str(&req.tenant_id)
        .map_err(|e| Status::invalid_argument(format!("tenant_id: {}", e)))?;
    let decision_id = Uuid::parse_str(&req.decision_id)
        .map_err(|e| Status::invalid_argument(format!("decision_id: {}", e)))?;

    let outcome = replay::query_decision_outcome(pool, tenant_id, decision_id)
        .await
        .map_err(|e: DomainError| e.to_status())?;

    let stage = match outcome.stage {
        Stage::NotFound => ProtoStage::NotFound,
        Stage::AuditDecisionRecorded => ProtoStage::AuditDecisionRecorded,
        Stage::AuditOutcomeRecorded => ProtoStage::AuditOutcomeRecorded,
    };

    Ok(QueryDecisionOutcomeResponse {
        stage: stage as i32,
        ledger_transaction_id: outcome
            .ledger_transaction_id
            .map(|u| u.to_string())
            .unwrap_or_default(),
        audit_decision_event_id: outcome
            .audit_decision_event_id
            .map(|u| u.to_string())
            .unwrap_or_default(),
        audit_outcome_event_id: outcome
            .audit_outcome_event_id
            .map(|u| u.to_string())
            .unwrap_or_default(),
        last_updated_at: outcome.last_updated_at.map(|t| Timestamp {
            seconds: t.timestamp(),
            nanos: t.timestamp_subsec_nanos() as i32,
        }),
        // effect_hash is held by the in-process adapter (per Stage 2 §4.6),
        // not by the ledger; we return empty here. Adapter recovery looks
        // it up locally before re-publishing. The proto field exists for
        // forward compatibility with a future ledger-side effect_hash store.
        effect_hash: Default::default(),
    })
}

/// Reconstruct a `CloudEvent` proto from the JSONB payload stored in
/// audit_outbox. We do NOT use `serde_json::from_value::<CloudEvent>`
/// because prost-generated messages do not derive `serde::Deserialize`
/// in the default codegen.
fn decode_cloudevent_from_jsonb(row: &AuditOutboxRow) -> Result<CloudEvent, anyhow::Error> {
    let payload = &row.cloudevent_payload;
    let get_str = |k: &str| -> String {
        payload
            .get(k)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default()
    };
    let get_u64 = |k: &str| -> u64 {
        payload.get(k).and_then(|v| v.as_u64()).unwrap_or_default()
    };
    let get_i64 = |k: &str| -> i64 {
        payload.get(k).and_then(|v| v.as_i64()).unwrap_or_default()
    };
    let get_i32 = |k: &str| -> i32 { get_i64(k) as i32 };

    let data_b64 = get_str("data_b64");
    let data: prost::bytes::Bytes = if data_b64.is_empty() {
        Default::default()
    } else {
        base64::engine::general_purpose::STANDARD
            .decode(data_b64)
            .map_err(|e| anyhow::anyhow!("data_b64 decode: {}", e))?
            .into()
    };

    let signature_hex = hex::encode(&row.cloudevent_payload_signature);
    let producer_signature: prost::bytes::Bytes = if signature_hex.is_empty() {
        Default::default()
    } else {
        row.cloudevent_payload_signature.clone().into()
    };

    Ok(CloudEvent {
        specversion: get_str("specversion"),
        r#type: get_str("type"),
        source: get_str("source"),
        id: get_str("id"),
        time: Some(Timestamp {
            seconds: get_i64("time_seconds"),
            nanos: get_i32("time_nanos"),
        }),
        datacontenttype: get_str("datacontenttype"),
        data,
        tenant_id: get_str("tenantid"),
        run_id: get_str("runid"),
        decision_id: get_str("decisionid"),
        schema_bundle_id: get_str("schema_bundle_id"),
        producer_id: get_str("producer_id"),
        producer_sequence: get_u64("producer_sequence"),
        producer_signature,
        signing_key_id: get_str("signing_key_id"),
    })
}
