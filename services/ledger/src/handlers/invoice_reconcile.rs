//! `Ledger::InvoiceReconcile` handler (Phase 2B Step 9).
//!
//! Wire contract: `proto/spendguard/ledger/v1/ledger.proto` InvoiceReconcile.
//!
//! Spec references:
//!   - Contract DSL §5  (commitStateMachine: any_to_invoice_reconciled;
//!                       reconciliationStrategy: invoice_priority)
//!   - Ledger §10       (audit.outcome captures FINAL state)
//!   - Stage 2 §0.2 D9  (Provider Webhook Receiver = only provider entry)
//!   - Stage 2 §0.2 D12 (audit.outcome strictly after audit.decision per
//!                       decision_id)
//!   - Stage 2 §8.2.3   (webhook flow: dedup by provider event id)
//!
//! Design source: `/tmp/codex-step9-r7.txt` (v7 LOCKED after 7 Codex rounds).
//!
//! Dual-row audit pattern:
//!   * Caller signs ONE audit.outcome event (FINAL close) and passes the
//!     outcome producer_sequence (= N+1 where N is the decision seq).
//!   * Handler synthesizes ONE audit.decision event (server-minted):
//!     - audit_event_id = sha256(outcome_id::TEXT || ":decision")[0..16]::UUID
//!     - producer_sequence = outcome_seq - 1
//!     - signing_key_id = "ledger-server-mint:v1" (POC sentinel; signature empty)
//!     - All other CloudEvent fields copied from outcome (source, runid,
//!       producer_id, time, schema_bundle_id) for cross-row consistency.
//!   * SP cross-validates payload-vs-column for both rows, then writes
//!     2 audit_outbox + 2 audit_outbox_global_keys (with idempotency_key
//!     suffixes ":decision" / ":outcome") + ledger + projection update.
//!
//! POC limitations (documented):
//!   1. invoice_amount > original_reserved → rejected (overrun_debt path
//!      deferred; Step 9 doesn't fully close FINAL state under overrun).
//!   2. tolerance_micros: 10000 (fiat) interpreted as 0 atomic for token
//!      unit; production needs unit-aware fiat conversion.
//!   3. signing_key_id "ledger-server-mint:v1" is forward-looking; POC
//!      works because canonical_ingest strict signatures globally disabled.
//!   4. ledger_transactions.audit_decision_event_id anchors to OUTCOME
//!      row (mirror Step 7); QueryDecisionOutcome RPC retrieves dual chain.
//!   5. unknown → invoice_reconciled is unreachable in POC.
//!   6. Caller MUST pre-allocate 2 contiguous producer_sequence values
//!      (decision = N, outcome = N+1) in its workload-instance space and
//!      pass N+1 (outcome) via wire's producer_sequence field.

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
    persistence::post_invoice_reconciled::{self, PostInvoiceReconciledInput},
    proto::ledger::v1::{
        invoice_reconcile_response::Outcome, CommitState, CommitSuccess, InvoiceReconcileRequest,
        InvoiceReconcileResponse,
    },
};

#[instrument(skip(pool, signer, req), fields(
    tenant = %req.tenant_id,
    reservation_id = %req.reservation_id,
    decision_id = %req.decision_id,
))]
pub async fn handle(
    pool: &PgPool,
    signer: &dyn spendguard_signing::Signer,
    req: InvoiceReconcileRequest,
) -> Result<InvoiceReconcileResponse, tonic::Status> {
    match handle_inner(pool, signer, req).await {
        Ok(resp) => Ok(resp),
        Err(DomainError::Internal(e)) => Err(tonic::Status::internal(e.to_string())),
        Err(DomainError::Db(e)) => Err(tonic::Status::unavailable(format!("db: {}", e))),
        Err(other) => {
            let proto_err = other.to_proto();
            Ok(InvoiceReconcileResponse {
                outcome: Some(Outcome::Error(proto_err)),
            })
        }
    }
}

