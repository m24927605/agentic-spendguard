//! `Ledger::Release` handler (Phase 2B Step 7.5).
//!
//! Wire contract: `proto/spendguard/ledger/v1/ledger.proto` Release.
//!
//! Spec references:
//!   - Contract DSL §6 stage 7 (commit_or_release; release lane)
//!   - Contract DSL §7 (reservation TTL/release/runtime_error/run_aborted)
//!   - Ledger §10 (account_kinds; release reverses reserved_hold via
//!     compensating debit/credit on reserved_hold + available_budget)
//!   - Stage 2 §4 (audit_outbox; per-decision uniqueness)
//!
//! Authority model — SP is server-derived (Codex Step 7.5 round 1):
//!   * Wire ReleaseRequest carries only identity + reason + audit_event.
//!   * SP looks up reserve tx via decision_id, recovers reservations
//!     + frozen pricing tuple from server state. NO caller pricing.
//!   * Single-reservation set only (POC limitation).
//!
//! Audit pattern:
//!   * audit.outcome event_type with ORIGINAL ReserveSet decision_id.
//!   * Mutually exclusive with commit_estimated (state machine
//!     enforces 'reserved' -> 'released' or 'reserved' -> 'committed';
//!     never both).

use base64::Engine as _;
use prost_types::Timestamp;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use tracing::{debug, instrument};
use uuid::Uuid;

use crate::{
    domain::{error::DomainError, minimal_replay},
    persistence::post_release::{self, PostReleaseInput},
    proto::ledger::v1::{
        release_request::Reason, release_response::Outcome, ReleaseRequest, ReleaseResponse,
        ReleaseSuccess,
    },
};

#[instrument(skip(pool, req), fields(
    tenant = %req.tenant_id,
    reservation_set_id = %req.reservation_set_id,
    decision_id = %req.decision_id,
    reason = req.reason,
))]
pub async fn handle(
    pool: &PgPool,
    req: ReleaseRequest,
) -> Result<ReleaseResponse, tonic::Status> {
    match handle_inner(pool, req).await {
        Ok(resp) => Ok(resp),
        Err(DomainError::Internal(e)) => Err(tonic::Status::internal(e.to_string())),
        Err(DomainError::Db(e)) => Err(tonic::Status::unavailable(format!("db: {}", e))),
        Err(other) => {
            let proto_err = other.to_proto();
            Ok(ReleaseResponse {
                outcome: Some(Outcome::Error(proto_err)),
            })
        }
    }
}

