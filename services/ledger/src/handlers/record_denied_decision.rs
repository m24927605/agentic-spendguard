//! `Ledger::RecordDeniedDecision` handler (Phase 3 wedge).
//!
//! Wire contract: `proto/spendguard/ledger/v1/ledger.proto`.
//!
//! Spec references:
//!   - Contract §6.1 「無 audit 則無 effect」 invariant — DENY is also an
//!     effect (the «no reservation» effect) and MUST audit.
//!   - Stage 2 §4 (audit_outbox + sync replica durability)
//!
//! Authority model: SP `post_denied_decision_transaction` is the sole
//! authority on idempotent replay + fencing CAS. Handler MUST NOT
//! pre-check `ledger_transactions` (TOCTOU).

use base64::Engine as _;
use prost_types::Timestamp;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tracing::{debug, instrument};
use uuid::Uuid;

use crate::{
    domain::error::DomainError,
    persistence::post_transaction::{
        self, PostApprovalRequiredInput, PostDeniedInput,
    },
    proto::{
        common::v1::BudgetClaim,
        ledger::v1::{
            record_denied_decision_response::Outcome, RecordDeniedDecisionRequest,
            RecordDeniedDecisionResponse, RecordDeniedDecisionSuccess,
        },
    },
};

#[instrument(skip(pool, req), fields(
    tenant = %req.tenant_id,
    decision_id = %req.decision_id,
    audit_event_id = %req.audit_decision_event_id,
    final_decision = %req.final_decision,
    matched_rules = req.matched_rule_ids.len()
))]
pub async fn handle(
    pool: &PgPool,
    req: RecordDeniedDecisionRequest,
) -> Result<RecordDeniedDecisionResponse, tonic::Status> {
    match handle_inner(pool, req).await {
        Ok(resp) => Ok(resp),
        Err(DomainError::Internal(e)) => Err(tonic::Status::internal(e.to_string())),
        Err(DomainError::Db(e)) => Err(tonic::Status::unavailable(format!("db: {}", e))),
        Err(other) => {
            let proto_err = other.to_proto();
            Ok(RecordDeniedDecisionResponse {
                outcome: Some(Outcome::Error(proto_err)),
            })
        }
    }
}