async fn handle_inner(
    pool: &PgPool,
    signer: &dyn spendguard_signing::Signer,
    req: InvoiceReconcileRequest,
) -> Result<InvoiceReconcileResponse, DomainError> {
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

    let outcome_audit_event = req
        .audit_event
        .as_ref()
        .ok_or_else(|| DomainError::InvalidRequest("audit_event required".into()))?;
    // v7 invariant: caller signs the OUTCOME row (FINAL close); SP/handler
    // mints decision row internally.
    if outcome_audit_event.r#type != "spendguard.audit.outcome" {
        return Err(DomainError::InvalidRequest(format!(
            "audit_event.type must be spendguard.audit.outcome; got '{}'",
            outcome_audit_event.r#type
        )));
    }
    if outcome_audit_event.decision_id != req.decision_id {
        return Err(DomainError::InvalidRequest(
            "audit_event.decision_id must match request.decision_id".into(),
        ));
    }
    let outcome_audit_event_id = parse_uuid(&outcome_audit_event.id, "audit_event.id")?;

    // v7 Δ2: outcome producer_sequence must be >= 2 to back-derive decision seq.
    // Codex challenge P2.2: narrow uint64 → i64 safely (NUMERIC fits i64).
    let outcome_seq: i64 = i64::try_from(req.producer_sequence).map_err(|_| {
        DomainError::InvalidRequest(format!(
            "producer_sequence {} exceeds i64::MAX",
            req.producer_sequence
        ))
    })?;
    if outcome_seq < 2 {
        return Err(DomainError::InvalidRequest(format!(
            "producer_sequence must be >= 2 to back-derive decision seq; got {}",
            outcome_seq
        )));
    }
    let decision_seq: i64 = outcome_seq
        .checked_sub(1)
        .expect("outcome_seq >= 2 enforced above");

    let invoice_amount = req
        .invoice_reconciled_amount_atomic
        .parse::<BigInt>()
        .map_err(|e| {
            DomainError::InvalidRequest(format!("invoice_reconciled_amount_atomic invalid: {e}"))
        })?;
    if invoice_amount.sign() != num_bigint::Sign::Plus {
        return Err(DomainError::InvalidRequest(
            "invoice_reconciled_amount_atomic must be > 0".into(),
        ));
    }

    let unit = req
        .unit
        .as_ref()
        .ok_or_else(|| DomainError::InvalidRequest("unit missing".into()))?;
    if unit.unit_id.is_empty() {
        return Err(DomainError::InvalidRequest("unit.unit_id empty".into()));
    }

    if req.provider_invoice_id.is_empty() {
        return Err(DomainError::InvalidRequest(
            "provider_invoice_id required (Step 9 webhook namespacing)".into(),
        ));
    }

    // Canonical request_hash. INCLUDES tenant + reservation + invoice_amount
    // + unit + pricing 4 fields + provider_invoice_id + op_kind. EXCLUDES
    // producer_sequence, audit ids, timestamps, fencing.epoch.
    let request_hash = canonical_request_hash(&req, &invoice_amount, &idempotency.request_hash)?;

    let ledger_transaction_id = Uuid::now_v7();

    // v7 derived decision event_id: sha256(outcome_id::TEXT || ":decision")[0..16]
    let derived_decision_event_id = derive_decision_event_id(&outcome_audit_event_id);

    // Synthesize decision CloudEvent payload (matches extract_cloudevent_payload
    // flat shape; data_b64 = base64 of minimal JSON; signing_key_id sentinel;
    // producer_sequence = outcome_seq - 1; everything else copied from outcome).
    let decision_data_json = json!({
        "transition":     "any -> invoice_reconciled (server-minted decision)",
        "decision_id":    decision_id.to_string(),
        "reservation_id": reservation_id.to_string(),
        "operation_kind": "invoice_reconcile",
    });
    let decision_data_bytes = serde_json::to_vec(&decision_data_json)
        .map_err(|e| DomainError::Internal(anyhow::anyhow!("decision data serialize: {e}")))?;
    let decision_data_b64 = base64::engine::general_purpose::STANDARD.encode(&decision_data_bytes);

    let decision_audit_outbox_id = Uuid::now_v7();
    let outcome_audit_outbox_id = Uuid::now_v7();

    let outcome_payload = extract_cloudevent_payload(outcome_audit_event)?;

    // Phase 5 GA hardening S6: ledger signs the server-minted decision
    // row with its own producer signer. signing_key_id reflects the
    // ledger's key; the JSON canonical form (sorted by serde_json key
    // order) is what we sign over. A successor slice (S8) bridges the
    // ledger's JSON canonical form and the sidecar's proto canonical
    // form into a single verifier.
    let mut decision_payload = json!({
        "specversion":     outcome_audit_event.specversion,
        "type":            "spendguard.audit.decision",
        "source":          outcome_audit_event.source,
        "id":              derived_decision_event_id.to_string(),
        "time_seconds":    outcome_audit_event.time.as_ref().map(|t| t.seconds).unwrap_or_default(),
        "time_nanos":      outcome_audit_event.time.as_ref().map(|t| t.nanos).unwrap_or_default(),
        "datacontenttype": "application/json",
        "data_b64":        decision_data_b64,
        "tenantid":        outcome_audit_event.tenant_id,
        "runid":           outcome_audit_event.run_id,
        "decisionid":      outcome_audit_event.decision_id,
        "schema_bundle_id": outcome_audit_event.schema_bundle_id,
        "producer_id":     outcome_audit_event.producer_id,
        "producer_sequence": decision_seq,
        "signing_key_id":  signer.key_id(),
    });
    let decision_canonical_bytes = serde_json::to_vec(&decision_payload).map_err(|e| {
        DomainError::Internal(anyhow::anyhow!("decision payload canonical serialize: {e}"))
    })?;
    let decision_signature = signer
        .sign(&decision_canonical_bytes)
        .await
        .map_err(|e| {
            DomainError::Internal(anyhow::anyhow!(
                "ledger sign synthesized decision row: {e}"
            ))
        })?;
    // Echo the actual signing_key_id back into the payload so the
    // GENERATED columns added by migration 0024 see the correct value.
    decision_payload["signing_key_id"] = json!(decision_signature.key_id);

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
        // v7: ledger_tx audit anchor → OUTCOME row id (mirror Step 7).
        "audit_decision_event_id": outcome_audit_event_id.to_string(),
        "fencing_scope_id":        fencing.scope_id,
        "fencing_epoch":           fencing.epoch as i64,
        "workload_instance_id":    fencing.workload_instance_id,
        "effective_at":             chrono::Utc::now().to_rfc3339(),
        "unit_id":                 unit.unit_id,
        "minimal_replay_response": minimal_replay_seed(&ledger_transaction_id, &decision_id, &reservation_id),
    });

    let audit_decision_outbox_json = json!({
        "audit_outbox_id":                  decision_audit_outbox_id.to_string(),
        "audit_decision_event_id":          derived_decision_event_id.to_string(),
        "event_type":                       "spendguard.audit.decision",
        "cloudevent_payload":               decision_payload,
        // S6: signed by the ledger's producer signer above.
        "cloudevent_payload_signature_hex": hex::encode(&decision_signature.bytes),
    });

    let audit_outcome_outbox_json = json!({
        "audit_outbox_id":                  outcome_audit_outbox_id.to_string(),
        "audit_decision_event_id":          outcome_audit_event_id.to_string(),
        "event_type":                       "spendguard.audit.outcome",
        "cloudevent_payload":               outcome_payload,
        "cloudevent_payload_signature_hex": hex::encode(&outcome_audit_event.producer_signature),
    });

    let returned_tx_id = post_invoice_reconciled::post(
        pool,
        PostInvoiceReconciledInput {
            transaction: transaction_json,
            reservation_id,
            invoice_amount: &invoice_amount,
            pricing: pricing_json,
            audit_decision_outbox_row: audit_decision_outbox_json,
            audit_outcome_outbox_row: audit_outcome_outbox_json,
            outcome_producer_seq: outcome_seq,
        },
    )
    .await?;

    if returned_tx_id == ledger_transaction_id {
        let success = build_success(pool, ledger_transaction_id, reservation_id).await?;
        return Ok(InvoiceReconcileResponse {
            outcome: Some(Outcome::Success(success)),
        });
    }

    debug!(returned = %returned_tx_id, "idempotent replay hit");
    let replay = build_replay(pool, returned_tx_id, &decision_id, &reservation_id).await?;
    Ok(InvoiceReconcileResponse {
        outcome: Some(Outcome::Replay(replay)),
    })
}

