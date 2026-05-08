//! `Ledger::ReserveSet` handler (v2 — Codex challenge patches applied).
//!
//! Wire contract: `proto/spendguard/ledger/v1/ledger.proto`.
//! Spec references:
//!   - Stage 2 §4 (audit_outbox + sync replica durability)
//!   - Stage 2 §8.2.1 (ReserveSet wire), §8.2.1.1 (lock_order_token)
//!   - Contract §6 (decision transaction stages 4-5)
//!   - Contract §4 (reservationSet all_or_nothing)
//!   - Contract §7 (reservation TTL)
//!   - Ledger §3 (per-unit balance), §5.5 (reservations projection),
//!     §6.3 (post_ledger_transaction), §13 (pricing freeze)
//!
//! Authority model:
//!   * The Postgres stored procedure `post_ledger_transaction` is the SOLE
//!     authority on idempotent replay, fencing CAS, lock-order canonicalization,
//!     pricing validation, balance assertion, and audit_outbox atomicity.
//!   * The handler MUST NOT pre-check `ledger_transactions` for replay; doing
//!     so allows TOCTOU between handler and proc.
//!   * Caller-supplied `request_hash` is treated as authoritative; mismatches
//!     raised by the proc surface as `IdempotencyConflict`.

use base64::Engine as _;
use prost_types::Timestamp;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use tracing::{debug, instrument};
use uuid::Uuid;

use crate::{
    domain::{error::DomainError, lock_order, minimal_replay},
    persistence::post_transaction::{self, PostTransactionInput},
    proto::{
        common::v1::{BudgetClaim, PricingFreeze},
        ledger::v1::{
            reserve_set_response::Outcome, ReserveSetRequest, ReserveSetResponse, ReserveSetSuccess,
            Reservation,
        },
    },
};

#[instrument(skip(pool, req), fields(
    tenant = %req.tenant_id,
    decision_id = %req.decision_id,
    audit_event_id = %req.audit_decision_event_id,
    claim_count = req.claims.len()
))]
pub async fn handle(
    pool: &PgPool,
    req: ReserveSetRequest,
) -> Result<ReserveSetResponse, tonic::Status> {
    match handle_inner(pool, req).await {
        Ok(resp) => Ok(resp),
        Err(DomainError::Internal(e)) => Err(tonic::Status::internal(e.to_string())),
        Err(DomainError::Db(e)) => Err(tonic::Status::unavailable(format!("db: {}", e))),
        Err(other) => {
            // Surface domain errors via the proto Error variant rather than
            // gRPC Status; clients should distinguish business errors from
            // transport faults.
            let proto_err = other.to_proto();
            Ok(ReserveSetResponse {
                outcome: Some(Outcome::Error(proto_err)),
            })
        }
    }
}