async fn handle_inner(
    pool: &PgPool,
    req: RecordDeniedDecisionRequest,
) -> Result<RecordDeniedDecisionResponse, DomainError> {
    validate(&req)?;

    let tenant_id = parse_uuid(&req.tenant_id, "tenant_id")?;
    let decision_id = parse_uuid(&req.decision_id, "decision_id")?;
    let audit_event_id = parse_uuid(&req.audit_decision_event_id, "audit_decision_event_id")?;

    let idempotency = req
        .idempotency
        .as_ref()
        .ok_or_else(|| DomainError::InvalidRequest("idempotency missing".into()))?;
    if idempotency.key.is_empty() {
        return Err(DomainError::InvalidRequest("idempotency.key empty".into()));
    }
    let request_hash = canonical_request_hash(&req)?;

    let fencing = req
        .fencing
        .as_ref()
        .ok_or_else(|| DomainError::InvalidRequest("fencing missing".into()))?;
    if fencing.scope_id.is_empty() {
        return Err(DomainError::InvalidRequest("fencing.scope_id empty".into()));
    }
    if fencing.workload_instance_id.is_empty() {
        return Err(DomainError::InvalidRequest(
            "fencing.workload_instance_id empty".into(),
        ));
    }
    if fencing.epoch == 0 {
        return Err(DomainError::FencingEpochStale(
            "epoch 0 is not a valid lease".into(),
        ));
    }

    let ledger_transaction_id = Uuid::now_v7();
    let audit_outbox_id = Uuid::now_v7();

    let lock_order_token = derive_denied_lock_order_token(&decision_id);

    let transaction_json = json!({
        "ledger_transaction_id":   ledger_transaction_id.to_string(),
        "tenant_id":               tenant_id.to_string(),
        "operation_kind":          "denied_decision",
        "idempotency_key":         idempotency.key,
        "request_hash_hex":        hex::encode(request_hash),
        "decision_id":              decision_id.to_string(),
        "audit_decision_event_id": audit_event_id.to_string(),
        "fencing_scope_id":        fencing.scope_id,
        "fencing_epoch":           fencing.epoch as i64,
        "workload_instance_id":    fencing.workload_instance_id,
        "effective_at":            chrono::Utc::now().to_rfc3339(),
        "lock_order_token":        lock_order_token,
        "minimal_replay_response": minimal_replay_seed(&ledger_transaction_id, &audit_event_id),
    });

    let audit_outbox_json = json!({
        "audit_outbox_id":               audit_outbox_id.to_string(),
        "event_type":                    "spendguard.audit.decision",
        "cloudevent_payload":             extract_cloudevent_payload(&req)?,
        "cloudevent_payload_signature_hex": extract_cloudevent_signature_hex(&req),
        "producer_sequence":             req.producer_sequence as i64,
    });

    // Round-2 #9 producer SP. When REQUIRE_APPROVAL fires with both
    // JSON payloads present, route through the new SP that writes
    // approval_requests + audit_outbox atomically. Other denial
    // kinds (STOP / DEGRADE / SKIP) take the original
    // post_denied_decision_transaction path.
    let want_approval_sp = req.final_decision == "REQUIRE_APPROVAL"
        && !req.decision_context_json.is_empty()
        && !req.requested_effect_json.is_empty();

    let producer_sequence = req.producer_sequence;

    if want_approval_sp {
        let decision_context_value: Value = serde_json::from_slice(&req.decision_context_json)
            .map_err(|e| {
                DomainError::InvalidRequest(format!(
                    "decision_context_json: invalid JSON ({e})"
                ))
            })?;
        let requested_effect_value: Value = serde_json::from_slice(&req.requested_effect_json)
            .map_err(|e| {
                DomainError::InvalidRequest(format!(
                    "requested_effect_json: invalid JSON ({e})"
                ))
            })?;

        let approval_ttl_seconds = if req.approval_ttl_seconds == 0 {
            3600
        } else {
            req.approval_ttl_seconds as i32
        };

        let out = post_transaction::post_approval_required(
            pool,
            PostApprovalRequiredInput {
                transaction: transaction_json,
                audit_outbox_row: audit_outbox_json,
                decision_context: decision_context_value,
                requested_effect: requested_effect_value,
                approval_ttl_seconds,
            },
        )
        .await?;

        if out.was_first_insert && out.ledger_transaction_id == ledger_transaction_id {
            return Ok(RecordDeniedDecisionResponse {
                outcome: Some(Outcome::Success(RecordDeniedDecisionSuccess {
                    ledger_transaction_id: ledger_transaction_id.to_string(),
                    audit_decision_event_id: audit_event_id.to_string(),
                    producer_sequence,
                    recorded_at: Some(now_ts()),
                    approval_id: out.approval_id.to_string(),
                })),
            });
        }

        debug!(
            returned = %out.ledger_transaction_id,
            approval = %out.approval_id,
            was_first = out.was_first_insert,
            "approval_required idempotent replay hit"
        );
        let replay = build_replay_response(pool, out.ledger_transaction_id).await?;
        return Ok(RecordDeniedDecisionResponse {
            outcome: Some(Outcome::Replay(replay)),
        });
    }

    let returned_tx_id = post_transaction::post_denied(
        pool,
        PostDeniedInput {
            transaction: transaction_json,
            audit_outbox_row: audit_outbox_json,
        },
    )
    .await?;

    if returned_tx_id == ledger_transaction_id {
        return Ok(RecordDeniedDecisionResponse {
            outcome: Some(Outcome::Success(RecordDeniedDecisionSuccess {
                ledger_transaction_id: ledger_transaction_id.to_string(),
                audit_decision_event_id: audit_event_id.to_string(),
                producer_sequence,
                recorded_at: Some(now_ts()),
                approval_id: String::new(),
            })),
        });
    }

    debug!(returned = %returned_tx_id, "denied_decision idempotent replay hit");
    let replay = build_replay_response(pool, returned_tx_id).await?;
    Ok(RecordDeniedDecisionResponse {
        outcome: Some(Outcome::Replay(replay)),
    })
}