async fn handle_inner(
    pool: &PgPool,
    req: ReleaseRequest,
) -> Result<ReleaseResponse, DomainError> {
    validate(&req)?;

    let tenant_id = parse_uuid(&req.tenant_id, "tenant_id")?;
    let reservation_set_id = parse_uuid(&req.reservation_set_id, "reservation_set_id")?;
    let decision_id = parse_uuid(&req.decision_id, "decision_id")?;

    let idempotency = req
        .idempotency
        .as_ref()
        .ok_or_else(|| DomainError::InvalidRequest("idempotency missing".into()))?;
    if idempotency.key.is_empty() {
        return Err(DomainError::InvalidRequest("idempotency.key empty".into()));
    }

    let fencing = req
        .fencing
        .as_ref()
        .ok_or_else(|| DomainError::InvalidRequest("fencing missing".into()))?;
    if fencing.scope_id.is_empty() || fencing.workload_instance_id.is_empty() {
        return Err(DomainError::InvalidRequest(
            "fencing.scope_id + workload_instance_id required".into(),
        ));
    }
    if fencing.epoch == 0 {
        return Err(DomainError::FencingEpochStale(
            "epoch 0 is not a valid lease".into(),
        ));
    }

    let audit_event = req
        .audit_event
        .as_ref()
        .ok_or_else(|| DomainError::InvalidRequest("audit_event required".into()))?;
    if audit_event.r#type != "spendguard.audit.outcome" {
        return Err(DomainError::InvalidRequest(format!(
            "audit_event.type must be spendguard.audit.outcome; got '{}'",
            audit_event.r#type
        )));
    }
    if audit_event.decision_id != req.decision_id {
        return Err(DomainError::InvalidRequest(
            "audit_event.decision_id must match request.decision_id".into(),
        ));
    }
    let audit_outbox_id = parse_uuid(&audit_event.id, "audit_event.id")?;

    let reason_enum = Reason::try_from(req.reason).unwrap_or(Reason::Unspecified);
    if reason_enum == Reason::Unspecified {
        return Err(DomainError::InvalidRequest(
            "reason required (TTL_EXPIRED / RUNTIME_ERROR / RUN_ABORTED / EXPLICIT)".into(),
        ));
    }
    let reason_str = match reason_enum {
        Reason::TtlExpired => "TTL_EXPIRED",
        Reason::RuntimeError => "RUNTIME_ERROR",
        Reason::RunAborted => "RUN_ABORTED",
        Reason::Explicit => "EXPLICIT",
        Reason::Unspecified => unreachable!(),
    };

    // Codex round 2 M2.2: explicit canonical request_hash definition.
    // Excludes retry-volatile fields (tx id, audit id, producer sequence,
    // timestamps, fencing epoch). Includes stable business intent:
    // tenant + reservation_set_id + decision_id + reason + operation_kind.
    let request_hash = canonical_request_hash(&req, reason_str, &idempotency.request_hash)?;

    let ledger_transaction_id = Uuid::now_v7();

    let transaction_json = json!({
        "ledger_transaction_id":   ledger_transaction_id.to_string(),
        "tenant_id":               tenant_id.to_string(),
        "idempotency_key":         idempotency.key,
        "request_hash_hex":        hex::encode(request_hash),
        "decision_id":             decision_id.to_string(),
        "audit_decision_event_id": audit_outbox_id.to_string(),
        "fencing_scope_id":        fencing.scope_id,
        "fencing_epoch":           fencing.epoch as i64,
        "workload_instance_id":    fencing.workload_instance_id,
        "effective_at":            chrono::Utc::now().to_rfc3339(),
        "minimal_replay_response": minimal_replay_seed(
            &ledger_transaction_id, &decision_id, &reservation_set_id, reason_str,
        ),
    });

    let audit_outbox_json = json!({
        "audit_outbox_id":                  audit_outbox_id.to_string(),
        "event_type":                       "spendguard.audit.outcome",
        "cloudevent_payload":               extract_cloudevent_payload(audit_event)?,
        "cloudevent_payload_signature_hex": hex::encode(&audit_event.producer_signature),
        "producer_sequence":                req.producer_sequence as i64,
    });

    let returned_tx_id = post_release::post(
        pool,
        PostReleaseInput {
            transaction: transaction_json,
            reservation_set_id,
            reason: reason_str,
            audit_outbox_row: audit_outbox_json,
        },
    )
    .await?;

    if returned_tx_id == ledger_transaction_id {
        return Ok(ReleaseResponse {
            outcome: Some(Outcome::Success(build_success(
                ledger_transaction_id,
                &reservation_set_id,
            ))),
        });
    }

    debug!(returned = %returned_tx_id, "idempotent replay hit");
    let replay = build_replay(pool, returned_tx_id, &decision_id, &reservation_set_id).await?;
    Ok(ReleaseResponse {
        outcome: Some(Outcome::Replay(replay)),
    })
}

// ---- helpers ---------------------------------------------------------------

fn validate(req: &ReleaseRequest) -> Result<(), DomainError> {
    if req.tenant_id.is_empty() {
        return Err(DomainError::InvalidRequest("tenant_id required".into()));
    }
    if req.reservation_set_id.is_empty() {
        return Err(DomainError::InvalidRequest(
            "reservation_set_id required".into(),
        ));
    }
    if req.decision_id.is_empty() {
        return Err(DomainError::InvalidRequest("decision_id required".into()));
    }
    if req.audit_event.is_none() {
        return Err(DomainError::InvalidRequest("audit_event required".into()));
    }
    Ok(())
}

fn parse_uuid(s: &str, field: &str) -> Result<Uuid, DomainError> {
    Uuid::parse_str(s)
        .map_err(|e| DomainError::InvalidRequest(format!("{}: invalid uuid ({})", field, e)))
}

fn canonical_request_hash(
    req: &ReleaseRequest,
    reason_str: &str,
    caller_hash: &[u8],
) -> Result<[u8; 32], DomainError> {
    let mut h = Sha256::new();
    h.update(b"v1:release:business_intent:");
    h.update(req.tenant_id.as_bytes());
    h.update(b"|reservation_set|");
    h.update(req.reservation_set_id.as_bytes());
    h.update(b"|decision|");
    h.update(req.decision_id.as_bytes());
    h.update(b"|reason|");
    h.update(reason_str.as_bytes());

    let canonical: [u8; 32] = h.finalize().into();
    if !caller_hash.is_empty() {
        if caller_hash.len() != 32 {
            return Err(DomainError::InvalidRequest(
                "Idempotency.request_hash must be 32 bytes".into(),
            ));
        }
        if caller_hash != canonical {
            return Err(DomainError::IdempotencyConflict);
        }
    }
    Ok(canonical)
}