async fn handle_inner(
    pool: &PgPool,
    req: ReserveSetRequest,
) -> Result<ReserveSetResponse, DomainError> {
    // -- 1. Validate request -----------------------------------------------
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
    let request_hash = canonical_request_hash(&req, idempotency)?;

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
    // Brand-new fencing scopes default to current_epoch=0; a properly-
    // acquired lease has incremented at least once. Reject 0 here so the
    // proc never sees it (defense-in-depth — proc also rejects it).
    if fencing.epoch == 0 {
        return Err(DomainError::FencingEpochStale(
            "epoch 0 is not a valid lease".into(),
        ));
    }

    let ttl_expires_at = req
        .ttl_expires_at
        .as_ref()
        .ok_or_else(|| DomainError::InvalidRequest("ttl_expires_at missing".into()))?;

    let pricing = req
        .pricing
        .as_ref()
        .ok_or_else(|| DomainError::InvalidRequest("pricing missing".into()))?;
    if pricing.pricing_version.is_empty() || pricing.price_snapshot_hash.is_empty() {
        return Err(DomainError::InvalidRequest(
            "pricing.pricing_version + price_snapshot_hash required".into(),
        ));
    }

    // -- 2. Lock order token derivation / validation ---------------------
    let derived_lock_order = lock_order::derive(&req.claims);
    let caller_lock_order = req.lock_order_token.as_ref().map(|t| t.value.clone());
    if let Some(c) = &caller_lock_order {
        if c != &derived_lock_order {
            return Err(DomainError::LockOrderTokenMismatch(format!(
                "caller={}, derived={}",
                c, derived_lock_order
            )));
        }
    }

    // -- 3. Generate handler-side ids -------------------------------------
    let ledger_transaction_id = Uuid::now_v7();
    let audit_outbox_id = Uuid::now_v7();

    // Reservation IDs are deterministic per (decision_id, claim ordinal): if
    // the request is replayed (idempotent), any client-side derivation should
    // recompute the same ids; we don't rely on this in code (replay returns
    // Replay variant which carries the canonical ids), but the determinism
    // helps debugging.
    let reservation_ids: Vec<Uuid> = req
        .claims
        .iter()
        .enumerate()
        .map(|(i, _)| derive_reservation_id(&decision_id, i))
        .collect();

    // -- 4. Build payloads for stored procedure ---------------------------
    let entries_json = build_entries_payload(
        &req.claims,
        &reservation_ids,
        pricing,
    )?;
    let reservations_json =
        build_reservations_payload(&req.claims, &reservation_ids, ttl_expires_at, &idempotency.key)?;

    let transaction_json = json!({
        "ledger_transaction_id": ledger_transaction_id.to_string(),
        "tenant_id":             tenant_id.to_string(),
        "operation_kind":        "reserve",
        "idempotency_key":       idempotency.key,
        "request_hash_hex":      hex::encode(request_hash),
        "decision_id":           decision_id.to_string(),
        "audit_decision_event_id": audit_event_id.to_string(),
        "fencing_scope_id":      fencing.scope_id,
        "fencing_epoch":         fencing.epoch as i64,
        "workload_instance_id":  fencing.workload_instance_id,
        "effective_at":          chrono::Utc::now().to_rfc3339(),
        "minimal_replay_response": minimal_replay_seed(
            &ledger_transaction_id,
            &decision_id,
            &reservation_ids,
            ttl_expires_at,
        ),
    });

    let audit_outbox_json = json!({
        "audit_outbox_id":               audit_outbox_id.to_string(),
        "event_type":                    "spendguard.audit.decision",
        "cloudevent_payload":             extract_cloudevent_payload(&req)?,
        "cloudevent_payload_signature_hex": extract_cloudevent_signature_hex(&req),
        "producer_sequence":             req.producer_sequence as i64,
    });

    // -- 5. Invoke stored procedure (sole authority for idempotency) -----
    let returned_tx_id = post_transaction::post(
        pool,
        PostTransactionInput {
            transaction: transaction_json,
            entries: entries_json,
            reservations: reservations_json,
            audit_outbox_row: audit_outbox_json,
            caller_lock_token: caller_lock_order.as_deref(),
        },
    )
    .await?;

    // -- 6. Branch on new tx vs idempotent replay ------------------------
    if returned_tx_id == ledger_transaction_id {
        return Ok(success_response(
            &req,
            ledger_transaction_id,
            audit_event_id,
            ttl_expires_at.clone(),
            &idempotency.key,
            &reservation_ids,
            &derived_lock_order,
        ));
    }

    debug!(returned = %returned_tx_id, "idempotent replay hit");
    let replay = build_replay_response(pool, returned_tx_id, &decision_id).await?;
    Ok(ReserveSetResponse {
        outcome: Some(Outcome::Replay(replay)),
    })
}

// ---- helpers ---------------------------------------------------------------

