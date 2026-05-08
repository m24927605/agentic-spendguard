//! `Ledger::ProviderReport` handler (Phase 2B Step 8).
//!
//! Wire contract: `proto/spendguard/ledger/v1/ledger.proto` ProviderReport.
//!
//! Spec references:
//!   - Contract DSL §5  (commitStateMachine: estimated -> provider_reported)
//!   - Contract DSL §5.1a (post-commit overrun -> overrun_debt path; rejected here)
//!   - Ledger §10       (delta entries adjust committed_spend vs available)
//!   - Stage 2 §0.2 D9  (Provider Webhook Receiver = only provider entry)
//!   - Stage 2 §8.2.3   (webhook flow; dedup by provider event id)
//!
//! Audit pattern (Codex Step 8 round 1 DD-A1):
//!   * `audit.decision` event_type for the transition (NOT outcome).
//!   * Caller mints decision_id deterministically from
//!     `sha256("provider_report:{provider}:{provider_account}:{provider_event_id}")`
//!     to make webhook re-delivery idempotent.
//!
//! Step 7 → Step 8 invariants preserved:
//!   * SP locks reservations (must be 'committed') + commits row (must be
//!     `latest_state='estimated'`); rejects later transitions.
//!   * Provider over-actual (provider > original_reserved) is rejected
//!     with OVERRUN_RESERVATION (post-commit overrun is overrun_debt path,
//!     deferred handler).
//!   * Pricing tuple (4 fields) MUST equal the original reserve's frozen
//!     tuple (IS DISTINCT FROM compare in SP).

use base64::Engine as _;
use num_bigint::BigInt;
use prost_types::Timestamp;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use tracing::{debug, instrument};
use uuid::Uuid;

use crate::{
    domain::{error::DomainError, minimal_replay},
    persistence::post_provider_reported::{self, PostProviderReportedInput},
    proto::ledger::v1::{
        provider_report_response::Outcome, CommitState, CommitSuccess, ProviderReportRequest,
        ProviderReportResponse,
    },
};

#[instrument(skip(pool, req), fields(
    tenant = %req.tenant_id,
    reservation_id = %req.reservation_id,
    decision_id = %req.decision_id,
))]
pub async fn handle(
    pool: &PgPool,
    req: ProviderReportRequest,
) -> Result<ProviderReportResponse, tonic::Status> {
    match handle_inner(pool, req).await {
        Ok(resp) => Ok(resp),
        Err(DomainError::Internal(e)) => Err(tonic::Status::internal(e.to_string())),
        Err(DomainError::Db(e)) => Err(tonic::Status::unavailable(format!("db: {}", e))),
        Err(other) => {
            let proto_err = other.to_proto();
            Ok(ProviderReportResponse {
                outcome: Some(Outcome::Error(proto_err)),
            })
        }
    }
}

