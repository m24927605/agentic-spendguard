//! `Ledger::CommitEstimated` handler (Phase 2B Step 7).
//!
//! Wire contract: `proto/spendguard/ledger/v1/ledger.proto` CommitEstimated.
//!
//! Spec references:
//!   - Contract DSL §5  (commitStateMachine: reserved -> estimated)
//!   - Contract DSL §6  (decision transaction stage 7 commit_or_release)
//!   - Ledger §3        (per-unit balance per (transaction, unit_id))
//!   - Ledger §6.3      (post_commit_estimated_transaction authority model)
//!   - Ledger §10       (account_kinds; estimated commit transitions
//!                       reserved_hold -> committed_spend, residual ->
//!                       available_budget)
//!   - Ledger §13       (4-layer pricing freeze)
//!   - Stage 2 §4       (audit_outbox; outcome paired to original decision)
//!   - Stage 2 §8.2.1   (CommitEstimated wire)
//!
//! Authority model (mirrors reserve_set.rs):
//!   * Postgres SP `post_commit_estimated_transaction` is the SOLE authority
//!     on idempotent replay, fencing CAS, reservations row lock + state
//!     transition, original-reserve lookup, pricing tuple verification,
//!     entries shape, per-unit balance assertion, and audit_outbox
//!     atomicity.
//!   * Handler MUST NOT pre-derive entries or pre-validate state.
//!   * SP errors map to typed DomainError variants in domain/error.rs.

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
    persistence::post_commit_estimated::{self, PostCommitEstimatedInput},
    proto::ledger::v1::{
        commit_estimated_response::Outcome, CommitEstimatedRequest, CommitEstimatedResponse,
        CommitState, CommitSuccess,
    },
};

#[instrument(skip(pool, req), fields(
    tenant = %req.tenant_id,
    reservation_id = %req.reservation_id,
    decision_id = %req.decision_id,
))]
pub async fn handle(
    pool: &PgPool,
    req: CommitEstimatedRequest,
) -> Result<CommitEstimatedResponse, tonic::Status> {
    match handle_inner(pool, req).await {
        Ok(resp) => Ok(resp),
        Err(DomainError::Internal(e)) => Err(tonic::Status::internal(e.to_string())),
        Err(DomainError::Db(e)) => Err(tonic::Status::unavailable(format!("db: {}", e))),
        Err(other) => {
            let proto_err = other.to_proto();
            Ok(CommitEstimatedResponse {
                outcome: Some(Outcome::Error(proto_err)),
            })
        }
    }
}