fn validate(req: &ReserveSetRequest) -> Result<(), DomainError> {
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
    if req.claims.is_empty() {
        return Err(DomainError::InvalidRequest("claims must not be empty".into()));
    }
    if req.audit_event.is_none() {
        return Err(DomainError::InvalidRequest(
            "audit_event (CloudEvent) required for audit_outbox row".into(),
        ));
    }
    for (i, claim) in req.claims.iter().enumerate() {
        if claim.budget_id.is_empty() {
            return Err(DomainError::InvalidRequest(format!(
                "claim[{}].budget_id empty",
                i
            )));
        }
        let unit = claim
            .unit
            .as_ref()
            .ok_or_else(|| DomainError::InvalidRequest(format!("claim[{}].unit missing", i)))?;
        if unit.unit_id.is_empty() {
            return Err(DomainError::InvalidRequest(format!(
                "claim[{}].unit.unit_id empty",
                i
            )));
        }
        if claim.window_instance_id.is_empty() {
            return Err(DomainError::InvalidRequest(format!(
                "claim[{}].window_instance_id empty",
                i
            )));
        }
        // amount_atomic must parse as non-negative integer.
        let amount = claim.amount_atomic.parse::<num_bigint::BigInt>().map_err(|e| {
            DomainError::InvalidRequest(format!(
                "claim[{}].amount_atomic invalid: {}",
                i, e
            ))
        })?;
        if amount.sign() == num_bigint::Sign::Minus {
            return Err(DomainError::InvalidRequest(format!(
                "claim[{}].amount_atomic must be non-negative",
                i
            )));
        }
    }
    Ok(())
}

fn parse_uuid(s: &str, field: &str) -> Result<Uuid, DomainError> {
    Uuid::parse_str(s)
        .map_err(|e| DomainError::InvalidRequest(format!("{}: invalid uuid ({})", field, e)))
}

/// Canonical request hash for idempotency replay (per Ledger §7).
///
/// Hashes ONLY the *business intent* fields whose change should be treated
/// as a different request: tenant, claims, pricing freeze, fencing scope
/// (id + epoch — caller workload identity is captured separately), and
/// the contract bundle reference. Sidecar-minted technical IDs
/// (`decision_id`, `audit_decision_event_id`, `producer_sequence`) and
/// time-varying envelope fields (`ttl_expires_at`, `audit_event.id`,
/// `audit_event.time`, `audit_event.data` snapshot, etc.) are deliberately
/// EXCLUDED — the same logical request retried after a sidecar restart
/// will mint fresh values for those, and the spec requires that retry to
/// collapse to the original ledger row, not to fail with
/// IdempotencyConflict.
///
/// If the caller supplied a non-empty `Idempotency.request_hash`, it MUST
/// match the canonical hash; mismatch is treated as a replay-with-
/// different-body conflict (Ledger §7).
fn canonical_request_hash(
    req: &ReserveSetRequest,
    idempotency: &crate::proto::common::v1::Idempotency,
) -> Result<[u8; 32], DomainError> {
    let mut h = Sha256::new();
    h.update(b"v1:reserve_set:business_intent:");
    h.update(req.tenant_id.as_bytes());

    // Claims (canonical order — sort to neutralize wire-order ambiguity).
    let mut sorted_claims: Vec<&BudgetClaim> = req.claims.iter().collect();
    sorted_claims.sort_by(|a, b| {
        let au = a.unit.as_ref().map(|u| u.unit_id.as_str()).unwrap_or("");
        let bu = b.unit.as_ref().map(|u| u.unit_id.as_str()).unwrap_or("");
        (
            a.budget_id.as_str(),
            au,
            a.window_instance_id.as_str(),
            a.direction,
        )
            .cmp(&(
                b.budget_id.as_str(),
                bu,
                b.window_instance_id.as_str(),
                b.direction,
            ))
    });
    for c in sorted_claims {
        h.update(b"|claim|");
        h.update(c.budget_id.as_bytes());
        let unit_id = c.unit.as_ref().map(|u| u.unit_id.as_str()).unwrap_or("");
        h.update(unit_id.as_bytes());
        h.update(c.amount_atomic.as_bytes());
        h.update(&[c.direction as u8]);
        h.update(c.window_instance_id.as_bytes());
    }

    // Pricing freeze (all 4 fields — same content = same intent).
    if let Some(p) = &req.pricing {
        h.update(b"|pricing|");
        h.update(p.pricing_version.as_bytes());
        h.update(&p.price_snapshot_hash);
        h.update(p.fx_rate_version.as_bytes());
        h.update(p.unit_conversion_version.as_bytes());
    }

    // Fencing scope identity + epoch. Different epoch = different writer
    // generation = legitimately different request even with same idempotency
    // key (the prior owner's reservation is fenced out).
    if let Some(f) = &req.fencing {
        h.update(b"|fencing|");
        h.update(f.epoch.to_be_bytes());
        h.update(f.scope_id.as_bytes());
    }

    // Contract bundle reference (different bundle = different policy =
    // legitimately different request).
    if let Some(b) = &req.contract_bundle {
        h.update(b"|contract_bundle|");
        h.update(b.bundle_id.as_bytes());
        h.update(&b.bundle_hash);
    }

    // Lock order token: when caller supplies it, include in canonical so a
    // mismatched token is detected as different intent. When server-derived,
    // it's a function of claims, already canonicalized above.
    if let Some(t) = &req.lock_order_token {
        if !t.value.is_empty() {
            h.update(b"|lock_order_token|");
            h.update(t.value.as_bytes());
        }
    }

    let canonical: [u8; 32] = h.finalize().into();

    // If caller supplied request_hash, it must match canonical exactly.
    if !idempotency.request_hash.is_empty() {
        if idempotency.request_hash.len() != 32 {
            return Err(DomainError::InvalidRequest(
                "Idempotency.request_hash must be 32 bytes (sha256)".into(),
            ));
        }
        if idempotency.request_hash.as_ref() != canonical {
            return Err(DomainError::IdempotencyConflict);
        }
    }
    Ok(canonical)
}

