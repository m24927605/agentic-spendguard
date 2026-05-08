//! POST /v1/webhook/{provider} — main entry point.
//!
//! Flow (v3 §Δ3):
//!   1. HMAC verify signature header against raw body
//!   2. Parse body + validate fields
//!   3. Compute canonical hash (byte-exact with ledger handler)
//!   4. Dedupe pre-check
//!   5. Allocate producer_sequence(s)
//!   6. Build CloudEvent + ledger gRPC request
//!   7. Call ledger
//!   8. Handle Success / Replay / Error / IdempotencyConflict
//!   9. Insert dedupe on success
//!  10. Return HTTP response

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use base64::Engine as _;
use hmac::{Hmac, Mac};
use num_bigint::BigInt;
use prost_types::Timestamp;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::{
    domain::{canonical_hash, error::ledger_code_to_http, error::ReceiverError},
    persistence::{dedupe, sequence::SequenceAllocator},
    proto::{
        common::v1::{CloudEvent, Fencing, Idempotency, PricingFreeze, UnitRef},
        ledger::v1::{
            invoice_reconcile_response, provider_report_response, InvoiceReconcileRequest,
            ProviderReportRequest,
        },
    },
    server::AppState,
};

type HmacSha256 = Hmac<Sha256>;

const ALLOWED_PROVIDERS: &[&str] = &["mock-llm"];

#[derive(Debug, Deserialize)]
struct WebhookBody {
    event_kind: String,
    tenant_id: String,
    provider_account: String,
    provider_event_id: String,
    reservation_id: String,
    amount_atomic: String,
    unit_id: String,
    pricing: PricingBody,
}

#[derive(Debug, Deserialize)]
struct PricingBody {
    pricing_version: String,
    price_snapshot_hash_hex: String,
    fx_rate_version: String,
    unit_conversion_version: String,
}

#[derive(Debug, serde::Serialize)]
pub struct WebhookResponse {
    pub ledger_transaction_id: String,
    pub outcome: &'static str,
}

pub async fn handle_webhook(
    Path(provider): Path<String>,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    match handle_inner(&provider, &state, &headers, &body).await {
        Ok(resp) => resp.into_response(),
        Err(e) => {
            let status = e.status();
            let msg = e.to_string();
            warn!(provider = %provider, status = %status.as_u16(), error = %msg, "webhook error");
            (status, Json(json!({ "error": msg }))).into_response()
        }
    }
}