// ---- helpers ---------------------------------------------------------------

fn validate(req: &InvoiceReconcileRequest) -> Result<(), DomainError> {
    if req.tenant_id.is_empty() {
        return Err(DomainError::InvalidRequest("tenant_id required".into()));
    }
    if req.reservation_id.is_empty() {
        return Err(DomainError::InvalidRequest("reservation_id required".into()));
    }
    if req.decision_id.is_empty() {
        return Err(DomainError::InvalidRequest("decision_id required".into()));
    }
    if req.invoice_reconciled_amount_atomic.is_empty() {
        return Err(DomainError::InvalidRequest(
            "invoice_reconciled_amount_atomic required".into(),
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

/// Canonical request_hash for InvoiceReconcile (v7 design):
///   * INCLUDES tenant + reservation + invoice_amount + unit + pricing 4
///     fields + provider_invoice_id (canonical namespaced provenance) +
///     operation_kind.
///   * EXCLUDES producer_sequence, audit_event ids, timestamps,
///     fencing.epoch, decision_id.
fn canonical_request_hash(
    req: &InvoiceReconcileRequest,
    invoice_amount: &BigInt,
    caller_hash: &[u8],
) -> Result<[u8; 32], DomainError> {
    let mut h = Sha256::new();
    h.update(b"v1:invoice_reconcile:business_intent:");
    h.update(req.tenant_id.as_bytes());
    h.update(b"|reservation|");
    h.update(req.reservation_id.as_bytes());
    h.update(b"|invoice_amount|");
    h.update(invoice_amount.to_string().as_bytes());
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
    h.update(req.provider_invoice_id.as_bytes());

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

/// Derive the audit.decision event_id deterministically from the outcome
/// event_id: sha256(outcome_id::TEXT || ":decision")[0..16] → UUID.
/// Mirrors the SP's pgsql derivation in 0016 (encode/digest/substring).
fn derive_decision_event_id(outcome_id: &Uuid) -> Uuid {
    let mut h = Sha256::new();
    h.update(outcome_id.to_string().as_bytes());
    h.update(b":decision");
    let bytes: [u8; 32] = h.finalize().into();
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&bytes[..16]);
    Uuid::from_bytes(buf)
}

fn minimal_replay_seed(tx: &Uuid, decision: &Uuid, reservation: &Uuid) -> Value {
    json!({
        "ledger_transaction_id": tx.to_string(),
        "decision_id":           decision.to_string(),
        "operation_kind":        "invoice_reconcile",
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
        latest_state: CommitState::InvoiceReconciled as i32,
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
/// the deterministic ID; InvoiceReconcile UPDATEs it (no new row).
fn derive_commit_id(reservation_id: &Uuid) -> Uuid {
    let mut h = Sha256::new();
    h.update(reservation_id.to_string().as_bytes());
    h.update(b":commit_estimated");
    let bytes: [u8; 32] = h.finalize().into();
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&bytes[..16]);
    Uuid::from_bytes(buf)
}