fn build_entries_payload(
    claims: &[BudgetClaim],
    reservation_ids: &[Uuid],
    pricing: &PricingFreeze,
) -> Result<Value, DomainError> {
    // Each claim → two ledger_entries (debit available_budget; credit reserved_hold).
    // Stored proc resolves ledger_account_id by JOIN on
    // (tenant_id, budget_id, window_instance_id, unit_id, account_kind).
    let mut entries = Vec::with_capacity(claims.len() * 2);
    let pricing_obj = json!({
        "pricing_version":         pricing.pricing_version,
        "price_snapshot_hash_hex": hex::encode(&pricing.price_snapshot_hash),
        "fx_rate_version":         pricing.fx_rate_version,
        "unit_conversion_version": pricing.unit_conversion_version,
    });

    for (i, claim) in claims.iter().enumerate() {
        let unit = claim
            .unit
            .as_ref()
            .ok_or_else(|| DomainError::InvalidRequest("claim.unit missing".into()))?;
        let reservation_id = reservation_ids[i];

        for kind in ["available_budget", "reserved_hold"] {
            let direction = match kind {
                "available_budget" => "debit",
                "reserved_hold" => "credit",
                _ => unreachable!(),
            };
            entries.push(json!({
                "ledger_entry_id":   Uuid::now_v7().to_string(),
                "budget_id":         claim.budget_id,
                "window_instance_id": claim.window_instance_id,
                "unit_id":           unit.unit_id,
                "account_kind":      kind,
                "direction":         direction,
                "amount_atomic":     claim.amount_atomic,
                "ledger_shard_id":   1,
                "pricing_version":         pricing_obj["pricing_version"],
                "price_snapshot_hash_hex": pricing_obj["price_snapshot_hash_hex"],
                "fx_rate_version":         pricing_obj["fx_rate_version"],
                "unit_conversion_version": pricing_obj["unit_conversion_version"],
                "reservation_id":    reservation_id.to_string(),
            }));
        }
    }
    Ok(Value::Array(entries))
}

fn build_reservations_payload(
    claims: &[BudgetClaim],
    reservation_ids: &[Uuid],
    ttl: &Timestamp,
    idempotency_key: &str,
) -> Result<Value, DomainError> {
    let ttl_iso = chrono::DateTime::<chrono::Utc>::from_timestamp(ttl.seconds, ttl.nanos as u32)
        .ok_or_else(|| DomainError::InvalidRequest("ttl_expires_at out of range".into()))?
        .to_rfc3339();

    let arr: Vec<Value> = claims
        .iter()
        .zip(reservation_ids.iter())
        .map(|(c, rid)| {
            json!({
                "reservation_id":     rid.to_string(),
                "budget_id":          c.budget_id,
                "window_instance_id": c.window_instance_id,
                "ttl_expires_at":     ttl_iso,
                "idempotency_key":    idempotency_key,
            })
        })
        .collect();
    Ok(Value::Array(arr))
}

