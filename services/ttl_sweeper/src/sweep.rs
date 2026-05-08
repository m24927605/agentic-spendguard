//! Per-reservation sweep: build ReleaseRequest + call ledger gRPC.

use base64::Engine as _;
use prost_types::Timestamp;
use serde_json::json;
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    canonical::{derive_reservation_set_id, release_hash},
    poll::ExpiredRow,
    proto::{
        common::v1::{CloudEvent, Fencing, Idempotency},
        ledger::v1::{release_request::Reason, release_response::Outcome, ReleaseRequest},
    },
    state::AppState,
};

const REASON_STR: &str = "TTL_EXPIRED";

pub async fn sweep_one(state: &mut AppState, row: ExpiredRow) -> anyhow::Result<()> {
    let reservation_set_id = derive_reservation_set_id(&row.original_decision_id);
    let canonical = release_hash(
        &row.tenant_id,
        &reservation_set_id,
        &row.original_decision_id,
        REASON_STR,
    );

    let idempotency_key = format!("ttl_release:{}", row.reservation_id);
    let producer_seq = state.seq.next_one();
    let audit_event_id = Uuid::now_v7();

    let now = chrono::Utc::now();
    let ts = Timestamp {
        seconds: now.timestamp(),
        nanos: now.timestamp_subsec_nanos() as i32,
    };

    // CloudEvent data carries reason + reservation_id (verify SQL decodes).
    let data_json = json!({
        "reason": REASON_STR,
        "reservation_id": row.reservation_id.to_string(),
        "ttl_expires_at": row.ttl_expires_at.to_rfc3339(),
    });
    let data_bytes = serde_json::to_vec(&data_json)?;

    let cloud_event = CloudEvent {
        specversion: "1.0".into(),
        r#type: "spendguard.audit.outcome".into(),
        source: format!(
            "ttl-sweeper://{}/{}",
            row.tenant_id, state.config.workload_instance_id
        ),
        id: audit_event_id.to_string(),
        time: Some(ts),
        datacontenttype: "application/json".into(),
        data: data_bytes.into(),
        tenant_id: row.tenant_id.to_string(),
        run_id: String::new(),
        decision_id: row.original_decision_id.to_string(),
        schema_bundle_id: String::new(),
        producer_id: format!("ttl-sweeper:{}", state.config.workload_instance_id),
        producer_sequence: producer_seq,
        signing_key_id: "ttl-sweeper:demo:v1".into(),
        producer_signature: Vec::new().into(),
    };

    let req = ReleaseRequest {
        tenant_id: row.tenant_id.to_string(),
        reservation_set_id: reservation_set_id.to_string(),
        idempotency: Some(Idempotency {
            key: idempotency_key.clone(),
            request_hash: canonical.to_vec().into(),
        }),
        fencing: Some(Fencing {
            epoch: state.config.fencing_initial_epoch as u64,
            scope_id: state.config.fencing_scope_id.clone(),
            workload_instance_id: state.config.workload_instance_id.clone(),
        }),
        reason: Reason::TtlExpired as i32,
        audit_event: Some(cloud_event),
        decision_id: row.original_decision_id.to_string(),
        producer_sequence: producer_seq,
    };

    match state.ledger_client.release(req).await {
        Ok(resp) => match resp.into_inner().outcome {
            Some(Outcome::Success(s)) => {
                info!(
                    reservation_id = %row.reservation_id,
                    ledger_tx = %s.ledger_transaction_id,
                    "TTL released"
                );
            }
            Some(Outcome::Replay(r)) => {
                info!(
                    reservation_id = %row.reservation_id,
                    ledger_tx = %r.ledger_transaction_id,
                    "TTL release replay"
                );
            }
            Some(Outcome::Error(e)) => {
                // Common: RESERVATION_STATE_CONFLICT (sidecar already
                // released), TTL_NOT_EXPIRED (clock skew between poll
                // and SP), MULTI_RESERVATION_SET_DEFERRED.
                warn!(
                    reservation_id = %row.reservation_id,
                    code = e.code,
                    message = %e.message,
                    "release error"
                );
            }
            None => warn!("ledger response missing outcome"),
        },
        Err(grpc_status) => warn!(
            reservation_id = %row.reservation_id,
            status = %grpc_status,
            "release gRPC failed"
        ),
    }

    // Marker — keeps base64 import linked if future encoding needed.
    let _ = base64::engine::general_purpose::STANDARD.encode([]);

    Ok(())
}
