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

/// Outcome of a single ledger release attempt.
///
/// Distinguishes a genuine release (`Released`/`Replay` — idempotent
/// success) from a ledger refusal (`Refused`, e.g.
/// RESERVATION_STATE_CONFLICT / TTL_NOT_EXPIRED), a transport failure
/// (`TransportError`), or a malformed response (`MissingOutcome`). The
/// caller maps `Released`/`Replay` to the swept-ok counter and the rest
/// to swept-err so the success metric reflects actual releases rather
/// than "the row was visited". In every case the reservation stays
/// `reserved` and is retried next cycle (fail-closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepOutcome {
    Released,
    Replay,
    Refused,
    TransportError,
    MissingOutcome,
}

impl SweepOutcome {
    /// True only when the reservation was actually released (including
    /// an idempotent replay). Drives the swept-ok vs swept-err metric.
    pub fn is_success(self) -> bool {
        matches!(self, SweepOutcome::Released | SweepOutcome::Replay)
    }
}

pub async fn sweep_one(state: &mut AppState, row: ExpiredRow) -> anyhow::Result<SweepOutcome> {
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

    let mut cloud_event = CloudEvent {
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
        signing_key_id: String::new(),
        producer_signature: Vec::new().into(),
        ..Default::default()
    };
    crate::audit::sign_cloudevent_in_place(&*state.signer, &mut cloud_event).await?;

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

    let outcome = match state.ledger_client.release(req).await {
        Ok(resp) => match resp.into_inner().outcome {
            Some(Outcome::Success(s)) => {
                info!(
                    reservation_id = %row.reservation_id,
                    ledger_tx = %s.ledger_transaction_id,
                    "TTL released"
                );
                SweepOutcome::Released
            }
            Some(Outcome::Replay(r)) => {
                info!(
                    reservation_id = %row.reservation_id,
                    ledger_tx = %r.ledger_transaction_id,
                    "TTL release replay"
                );
                SweepOutcome::Replay
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
                SweepOutcome::Refused
            }
            None => {
                warn!("ledger response missing outcome");
                SweepOutcome::MissingOutcome
            }
        },
        Err(grpc_status) => {
            warn!(
                reservation_id = %row.reservation_id,
                status = %grpc_status,
                "release gRPC failed"
            );
            SweepOutcome::TransportError
        }
    };

    // Marker — keeps base64 import linked if future encoding needed.
    let _ = base64::engine::general_purpose::STANDARD.encode([]);

    // The reservation stays `reserved` and is retried next cycle for
    // every non-`Released`/`Replay` outcome (fail-closed). The caller
    // uses `SweepOutcome::is_success()` to pick the swept-ok vs
    // swept-err counter so the success metric reflects actual releases.
    Ok(outcome)
}