fn derive_reservation_id(decision_id: &Uuid, ordinal: usize) -> Uuid {
    let mut h = Sha256::new();
    h.update(decision_id.as_bytes());
    h.update(b":res:");
    h.update(ordinal.to_be_bytes());
    let bytes: [u8; 32] = h.finalize().into();
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&bytes[..16]);
    buf[6] = (buf[6] & 0x0f) | 0x40; // v4
    buf[8] = (buf[8] & 0x3f) | 0x80;
    Uuid::from_bytes(buf)
}

fn derive_reservation_set_id(decision_id: &Uuid) -> Uuid {
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

fn extract_cloudevent_payload(req: &ReserveSetRequest) -> Result<Value, DomainError> {
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

fn extract_cloudevent_signature_hex(req: &ReserveSetRequest) -> String {
    req.audit_event
        .as_ref()
        .map(|e| hex::encode(&e.producer_signature))
        .unwrap_or_default()
}

/// Build the JSONB stored in `ledger_transactions.minimal_replay_response`.
///
/// On Replay, `build_replay_response` reads this JSON to surface the same
/// projection_ids (in claim-ordinal order) and ttl_expires_at the original
/// Success response carried — so the loser of a concurrent same-key race
/// returns a byte-equivalent response to the winner.
fn minimal_replay_seed(
    tx: &Uuid,
    decision: &Uuid,
    reservation_ids: &[Uuid],
    ttl_expires_at: &Timestamp,
) -> Value {
    let ttl_iso = chrono::DateTime::<chrono::Utc>::from_timestamp(
        ttl_expires_at.seconds,
        ttl_expires_at.nanos as u32,
    )
    .map(|t| t.to_rfc3339())
    .unwrap_or_default();
    let ids_arr: Vec<Value> = reservation_ids
        .iter()
        .map(|id| Value::String(id.to_string()))
        .collect();
    json!({
        "ledger_transaction_id": tx.to_string(),
        "decision_id":           decision.to_string(),
        "operation_kind":        "reserve",
        // Canonical ordered projection ids (claim-ordinal).
        "reservation_ids":       Value::Array(ids_arr),
        // ISO-8601 UTC timestamp for the ttl_expires_at anchor.
        "ttl_expires_at":        ttl_iso,
    })
}

fn success_response(
    req: &ReserveSetRequest,
    ledger_transaction_id: Uuid,
    audit_event_id: Uuid,
    ttl_expires_at: Timestamp,
    idempotency_key: &str,
    reservation_ids: &[Uuid],
    lock_order_token: &str,
) -> ReserveSetResponse {
    let now = chrono::Utc::now();
    let reservations: Vec<Reservation> = req
        .claims
        .iter()
        .zip(reservation_ids.iter())
        .map(|(c, rid)| Reservation {
            reservation_id: rid.to_string(),
            budget_id: c.budget_id.clone(),
            window_instance_id: c.window_instance_id.clone(),
            unit: c.unit.clone(),
            amount_atomic: c.amount_atomic.clone(),
            ttl_expires_at: Some(ttl_expires_at.clone()),
            idempotency_key: idempotency_key.to_string(),
            current_state: crate::proto::ledger::v1::reservation::State::Reserved as i32,
        })
        .collect();

    let success = ReserveSetSuccess {
        ledger_transaction_id: ledger_transaction_id.to_string(),
        reservation_set_id: derive_reservation_set_id(&parse_uuid(&req.decision_id, "_").unwrap_or(Uuid::nil()))
            .to_string(),
        reservations,
        audit_decision_event_id: audit_event_id.to_string(),
        producer_sequence: req.producer_sequence,
        lock_order_token: Some(crate::proto::common::v1::LockOrderToken {
            value: lock_order_token.to_string(),
        }),
        full: None,
        recorded_at: Some(Timestamp {
            seconds: now.timestamp(),
            nanos: now.timestamp_subsec_nanos() as i32,
        }),
    };

    ReserveSetResponse {
        outcome: Some(Outcome::Success(success)),
    }
}

async fn build_replay_response(
    pool: &PgPool,
    existing_tx_id: Uuid,
    _retry_decision_id: &Uuid,
) -> Result<crate::proto::common::v1::Replay, DomainError> {
    // Replay MUST surface the ORIGINAL row's identifiers (not the retry's
    // freshly-minted ones), per Contract §6 / Ledger §7 idempotency. We
    // therefore read from `ledger_transactions.minimal_replay_response` —
    // a JSONB snapshot stored at first-success that captures the
    // canonical-order reservation_ids and the original ttl_expires_at.
    // This is preferred over querying the `reservations` projection table
    // because (a) it preserves claim-ordinal order without a schema
    // ordinal column, and (b) it captures the original wallclock TTL so
    // the loser of a concurrent same-key race surfaces the same TTL the
    // winner did.
    let row = sqlx::query(
        "SELECT operation_kind, audit_decision_event_id, decision_id, \
                posting_state, recorded_at, minimal_replay_response \
           FROM ledger_transactions \
          WHERE ledger_transaction_id = $1",
    )
    .bind(existing_tx_id)
    .fetch_one(pool)
    .await
    .map_err(|e| DomainError::Internal(anyhow::anyhow!("replay lookup: {}", e)))?;

    let op_kind: String = row.get("operation_kind");
    let posting_state: String = row.get("posting_state");
    let recorded_at: chrono::DateTime<chrono::Utc> = row.get("recorded_at");

    // Fail-closed on NULL identifiers. The handler always supplies both on
    // INSERT, so a NULL here means schema drift or a direct-SQL row from
    // outside the handler — replaying as a nil UUID would silently corrupt
    // the audit chain.
    let audit_id: Uuid = row
        .try_get::<Option<Uuid>, _>("audit_decision_event_id")
        .map_err(|e| DomainError::Internal(anyhow::anyhow!("replay lookup audit_id: {}", e)))?
        .ok_or_else(|| DomainError::Internal(anyhow::anyhow!(
            "replay row {} has NULL audit_decision_event_id",
            existing_tx_id
        )))?;
    let original_decision_id: Uuid = row
        .try_get::<Option<Uuid>, _>("decision_id")
        .map_err(|e| DomainError::Internal(anyhow::anyhow!("replay lookup decision_id: {}", e)))?
        .ok_or_else(|| DomainError::Internal(anyhow::anyhow!(
            "replay row {} has NULL decision_id",
            existing_tx_id
        )))?;

    // Derive reservation_set_id from the ORIGINAL decision_id so retries
    // see the same operation_id the first call returned.
    let reservation_set_id = derive_reservation_set_id(&original_decision_id);

    // Pull canonical reservation_ids + original ttl_expires_at from the
    // JSONB snapshot. minimal_replay_response is written in the same tx
    // as ledger_transactions, so it's always consistent with the row.
    let snapshot: Value = row
        .try_get::<Option<Value>, _>("minimal_replay_response")
        .map_err(|e| DomainError::Internal(anyhow::anyhow!("replay snapshot: {}", e)))?
        .unwrap_or(Value::Null);

    let projection_ids: Vec<String> = snapshot
        .get("reservation_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let ttl_expires_at = snapshot
        .get("ttl_expires_at")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| Timestamp {
            seconds: dt.timestamp(),
            nanos: dt.timestamp_subsec_nanos() as i32,
        });

    Ok(minimal_replay::from_db_row(
        existing_tx_id.to_string(),
        &op_kind,
        audit_id.to_string(),
        reservation_set_id.to_string(),
        original_decision_id.to_string(),
        projection_ids,
        ttl_expires_at,
        recorded_at,
        &posting_state,
    ))
}