async fn handle_inner(
    pool: &PgPool,
    req: ProviderReportRequest,
) -> Result<ProviderReportResponse, DomainError> {
    validate(&req)?;

    let tenant_id = parse_uuid(&req.tenant_id, "tenant_id")?;
    let reservation_id = parse_uuid(&req.reservation_id, "reservation_id")?;
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

    let pricing = req
        .pricing
        .as_ref()
        .ok_or_else(|| DomainError::InvalidRequest("pricing missing".into()))?;
    if pricing.pricing_version.is_empty() || pricing.price_snapshot_hash.is_empty() {
        return Err(DomainError::InvalidRequest(
            "pricing.pricing_version + price_snapshot_hash required".into(),
        ));
    }

    let audit_event = req
        .audit_event
        .as_ref()
        .ok_or_else(|| DomainError::InvalidRequest("audit_event required".into()))?;
    // P2.3: validate audit_event consistency.
    if audit_event.r#type != "spendguard.audit.decision" {
        return Err(DomainError::InvalidRequest(format!(
            "audit_event.type must be spendguard.audit.decision; got '{}'",
            audit_event.r#type
        )));
    }
    if audit_event.decision_id != req.decision_id {
        return Err(DomainError::InvalidRequest(
            "audit_event.decision_id must match request.decision_id".into(),
        ));
    }
    let audit_outbox_id = parse_uuid(&audit_event.id, "audit_event.id")?;

    let provider_amount = req
        .provider_reported_amount_atomic
        .parse::<BigInt>()
        .map_err(|e| {
            DomainError::InvalidRequest(format!("provider_reported_amount_atomic invalid: {e}"))
        })?;
    if provider_amount.sign() != num_bigint::Sign::Plus {
        return Err(DomainError::InvalidRequest(
            "provider_reported_amount_atomic must be > 0".into(),
        ));
    }

    let unit = req
        .unit
        .as_ref()
        .ok_or_else(|| DomainError::InvalidRequest("unit missing".into()))?;
    if unit.unit_id.is_empty() {
        return Err(DomainError::InvalidRequest("unit.unit_id empty".into()));
    }

    if req.provider_response_metadata.is_empty() {
        return Err(DomainError::InvalidRequest(
            "provider_response_metadata required (Step 8 POC: namespaced provider_event_id)".into(),
        ));
    }

    // Canonical request_hash. Excludes producer_sequence, audit ids,
    // timestamps, fencing.epoch, decision_id (caller-derived from same
    // namespaced metadata, may legitimately differ across receiver versions).
    let request_hash =
        canonical_request_hash(&req, &provider_amount, &idempotency.request_hash)?;

    let ledger_transaction_id = Uuid::now_v7();

    let pricing_json = json!({
        "pricing_version":         pricing.pricing_version,
        "price_snapshot_hash_hex": hex::encode(&pricing.price_snapshot_hash),
        "fx_rate_version":         pricing.fx_rate_version,
        "unit_conversion_version": pricing.unit_conversion_version,
    });

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
        // Codex Step 8 challenge P2.1: pass unit_id so SP can verify
        // caller's unit matches the original reserve entry.
        "unit_id":                 unit.unit_id,
        "minimal_replay_response": minimal_replay_seed(&ledger_transaction_id, &decision_id, &reservation_id),
    });

    let audit_outbox_json = json!({
        "audit_outbox_id":                  audit_outbox_id.to_string(),
        "event_type":                       "spendguard.audit.decision",
        "cloudevent_payload":               extract_cloudevent_payload(audit_event)?,
        "cloudevent_payload_signature_hex": hex::encode(&audit_event.producer_signature),
        "producer_sequence":                req.producer_sequence as i64,
    });

    let returned_tx_id = post_provider_reported::post(
        pool,
        PostProviderReportedInput {
            transaction: transaction_json,
            reservation_id,
            provider_amount: &provider_amount,
            pricing: pricing_json,
            audit_outbox_row: audit_outbox_json,
        },
    )
    .await?;

    if returned_tx_id == ledger_transaction_id {
        let success = build_success(pool, ledger_transaction_id, reservation_id).await?;
        return Ok(ProviderReportResponse {
            outcome: Some(Outcome::Success(success)),
        });
    }

    debug!(returned = %returned_tx_id, "idempotent replay hit");
    let replay = build_replay(pool, returned_tx_id, &decision_id, &reservation_id).await?;
    Ok(ProviderReportResponse {
        outcome: Some(Outcome::Replay(replay)),
    })
}

// ---- helpers ---------------------------------------------------------------