async fn handle_inner(
    provider: &str,
    state: &AppState,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<(StatusCode, Json<WebhookResponse>), ReceiverError> {
    // 1. provider allow-list (Codex r2 V2.5)
    if !ALLOWED_PROVIDERS.contains(&provider) {
        return Err(ReceiverError::UnknownProvider);
    }

    // 2. Signature verify
    let sig_hex = headers
        .get("x-spendguard-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ReceiverError::Unauthorized("missing X-SpendGuard-Signature".into()))?;

    let sig_bytes = hex::decode(sig_hex)
        .map_err(|_| ReceiverError::Unauthorized("signature not hex".into()))?;

    let secret = secret_for_provider(state, provider)?;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| ReceiverError::Internal(anyhow::anyhow!("hmac init: {e}")))?;
    mac.update(body);
    let expected = mac.finalize().into_bytes();

    if expected.ct_eq(&sig_bytes).unwrap_u8() != 1 {
        return Err(ReceiverError::Unauthorized("signature mismatch".into()));
    }

    // 3. Parse body
    let parsed: WebhookBody = serde_json::from_slice(body)
        .map_err(|e| ReceiverError::InvalidRequest(format!("body json: {e}")))?;

    if parsed.tenant_id.is_empty()
        || parsed.provider_account.is_empty()
        || parsed.provider_event_id.is_empty()
        || parsed.reservation_id.is_empty()
        || parsed.amount_atomic.is_empty()
        || parsed.unit_id.is_empty()
        || parsed.pricing.pricing_version.is_empty()
        || parsed.pricing.price_snapshot_hash_hex.is_empty()
        || parsed.pricing.fx_rate_version.is_empty()
        || parsed.pricing.unit_conversion_version.is_empty()
    {
        return Err(ReceiverError::InvalidRequest(
            "missing required field".into(),
        ));
    }

    let tenant_uuid = Uuid::parse_str(&parsed.tenant_id)
        .map_err(|e| ReceiverError::InvalidRequest(format!("tenant_id: {e}")))?;
    Uuid::parse_str(&parsed.reservation_id)
        .map_err(|e| ReceiverError::InvalidRequest(format!("reservation_id: {e}")))?;
    Uuid::parse_str(&parsed.unit_id)
        .map_err(|e| ReceiverError::InvalidRequest(format!("unit_id: {e}")))?;

    // Codex challenge P2.2: validate amount fits NUMERIC(38,0) at receiver
    // before paying for the ledger round-trip (oversized values would
    // surface as Postgres 22003 → CODE_UNSPECIFIED → opaque 502).
    if parsed.amount_atomic.is_empty()
        || parsed.amount_atomic.len() > 38
        || !parsed
            .amount_atomic
            .chars()
            .all(|c| c.is_ascii_digit())
    {
        return Err(ReceiverError::InvalidRequest(
            "amount_atomic must be 1-38 ASCII digits (NUMERIC(38,0))".into(),
        ));
    }
    let amount: BigInt = parsed
        .amount_atomic
        .parse()
        .map_err(|e| ReceiverError::InvalidRequest(format!("amount_atomic: {e}")))?;
    if amount.sign() != num_bigint::Sign::Plus {
        return Err(ReceiverError::InvalidRequest(
            "amount_atomic must be > 0".into(),
        ));
    }

    let price_snapshot_hash = hex::decode(&parsed.pricing.price_snapshot_hash_hex)
        .map_err(|e| ReceiverError::InvalidRequest(format!("price_snapshot_hash_hex: {e}")))?;
    if price_snapshot_hash.is_empty() {
        return Err(ReceiverError::InvalidRequest(
            "price_snapshot_hash_hex must be non-empty".into(),
        ));
    }

    // 4. Build provenance + decision_id
    let event_kind = parsed.event_kind.as_str();
    let op_kind = match event_kind {
        "provider_report" => "provider_report",
        "invoice_reconcile" => "invoice_reconcile",
        _ => return Err(ReceiverError::InvalidRequest(format!(
            "unknown event_kind: {}",
            event_kind
        ))),
    };
    let provenance = format!(
        "{}:{}:{}:{}",
        op_kind, provider, parsed.provider_account, parsed.provider_event_id
    );
    let decision_id = derive_decision_id_v4(&provenance);

    // 5. Canonical hash (BYTE-EXACT match handler)
    let canonical = match event_kind {
        "provider_report" => canonical_hash::provider_report_hash(
            &parsed.tenant_id,
            &parsed.reservation_id,
            &amount,
            &parsed.unit_id,
            &parsed.pricing.pricing_version,
            &price_snapshot_hash,
            &parsed.pricing.fx_rate_version,
            &parsed.pricing.unit_conversion_version,
            &provenance,
        ),
        "invoice_reconcile" => canonical_hash::invoice_reconcile_hash(
            &parsed.tenant_id,
            &parsed.reservation_id,
            &amount,
            &parsed.unit_id,
            &parsed.pricing.pricing_version,
            &price_snapshot_hash,
            &parsed.pricing.fx_rate_version,
            &parsed.pricing.unit_conversion_version,
            &provenance,
        ),
        _ => unreachable!(),
    };

    // 6. Dedupe pre-check
    let existing = dedupe::lookup(
        &state.pg,
        provider,
        event_kind,
        &parsed.provider_account,
        &parsed.provider_event_id,
    )
    .await?;
    if let Some(row) = existing {
        if row.canonical_hash == canonical {
            return Ok((
                StatusCode::OK,
                Json(WebhookResponse {
                    ledger_transaction_id: row.ledger_transaction_id.to_string(),
                    outcome: "replay",
                }),
            ));
        }
        return Err(ReceiverError::Conflict);
    }

    // 7. Allocate producer_sequence
    let request_outer = match event_kind {
        "provider_report" => {
            let seq = state.seq.next_one();
            let ce = build_cloudevent(
                state,
                CloudEventKind::Decision,
                provider,
                &parsed,
                &decision_id,
                seq,
                Some(&amount),
            )?;
            let req = ProviderReportRequest {
                tenant_id: parsed.tenant_id.clone(),
                reservation_id: parsed.reservation_id.clone(),
                provider_reported_amount_atomic: parsed.amount_atomic.clone(),
                unit: Some(UnitRef {
                    unit_id: parsed.unit_id.clone(),
                    ..Default::default()
                }),
                provider_response_metadata: provenance.clone(),
                idempotency: Some(Idempotency {
                    key: provenance.clone(),
                    request_hash: canonical.to_vec().into(),
                }),
                fencing: Some(Fencing {
                    epoch: state.config.fencing_initial_epoch as u64,
                    scope_id: state.config.fencing_scope_id.clone(),
                    workload_instance_id: state.config.workload_instance_id.clone(),
                }),
                pricing: Some(PricingFreeze {
                    pricing_version: parsed.pricing.pricing_version.clone(),
                    price_snapshot_hash: price_snapshot_hash.clone().into(),
                    fx_rate_version: parsed.pricing.fx_rate_version.clone(),
                    unit_conversion_version: parsed.pricing.unit_conversion_version.clone(),
                }),
                audit_event: Some(ce),
                producer_sequence: seq,
                decision_id: decision_id.to_string(),
            };
            RequestOuter::ProviderReport(req)
        }
        "invoice_reconcile" => {
            let (_decision_seq, outcome_seq) = state.seq.next_block(2);
            let ce = build_cloudevent(
                state,
                CloudEventKind::Outcome,
                provider,
                &parsed,
                &decision_id,
                outcome_seq,
                Some(&amount),
            )?;
            let req = InvoiceReconcileRequest {
                tenant_id: parsed.tenant_id.clone(),
                provider_invoice_id: provenance.clone(),
                invoice_reconciled_amount_atomic: parsed.amount_atomic.clone(),
                unit: Some(UnitRef {
                    unit_id: parsed.unit_id.clone(),
                    ..Default::default()
                }),
                reservation_id: parsed.reservation_id.clone(),
                idempotency: Some(Idempotency {
                    key: provenance.clone(),
                    request_hash: canonical.to_vec().into(),
                }),
                pricing: Some(PricingFreeze {
                    pricing_version: parsed.pricing.pricing_version.clone(),
                    price_snapshot_hash: price_snapshot_hash.clone().into(),
                    fx_rate_version: parsed.pricing.fx_rate_version.clone(),
                    unit_conversion_version: parsed.pricing.unit_conversion_version.clone(),
                }),
                audit_event: Some(ce),
                producer_sequence: outcome_seq,
                decision_id: decision_id.to_string(),
                fencing: Some(Fencing {
                    epoch: state.config.fencing_initial_epoch as u64,
                    scope_id: state.config.fencing_scope_id.clone(),
                    workload_instance_id: state.config.workload_instance_id.clone(),
                }),
            };
            RequestOuter::InvoiceReconcile(req)
        }
        _ => unreachable!(),
    };

    // 8. Call ledger gRPC
    let mut client = state.ledger_client.clone();
    let response = match request_outer {
        RequestOuter::ProviderReport(req) => match client.provider_report(req).await {
            Ok(resp) => OneofResp::ProviderReport(resp.into_inner().outcome),
            Err(status) => OneofResp::GrpcError(status),
        },
        RequestOuter::InvoiceReconcile(req) => match client.invoice_reconcile(req).await {
            Ok(resp) => OneofResp::InvoiceReconcile(resp.into_inner().outcome),
            Err(status) => OneofResp::GrpcError(status),
        },
    };

    let (status, ledger_tx_id, outcome): (StatusCode, Uuid, &'static str) = match response {
        OneofResp::ProviderReport(Some(provider_report_response::Outcome::Success(s))) => {
            let id = Uuid::parse_str(&s.ledger_transaction_id)
                .map_err(|e| ReceiverError::Internal(anyhow::anyhow!("tx_id parse: {e}")))?;
            (StatusCode::OK, id, "success")
        }
        OneofResp::ProviderReport(Some(provider_report_response::Outcome::Replay(r))) => {
            let id = Uuid::parse_str(&r.ledger_transaction_id)
                .map_err(|e| ReceiverError::Internal(anyhow::anyhow!("tx_id parse: {e}")))?;
            (StatusCode::OK, id, "replay")
        }
        OneofResp::ProviderReport(Some(provider_report_response::Outcome::Error(e))) => {
            return Err(map_ledger_error(e.code, &e.message))
        }
        OneofResp::ProviderReport(None) => {
            return Err(ReceiverError::Internal(anyhow::anyhow!(
                "ledger response missing outcome"
            )))
        }
        OneofResp::InvoiceReconcile(Some(invoice_reconcile_response::Outcome::Success(s))) => {
            let id = Uuid::parse_str(&s.ledger_transaction_id)
                .map_err(|e| ReceiverError::Internal(anyhow::anyhow!("tx_id parse: {e}")))?;
            (StatusCode::OK, id, "success")
        }
        OneofResp::InvoiceReconcile(Some(invoice_reconcile_response::Outcome::Replay(r))) => {
            let id = Uuid::parse_str(&r.ledger_transaction_id)
                .map_err(|e| ReceiverError::Internal(anyhow::anyhow!("tx_id parse: {e}")))?;
            (StatusCode::OK, id, "replay")
        }
        OneofResp::InvoiceReconcile(Some(invoice_reconcile_response::Outcome::Error(e))) => {
            return Err(map_ledger_error(e.code, &e.message))
        }
        OneofResp::InvoiceReconcile(None) => {
            return Err(ReceiverError::Internal(anyhow::anyhow!(
                "ledger response missing outcome"
            )))
        }
        OneofResp::GrpcError(grpc_status) => {
            // Codex r3 V3.1: detect IdempotencyConflict via substring in either
            // the gRPC status message or details.
            let msg = grpc_status.message();
            if msg.contains("idempotency_key reused") {
                // Race-window: dedupe row not yet inserted by winner.
                // Re-look up existing dedupe row OR ledger_transactions.
                let dr = dedupe::lookup(
                    &state.pg,
                    provider,
                    event_kind,
                    &parsed.provider_account,
                    &parsed.provider_event_id,
                )
                .await?;
                if let Some(row) = dr {
                    if row.canonical_hash == canonical {
                        return Ok((
                            StatusCode::OK,
                            Json(WebhookResponse {
                                ledger_transaction_id: row.ledger_transaction_id.to_string(),
                                outcome: "replay",
                            }),
                        ));
                    } else {
                        return Err(ReceiverError::Conflict);
                    }
                }
                let tx = dedupe::lookup_ledger_tx(
                    &state.pg,
                    tenant_uuid,
                    op_kind,
                    &provenance,
                )
                .await?;
                if let Some((tx_id, hash)) = tx {
                    if hash == canonical.to_vec() {
                        // Insert dedupe row defensively (POC self-heal).
                        let _ = dedupe::insert(
                            &state.pg,
                            provider,
                            event_kind,
                            &parsed.provider_account,
                            &parsed.provider_event_id,
                            &canonical,
                            tx_id,
                        )
                        .await;
                        return Ok((
                            StatusCode::OK,
                            Json(WebhookResponse {
                                ledger_transaction_id: tx_id.to_string(),
                                outcome: "replay",
                            }),
                        ));
                    }
                    return Err(ReceiverError::Conflict);
                }
                return Err(ReceiverError::LedgerTransient(format!(
                    "idempotency conflict but no replay row found: {}",
                    msg
                )));
            }
            return Err(ReceiverError::LedgerTransient(format!(
                "ledger gRPC failed: {}",
                msg
            )));
        }
    };

    // 9. Insert dedupe (best-effort)
    let inserted = dedupe::insert(
        &state.pg,
        provider,
        event_kind,
        &parsed.provider_account,
        &parsed.provider_event_id,
        &canonical,
        ledger_tx_id,
    )
    .await?;
    if !inserted {
        debug!("webhook_dedupe row already present (concurrent winner)");
    }

    info!(
        provider = %provider,
        event_kind = %event_kind,
        tx_id = %ledger_tx_id,
        outcome = %outcome,
        "webhook routed"
    );

    Ok((
        status,
        Json(WebhookResponse {
            ledger_transaction_id: ledger_tx_id.to_string(),
            outcome,
        }),
    ))
}

// ---- helpers ----------------------------------------------------------------

#[allow(clippy::large_enum_variant)]
enum RequestOuter {
    ProviderReport(ProviderReportRequest),
    InvoiceReconcile(InvoiceReconcileRequest),
}

#[allow(clippy::large_enum_variant)]
enum OneofResp {
    ProviderReport(Option<provider_report_response::Outcome>),
    InvoiceReconcile(Option<invoice_reconcile_response::Outcome>),
    GrpcError(tonic::Status),
}

fn map_ledger_error(code: i32, message: &str) -> ReceiverError {
    // Codex challenge P2.1: carry the EXACT StatusCode v3 prescribed.
    // The earlier coarse Business/Transient/Unspecified buckets lost
    // precision for 500 (AUDIT_INVARIANT_VIOLATED), 501 (deferred),
    // 401 (TENANT_DISABLED / SIGNATURE_INVALID).
    let status = ledger_code_to_http(code, message);
    if status == StatusCode::CONFLICT {
        ReceiverError::Conflict
    } else {
        ReceiverError::LedgerExact {
            status,
            message: format!("code={} {}", code, message),
        }
    }
}

fn secret_for_provider(state: &AppState, provider: &str) -> Result<String, ReceiverError> {
    match provider {
        "mock-llm" => Ok(state.config.mock_llm_secret.clone()),
        _ => Err(ReceiverError::UnknownProvider),
    }
}

/// sha256(text)[0..16] formatted as v4 UUID (matches demo simulator
/// derivation in run_demo.py:325-339).
fn derive_decision_id_v4(text: &str) -> Uuid {
    let mut h = Sha256::new();
    h.update(text.as_bytes());
    let bytes: [u8; 32] = h.finalize().into();
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&bytes[..16]);
    // Set v4 + RFC 4122 variant bits.
    buf[6] = (buf[6] & 0x0F) | 0x40;
    buf[8] = (buf[8] & 0x3F) | 0x80;
    Uuid::from_bytes(buf)
}