fn validate(req: &RecordDeniedDecisionRequest) -> Result<(), DomainError> {
    if req.tenant_id.is_empty() {
        return Err(DomainError::InvalidRequest("tenant_id required".into()));
    }
    if req.decision_id.is_empty() {
        return Err(DomainError::InvalidRequest("decision_id required".into()));
    }
    if req.audit_decision_event_id.is_empty() {
        return Err(DomainError::InvalidRequest(
            "audit_decision_event_id required".into(),
        ));
    }
    if req.audit_event.is_none() {
        return Err(DomainError::InvalidRequest(
            "audit_event (CloudEvent) required for audit_outbox row".into(),
        ));
    }
    if req.final_decision.is_empty() {
        return Err(DomainError::InvalidRequest("final_decision required".into()));
    }
    // CONTINUE belongs on the reserve path; refuse it here so audit
    // forensics never sees a denied_decision row claiming CONTINUE.
    if req.final_decision == "CONTINUE" {
        return Err(DomainError::InvalidRequest(
            "RecordDeniedDecision rejects CONTINUE; route to ReserveSet".into(),
        ));
    }
    if req.matched_rule_ids.is_empty() {
        return Err(DomainError::InvalidRequest(
            "matched_rule_ids must be non-empty when DENY".into(),
        ));
    }
    Ok(())
}

fn parse_uuid(s: &str, field: &str) -> Result<Uuid, DomainError> {
    Uuid::parse_str(s)
        .map_err(|e| DomainError::InvalidRequest(format!("{}: invalid uuid ({})", field, e)))
}

fn canonical_request_hash(req: &RecordDeniedDecisionRequest) -> Result<[u8; 32], DomainError> {
    let mut h = Sha256::new();
    h.update(b"v1:denied_decision:business_intent:");
    h.update(req.tenant_id.as_bytes());

    // Attempted claims (canonical order).
    let mut sorted: Vec<&BudgetClaim> = req.attempted_claims.iter().collect();
    sorted.sort_by(|a, b| {
        let au = a.unit.as_ref().map(|u| u.unit_id.as_str()).unwrap_or("");
        let bu = b.unit.as_ref().map(|u| u.unit_id.as_str()).unwrap_or("");
        (a.budget_id.as_str(), au, a.amount_atomic.as_str())
            .cmp(&(b.budget_id.as_str(), bu, b.amount_atomic.as_str()))
    });
    for c in sorted {
        h.update(c.budget_id.as_bytes());
        h.update(b":");
        if let Some(u) = &c.unit {
            h.update(u.unit_id.as_bytes());
        }
        h.update(b":");
        h.update(c.amount_atomic.as_bytes());
        h.update(b":");
        h.update(c.window_instance_id.as_bytes());
        h.update(b"|");
    }
    h.update(b"|rules:");
    let mut rules: Vec<&str> = req.matched_rule_ids.iter().map(|s| s.as_str()).collect();
    rules.sort_unstable();
    for r in rules {
        h.update(r.as_bytes());
        h.update(b",");
    }
    h.update(b"|reasons:");
    let mut reasons: Vec<&str> = req.reason_codes.iter().map(|s| s.as_str()).collect();
    reasons.sort_unstable();
    for r in reasons {
        h.update(r.as_bytes());
        h.update(b",");
    }
    h.update(b"|decision:");
    h.update(req.final_decision.as_bytes());
    if let Some(b) = &req.contract_bundle {
        h.update(b"|bundle_id:");
        h.update(b.bundle_id.as_bytes());
        h.update(b"|bundle_hash:");
        h.update(&b.bundle_hash);
    }
    if let Some(p) = &req.pricing {
        h.update(b"|pricing_version:");
        h.update(p.pricing_version.as_bytes());
        h.update(b"|price_snapshot:");
        h.update(&p.price_snapshot_hash);
    }
    if let Some(f) = &req.fencing {
        h.update(b"|scope:");
        h.update(f.scope_id.as_bytes());
    }

    Ok(h.finalize().into())
}

