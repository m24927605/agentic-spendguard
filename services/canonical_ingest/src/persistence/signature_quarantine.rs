//! Phase 5 GA hardening S8: insert into `audit_signature_quarantine`.
//!
//! Append-only. Schema in `services/canonical_ingest/migrations/0007`.

use serde_json::json;
use sqlx::PgPool;

use crate::{domain::error::DomainError, proto::common::v1::CloudEvent};

/// Metadata recorded when the stored `claimed_canonical_bytes` are a
/// truncated copy of an oversized canonical form. The full SHA-256 of
/// the ORIGINAL (untruncated) bytes is preserved so the forensic record
/// is verifiable even though the stored payload is clipped — the event
/// is durably recorded instead of being silently dropped.
#[derive(Debug, Clone)]
pub struct TruncationNote {
    /// Length of the original (pre-truncation) canonical bytes.
    pub original_len: usize,
    /// Lowercase hex SHA-256 of the original (pre-truncation) bytes.
    pub original_sha256_hex: String,
}

/// Insert one quarantine row. `reason` MUST match the CHECK constraint
/// in migration 0007.
pub async fn insert(
    pool: &PgPool,
    evt: &CloudEvent,
    canonical_bytes: &[u8],
    reason: &str,
) -> Result<(), DomainError> {
    insert_with_truncation(pool, evt, canonical_bytes, reason, None).await
}

/// Insert one quarantine row, optionally recording that
/// `canonical_bytes` is a truncated copy of an oversized canonical form
/// (the full SHA-256 + original length are stored in `debug_info`).
pub async fn insert_with_truncation(
    pool: &PgPool,
    evt: &CloudEvent,
    canonical_bytes: &[u8],
    reason: &str,
    truncation: Option<TruncationNote>,
) -> Result<(), DomainError> {
    // Derive signing_algorithm from key_id prefix (mirror of migration
    // 0024's CASE expression). Stored alongside so quarantine triage
    // queries don't have to re-parse.
    let algorithm = if evt.signing_key_id.starts_with("ed25519:") {
        "ed25519"
    } else if evt.signing_key_id.starts_with("arn:aws:kms:")
        || evt.signing_key_id.starts_with("kms-")
    {
        "kms-ed25519"
    } else if evt.signing_key_id.starts_with("disabled:") {
        "disabled"
    } else {
        "pre-S6"
    };

    let debug_info = json!({
        "claimed_signature_len": evt.producer_signature.len(),
        "canonical_form": if evt.producer_id.starts_with("ledger:") { "json" } else { "proto" },
        "canonical_truncated": truncation.is_some(),
        "canonical_original_len": truncation.as_ref().map(|t| t.original_len),
        "canonical_original_sha256": truncation.as_ref().map(|t| t.original_sha256_hex.clone()),
    });

    sqlx::query(
        r#"
        INSERT INTO audit_signature_quarantine (
            claimed_event_id, claimed_tenant_id, claimed_event_type,
            claimed_decision_id, claimed_run_id, claimed_producer_id,
            claimed_producer_sequence,
            claimed_canonical_bytes, claimed_signature,
            claimed_signing_key_id, claimed_signing_algorithm,
            reason, debug_info
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
        "#,
    )
    .bind(&evt.id)
    .bind(&evt.tenant_id)
    .bind(&evt.r#type)
    .bind(if evt.decision_id.is_empty() {
        None
    } else {
        Some(&evt.decision_id)
    })
    .bind(if evt.run_id.is_empty() {
        None
    } else {
        Some(&evt.run_id)
    })
    .bind(&evt.producer_id)
    .bind(evt.producer_sequence as i64)
    .bind(canonical_bytes)
    .bind(evt.producer_signature.as_ref())
    .bind(&evt.signing_key_id)
    .bind(algorithm)
    .bind(reason)
    .bind(debug_info)
    .execute(pool)
    .await
    .map_err(|e| {
        DomainError::Internal(anyhow::anyhow!("audit_signature_quarantine insert: {e}"))
    })?;

    Ok(())
}