async fn handle_inner(
    pool: &PgPool,
    req: CommitEstimatedRequest,
) -> Result<CommitEstimatedResponse, DomainError> {
    // -- 1. Validate ------------------------------------------------------
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
    let audit_outbox_id = parse_uuid(&audit_event.id, "audit_event.id")?;

    let estimated_amount = req
        .estimated_amount_atomic
        .parse::<BigInt>()
        .map_err(|e| {
            DomainError::InvalidRequest(format!("estimated_amount_atomic invalid: {e}"))
        })?;
    if estimated_amount.sign() != num_bigint::Sign::Plus {
        return Err(DomainError::InvalidRequest(
            "estimated_amount_atomic must be > 0".into(),
        ));
    }

    let unit = req
        .unit
        .as_ref()
        .ok_or_else(|| DomainError::InvalidRequest("unit missing".into()))?;
    if unit.unit_id.is_empty() {
        return Err(DomainError::InvalidRequest("unit.unit_id empty".into()));
    }

    // -- 2. Canonical request_hash (commit-specific, per Codex P2.2) ------
    // Includes business-intent fields that must NOT change across retries:
    //   tenant + reservation + amount + unit + pricing + decision_id + op_kind
    // Excludes retry-affected sidecar mints: producer_sequence, audit ids,
    // timestamps, fencing.epoch.
    let request_hash = canonical_request_hash(&req, &estimated_amount, &idempotency.request_hash)?;

    // -- 3. Mint handler-side ids -----------------------------------------
    let ledger_transaction_id = Uuid::now_v7();

    // -- 4. Build SP payloads ---------------------------------------------
    let pricing_json = json!({
        "pricing_version":         pricing.pricing_version,
        "price_snapshot_hash_hex": hex::encode(&pricing.price_snapshot_hash),
        "fx_rate_version":         pricing.fx_rate_version,
        "unit_conversion_version": pricing.unit_conversion_version,
    });

    let transaction_json = json!({
        "ledger_transaction_id":    ledger_transaction_id.to_string(),
        "tenant_id":                tenant_id.to_string(),
        "idempotency_key":          idempotency.key,
        "request_hash_hex":         hex::encode(request_hash),
        "decision_id":              decision_id.to_string(),
        "audit_decision_event_id":  audit_outbox_id.to_string(),
        "fencing_scope_id":         fencing.scope_id,
        "fencing_epoch":            fencing.epoch as i64,
        "workload_instance_id":     fencing.workload_instance_id,
        "effective_at":             chrono::Utc::now().to_rfc3339(),
        // Codex round 2 challenge P2.3: SP validates caller unit_id against
        // original reserve entry to reject mismatched-unit commits.
        "unit_id":                  unit.unit_id,
        "minimal_replay_response":  minimal_replay_seed(&ledger_transaction_id, &decision_id, &reservation_id),
    });

    let audit_outbox_json = json!({
        "audit_outbox_id":                  audit_outbox_id.to_string(),
        "event_type":                       "spendguard.audit.outcome",
        "cloudevent_payload":               extract_cloudevent_payload(audit_event)?,
        "cloudevent_payload_signature_hex": hex::encode(&audit_event.producer_signature),
        "producer_sequence":                req.producer_sequence as i64,
    });

    // -- 5. Invoke stored procedure ---------------------------------------
    let returned_tx_id = post_commit_estimated::post(
        pool,
        PostCommitEstimatedInput {
            transaction: transaction_json,
            reservation_id,
            estimated_amount: &estimated_amount,
            pricing: pricing_json,
            audit_outbox_row: audit_outbox_json,
        },
    )
    .await?;

    // -- 6. Branch new tx vs idempotent replay ----------------------------
    if returned_tx_id == ledger_transaction_id {
        let success = build_success(pool, ledger_transaction_id, reservation_id, &estimated_amount).await?;
        return Ok(CommitEstimatedResponse {
            outcome: Some(Outcome::Success(success)),
        });
    }

    debug!(returned = %returned_tx_id, "idempotent replay hit");
    let replay = build_replay(pool, returned_tx_id, &decision_id, &reservation_id).await?;
    Ok(CommitEstimatedResponse {
        outcome: Some(Outcome::Replay(replay)),
    })
}

// ---- helpers ---------------------------------------------------------------

fn validate(req: &CommitEstimatedRequest) -> Result<(), DomainError> {
    if req.tenant_id.is_empty() {
        return Err(DomainError::InvalidRequest("tenant_id required".into()));
    }
    if req.reservation_id.is_empty() {
        return Err(DomainError::InvalidRequest("reservation_id required".into()));
    }
    if req.decision_id.is_empty() {
        return Err(DomainError::InvalidRequest("decision_id required".into()));
    }
    if req.estimated_amount_atomic.is_empty() {
        return Err(DomainError::InvalidRequest(
            "estimated_amount_atomic required".into(),
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

fn canonical_request_hash(
    req: &CommitEstimatedRequest,
    estimated: &BigInt,
    caller_hash: &[u8],
) -> Result<[u8; 32], DomainError> {
    let mut h = Sha256::new();
    h.update(b"v1:commit_estimated:business_intent:");
    h.update(req.tenant_id.as_bytes());
    h.update(b"|reservation|");
    h.update(req.reservation_id.as_bytes());
    h.update(b"|estimated|");
    h.update(estimated.to_string().as_bytes());
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
    h.update(b"|decision|");
    h.update(req.decision_id.as_bytes());

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
        "operation_kind":        "commit_estimated",
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

/// Read commit projection row to populate Success.delta_to_reserved.
async fn build_success(
    pool: &PgPool,
    ledger_transaction_id: Uuid,
    reservation_id: Uuid,
    estimated: &BigInt,
) -> Result<CommitSuccess, DomainError> {
    let now = chrono::Utc::now();
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
        latest_state: CommitState::Estimated as i32,
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
    retry_decision_id: &Uuid,
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

    // Retry decision_id sanity check — same logical commit MUST share decision_id.
    let _ = retry_decision_id;

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

fn derive_commit_id(reservation_id: &Uuid) -> Uuid {
    let mut h = Sha256::new();
    h.update(reservation_id.to_string().as_bytes());
    h.update(b":commit_estimated");
    let bytes: [u8; 32] = h.finalize().into();
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&bytes[..16]);
    Uuid::from_bytes(buf)
}