fn validate(req: &ProviderReportRequest) -> Result<(), DomainError> {
    if req.tenant_id.is_empty() {
        return Err(DomainError::InvalidRequest("tenant_id required".into()));
    }
    if req.reservation_id.is_empty() {
        return Err(DomainError::InvalidRequest("reservation_id required".into()));
    }
    if req.decision_id.is_empty() {
        return Err(DomainError::InvalidRequest("decision_id required".into()));
    }
    if req.provider_reported_amount_atomic.is_empty() {
        return Err(DomainError::InvalidRequest(
            "provider_reported_amount_atomic required".into(),
        ));
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

/// Canonical request_hash for ProviderReport (Codex round 1 P1.3 + P2.1):
///   * INCLUDES tenant + reservation + amount + unit + pricing 4 fields +
///     provider_response_metadata (canonical namespaced provenance string
///     per Step 8 v3 POC convention) + operation_kind.
///   * EXCLUDES producer_sequence, audit_event ids, timestamps,
///     fencing.epoch, decision_id (caller-derived; may legitimately
///     differ across receiver versions).
fn canonical_request_hash(
    req: &ProviderReportRequest,
    provider_amount: &BigInt,
    caller_hash: &[u8],
) -> Result<[u8; 32], DomainError> {
    let mut h = Sha256::new();
    h.update(b"v1:provider_report:business_intent:");
    h.update(req.tenant_id.as_bytes());
    h.update(b"|reservation|");
    h.update(req.reservation_id.as_bytes());
    h.update(b"|provider_amount|");
    h.update(provider_amount.to_string().as_bytes());
    h.update(b"|unit|");
    if let Some(u) = &req.unit {
        h.update(u.unit_id.as_bytes());
    }
    h.update(b"|pricing|");
    if let Some(p) = &req.pricing {
        h.update(p.pricing_version.as_bytes());
        h.update(&p.price_snapshot_hash);
        h.update(p.fx_rate_version.as_bytes());
        h.update(p.unit_conversion_version.as_bytes());
    }
    h.update(b"|provenance|");
    h.update(req.provider_response_metadata.as_bytes());

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

fn minimal_replay_seed(tx: &Uuid, decision: &Uuid, reservation: &Uuid) -> Value {
    json!({
        "ledger_transaction_id": tx.to_string(),
        "decision_id":           decision.to_string(),
        "operation_kind":        "provider_report",
        "reservation_ids":       vec![reservation.to_string()],
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

async fn build_success(
    pool: &PgPool,
    ledger_transaction_id: Uuid,
    reservation_id: Uuid,
) -> Result<CommitSuccess, DomainError> {
    let now = chrono::Utc::now();
    // Read commits row (deterministic commit_id from Step 7) for the
    // current delta_to_reserved_atomic value. SP UPDATEs this with the
    // new value (provider - original_reserved) before returning.
    let commit_id = derive_commit_id(&reservation_id);
    let row = sqlx::query(
        "SELECT delta_to_reserved_atomic::TEXT AS delta, latest_state \
           FROM commits WHERE commit_id = $1",
    )
    .bind(commit_id)
    .fetch_one(pool)
    .await
    .map_err(|e| DomainError::Internal(anyhow::anyhow!("commits lookup: {e}")))?;
    let delta: String = row.try_get("delta").unwrap_or_default();
    Ok(CommitSuccess {
        ledger_transaction_id: ledger_transaction_id.to_string(),
        reservation_id: reservation_id.to_string(),
        latest_state: CommitState::ProviderReported as i32,
        delta_to_reserved_atomic: delta,
        recorded_at: Some(Timestamp {
            seconds: now.timestamp(),
            nanos: now.timestamp_subsec_nanos() as i32,
        }),
    })
}

async fn build_replay(
    pool: &PgPool,
    existing_tx_id: Uuid,
    _retry_decision_id: &Uuid,
    reservation_id: &Uuid,
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
        derive_commit_id(reservation_id).to_string(),
        original_decision_id.to_string(),
        vec![reservation_id.to_string()],
        None,
        recorded_at,
        &posting_state,
    ))
}

/// Same derivation as Step 7 commit_estimated handler — commits row uses
/// the deterministic ID; ProviderReport UPDATEs it (no new row).
fn derive_commit_id(reservation_id: &Uuid) -> Uuid {
    let mut h = Sha256::new();
    h.update(reservation_id.to_string().as_bytes());
    h.update(b":commit_estimated");
    let bytes: [u8; 32] = h.finalize().into();
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&bytes[..16]);
    Uuid::from_bytes(buf)
}