fn minimal_replay_seed(
    tx: &Uuid,
    decision: &Uuid,
    reservation_set: &Uuid,
    reason: &str,
) -> Value {
    json!({
        "ledger_transaction_id": tx.to_string(),
        "decision_id":           decision.to_string(),
        "operation_kind":        "release",
        "operation_id":          reservation_set.to_string(),
        "reason":                reason,
    })
}

fn extract_cloudevent_payload(
    evt: &crate::proto::common::v1::CloudEvent,
) -> Result<Value, DomainError> {
    Ok(json!({
        "specversion":     evt.specversion,
        "type":            evt.r#type,
        "source":          evt.source,
        "id":              evt.id,
        "time_seconds":    evt.time.as_ref().map(|t| t.seconds).unwrap_or_default(),
        "time_nanos":      evt.time.as_ref().map(|t| t.nanos).unwrap_or_default(),
        "datacontenttype": evt.datacontenttype,
        "data_b64":        base64::engine::general_purpose::STANDARD.encode(&evt.data),
        "tenantid":        evt.tenant_id,
        "runid":           evt.run_id,
        "decisionid":      evt.decision_id,
        "schema_bundle_id": evt.schema_bundle_id,
        "producer_id":     evt.producer_id,
        "producer_sequence": evt.producer_sequence,
        "signing_key_id":  evt.signing_key_id,
    }))
}

fn build_success(ledger_transaction_id: Uuid, reservation_set_id: &Uuid) -> ReleaseSuccess {
    let now = chrono::Utc::now();
    ReleaseSuccess {
        ledger_transaction_id: ledger_transaction_id.to_string(),
        // POC single-claim: released_reservation_ids contains the single
        // reservation_id (derived from set). Caller can re-derive same;
        // SP doesn't return it, but minimal_replay_response carries
        // operation_id = reservation_set_id.
        released_reservation_ids: vec![reservation_set_id.to_string()],
        recorded_at: Some(Timestamp {
            seconds: now.timestamp(),
            nanos: now.timestamp_subsec_nanos() as i32,
        }),
    }
}

async fn build_replay(
    pool: &PgPool,
    existing_tx_id: Uuid,
    _retry_decision_id: &Uuid,
    reservation_set_id: &Uuid,
) -> Result<crate::proto::common::v1::Replay, DomainError> {
    let row = sqlx::query(
        "SELECT operation_kind, audit_decision_event_id, decision_id, \
                posting_state, recorded_at \
           FROM ledger_transactions \
          WHERE ledger_transaction_id = $1",
    )
    .bind(existing_tx_id)
    .fetch_one(pool)
    .await
    .map_err(|e| DomainError::Internal(anyhow::anyhow!("replay lookup: {e}")))?;

    let op_kind: String = row.get("operation_kind");
    let posting_state: String = row.get("posting_state");
    let recorded_at: chrono::DateTime<chrono::Utc> = row.get("recorded_at");
    let audit_id: Uuid = row
        .try_get::<Option<Uuid>, _>("audit_decision_event_id")
        .map_err(|e| DomainError::Internal(anyhow::anyhow!("replay audit id: {e}")))?
        .ok_or_else(|| {
            DomainError::Internal(anyhow::anyhow!(
                "replay row {} has NULL audit_decision_event_id",
                existing_tx_id
            ))
        })?;
    let original_decision_id: Uuid = row
        .try_get::<Option<Uuid>, _>("decision_id")
        .map_err(|e| DomainError::Internal(anyhow::anyhow!("replay decision id: {e}")))?
        .ok_or_else(|| {
            DomainError::Internal(anyhow::anyhow!(
                "replay row {} has NULL decision_id",
                existing_tx_id
            ))
        })?;

    Ok(minimal_replay::from_db_row(
        existing_tx_id.to_string(),
        &op_kind,
        audit_id.to_string(),
        reservation_set_id.to_string(),
        original_decision_id.to_string(),
        Vec::new(),
        None,
        recorded_at,
        &posting_state,
    ))
}