fn extract_cloudevent_payload(req: &RecordDeniedDecisionRequest) -> Result<Value, DomainError> {
    let evt = req
        .audit_event
        .as_ref()
        .ok_or_else(|| DomainError::InvalidRequest("audit_event required".into()))?;
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

fn extract_cloudevent_signature_hex(req: &RecordDeniedDecisionRequest) -> String {
    req.audit_event
        .as_ref()
        .map(|e| hex::encode(&e.producer_signature))
        .unwrap_or_default()
}

fn minimal_replay_seed(tx: &Uuid, audit: &Uuid) -> Value {
    json!({
        "ledger_transaction_id":   tx.to_string(),
        "audit_decision_event_id": audit.to_string(),
    })
}

fn derive_denied_lock_order_token(decision_id: &Uuid) -> String {
    // DENY touches no ledger accounts, so there's no real lock order.
    // We still need a unique non-empty token to satisfy the
    // ledger_transactions.lock_order_token NOT NULL constraint.
    let mut h = Sha256::new();
    h.update(b"v1:denied_decision:");
    h.update(decision_id.as_bytes());
    format!("v1:denied:{}", hex::encode(h.finalize()))
}

fn now_ts() -> Timestamp {
    let now = chrono::Utc::now();
    Timestamp {
        seconds: now.timestamp(),
        nanos: now.timestamp_subsec_nanos() as i32,
    }
}

async fn build_replay_response(
    pool: &PgPool,
    tx_id: Uuid,
) -> Result<crate::proto::common::v1::Replay, DomainError> {
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT audit_decision_event_id, decision_id, recorded_at, posting_state
         FROM ledger_transactions
         WHERE ledger_transaction_id = $1
           AND operation_kind = 'denied_decision'",
    )
    .bind(tx_id)
    .fetch_one(pool)
    .await
    .map_err(crate::domain::error::map_pg_error)?;

    let audit_event_id: Option<Uuid> = row.get("audit_decision_event_id");
    let decision_id: Option<Uuid> = row.get("decision_id");
    let recorded_at: chrono::DateTime<chrono::Utc> = row.get("recorded_at");
    let posting_state: String = row.get("posting_state");

    let status_code = match posting_state.as_str() {
        "posted" => crate::proto::common::v1::replay::StatusCode::Posted,
        "voided" => crate::proto::common::v1::replay::StatusCode::Voided,
        "pending" => crate::proto::common::v1::replay::StatusCode::Pending,
        _ => crate::proto::common::v1::replay::StatusCode::Unspecified,
    };

    Ok(crate::proto::common::v1::Replay {
        ledger_transaction_id: tx_id.to_string(),
        operation_kind: "denied_decision".to_string(),
        audit_decision_event_id: audit_event_id
            .map(|u| u.to_string())
            .unwrap_or_default(),
        recorded_at: Some(Timestamp {
            seconds: recorded_at.timestamp(),
            nanos: recorded_at.timestamp_subsec_nanos() as i32,
        }),
        operation_id: tx_id.to_string(),
        status_code: status_code as i32,
        decision_id: decision_id.map(|u| u.to_string()).unwrap_or_default(),
        projection_ids: vec![],
        ttl_expires_at: None,
    })
}