#[derive(Copy, Clone)]
enum CloudEventKind {
    Decision,
    Outcome,
}

fn build_cloudevent(
    state: &AppState,
    kind: CloudEventKind,
    provider: &str,
    parsed: &WebhookBody,
    decision_id: &Uuid,
    seq: u64,
    amount: Option<&BigInt>,
) -> Result<CloudEvent, ReceiverError> {
    let now = chrono::Utc::now();
    let ts = Timestamp {
        seconds: now.timestamp(),
        nanos: now.timestamp_subsec_nanos() as i32,
    };
    let event_id = Uuid::now_v7().to_string();
    let (type_str, kind_label) = match kind {
        CloudEventKind::Decision => ("spendguard.audit.decision", "provider_report"),
        CloudEventKind::Outcome => ("spendguard.audit.outcome", "invoice_reconcile"),
    };
    let amount_str = amount.map(|a| a.to_string()).unwrap_or_default();
    let data_obj = json!({
        "kind": kind_label,
        "provider": provider,
        "provider_account": parsed.provider_account,
        "provider_event_id": parsed.provider_event_id,
        "amount_atomic": amount_str,
    });
    let data_bytes = serde_json::to_vec(&data_obj)
        .map_err(|e| ReceiverError::Internal(anyhow::anyhow!("ce data serialize: {e}")))?;

    Ok(CloudEvent {
        specversion: "1.0".into(),
        r#type: type_str.into(),
        source: format!("webhook-receiver://{}/{}", provider, parsed.provider_account),
        id: event_id,
        time: Some(ts),
        datacontenttype: "application/json".into(),
        data: data_bytes.into(),
        tenant_id: parsed.tenant_id.clone(),
        run_id: String::new(),
        decision_id: decision_id.to_string(),
        schema_bundle_id: String::new(),
        producer_id: format!("webhook-receiver:{}", state.config.workload_instance_id),
        producer_sequence: seq,
        signing_key_id: "webhook-receiver:demo:v1".into(),
        producer_signature: Vec::new().into(),
    })
}

// Marker — keeps base64 import alive if future encoding needed.
#[allow(dead_code)]
fn _unused_base64_marker() {
    let _ = base64::engine::general_purpose::STANDARD.encode([]);
}

// Marker — explicit reference to SequenceAllocator so unused-import lints don't fire.
#[allow(dead_code)]
fn _seq_marker(s: &SequenceAllocator) -> u64 {
    s.next_one()
}
