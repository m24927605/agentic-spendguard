//! Phase 5 GA hardening S19 followup #10: retention sweeper.
//!
//! Background worker that polls `audit_outbox` + `provider_usage_records`
//! and redacts rows past their tenant's retention window. Mirrors the
//! `ttl_sweeper` shape (lease + poll loop + round-9 `is_leader_now()`
//! gating). NEVER deletes — migration 0028's BEFORE DELETE triggers
//! reject deletes regardless. Redaction is in-place UPDATE to a
//! marker JSONB; the canonical SHA-256 hash of the original bytes is
//! preserved in a sibling field so the audit chain stays valid.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use thiserror::Error;
use tracing::{info, warn};

#[derive(Debug, Error)]
pub enum SweeperError {
    #[error("db: {0}")]
    Db(#[from] sqlx::Error),
    #[error("config: {0}")]
    Config(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SweepKind {
    PromptRedaction,
    ProviderRawRedaction,
    TombstoneCheck,
}

impl SweepKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PromptRedaction => "prompt_redaction",
            Self::ProviderRawRedaction => "provider_raw_redaction",
            Self::TombstoneCheck => "tombstone_check",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SweepOutcome {
    pub sweep_kind: SweepKind,
    pub rows_examined: i64,
    pub rows_redacted: i64,
    pub rows_failed: i64,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
}

/// Marker JSONB written in place of the original `data` field. Per
/// data-classification.md operator playbook.
fn redacted_marker(now: DateTime<Utc>) -> serde_json::Value {
    serde_json::json!({
        "_redacted":   true,
        "redacted_at": now.to_rfc3339(),
    })
}

/// Sweep audit_outbox: redact `cloudevent_payload->'data'` rows whose
/// tenant's `prompt_retention_days` window has elapsed. Preserves the
/// SHA-256 hash of the original bytes in `_data_sha256_hex` so audit
/// chain hash continuity holds for future verifiers.
pub async fn sweep_audit_outbox_prompts(
    pool: &PgPool,
    batch_size: i64,
) -> Result<SweepOutcome, SweeperError> {
    let started_at = Utc::now();
    // Find candidate rows: each row's tenant has prompt_retention_days
    // configured and the row is older than that window AND the data
    // field hasn't already been redacted.
    let candidates: Vec<(uuid::Uuid, serde_json::Value)> = sqlx::query_as(
        r#"
        SELECT a.audit_outbox_id, a.cloudevent_payload->'data' AS data
          FROM audit_outbox a
          JOIN tenant_data_policy p ON p.tenant_id = a.tenant_id
         WHERE p.prompt_retention_days > 0
           AND a.recorded_at < clock_timestamp()
                             - (p.prompt_retention_days * INTERVAL '1 day')
           AND COALESCE((a.cloudevent_payload->'data'->>'_redacted')::BOOLEAN, FALSE) = FALSE
           AND a.cloudevent_payload->'data' IS NOT NULL
         ORDER BY a.recorded_at ASC
         LIMIT $1
        "#,
    )
    .bind(batch_size)
    .fetch_all(pool)
    .await?;

    let rows_examined = candidates.len() as i64;
    let mut rows_redacted = 0i64;
    let mut rows_failed = 0i64;

    for (audit_id, data) in candidates {
        let data_bytes = data.to_string().into_bytes();
        let mut h = Sha256::new();
        h.update(&data_bytes);
        let digest_hex = hex::encode(h.finalize());
        let now = Utc::now();
        let marker = redacted_marker(now);

        let result = sqlx::query(
            r#"
            UPDATE audit_outbox
               SET cloudevent_payload =
                       jsonb_set(
                           jsonb_set(cloudevent_payload, '{data}', $2::JSONB, true),
                           '{_data_sha256_hex}', to_jsonb($3::TEXT), true)
             WHERE audit_outbox_id = $1
            "#,
        )
        .bind(audit_id)
        .bind(&marker)
        .bind(&digest_hex)
        .execute(pool)
        .await;

        match result {
            Ok(_) => rows_redacted += 1,
            Err(e) => {
                warn!(audit_id = %audit_id, err = %e, "audit_outbox redaction failed");
                rows_failed += 1;
            }
        }
    }

    Ok(SweepOutcome {
        sweep_kind: SweepKind::PromptRedaction,
        rows_examined,
        rows_redacted,
        rows_failed,
        started_at,
        finished_at: Utc::now(),
    })
}

/// Sweep provider_usage_records: redact `raw_payload` rows whose
/// tenant's `provider_raw_retention_days` window has elapsed.
pub async fn sweep_provider_usage_raw(
    pool: &PgPool,
    batch_size: i64,
) -> Result<SweepOutcome, SweeperError> {
    let started_at = Utc::now();
    let candidates: Vec<(uuid::Uuid,)> = sqlx::query_as(
        r#"
        SELECT r.record_id
          FROM provider_usage_records r
          JOIN tenant_data_policy p ON p.tenant_id = r.tenant_id
         WHERE p.provider_raw_retention_days > 0
           AND r.received_at < clock_timestamp()
                             - (p.provider_raw_retention_days * INTERVAL '1 day')
           AND COALESCE((r.raw_payload->>'_redacted')::BOOLEAN, FALSE) = FALSE
           AND r.raw_payload IS NOT NULL
         ORDER BY r.received_at ASC
         LIMIT $1
        "#,
    )
    .bind(batch_size)
    .fetch_all(pool)
    .await?;

    let rows_examined = candidates.len() as i64;
    let mut rows_redacted = 0i64;
    let mut rows_failed = 0i64;

    for (record_id,) in candidates {
        let now = Utc::now();
        let marker = redacted_marker(now);
        let result = sqlx::query(
            r#"
            UPDATE provider_usage_records
               SET raw_payload = $2::JSONB
             WHERE record_id = $1
            "#,
        )
        .bind(record_id)
        .bind(&marker)
        .execute(pool)
        .await;
        match result {
            Ok(_) => rows_redacted += 1,
            Err(e) => {
                warn!(record_id = %record_id, err = %e, "provider_usage_records redaction failed");
                rows_failed += 1;
            }
        }
    }

    Ok(SweepOutcome {
        sweep_kind: SweepKind::ProviderRawRedaction,
        rows_examined,
        rows_redacted,
        rows_failed,
        started_at,
        finished_at: Utc::now(),
    })
}

/// Insert one row into retention_sweeper_log with the outcome of a
/// sweep pass. Matches migration 0028's CHECK constraints.
pub async fn log_sweep(
    pool: &PgPool,
    outcome: &SweepOutcome,
) -> Result<(), SweeperError> {
    let outcome_text = if outcome.rows_failed == 0 {
        "success"
    } else if outcome.rows_redacted > 0 {
        "partial_failure"
    } else {
        "permanent_failure"
    };
    sqlx::query(
        r#"
        INSERT INTO retention_sweeper_log
            (started_at, finished_at, outcome, sweep_kind,
             rows_examined, rows_redacted, rows_failed)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
    )
    .bind(outcome.started_at)
    .bind(outcome.finished_at)
    .bind(outcome_text)
    .bind(outcome.sweep_kind.as_str())
    .bind(outcome.rows_examined)
    .bind(outcome.rows_redacted)
    .bind(outcome.rows_failed)
    .execute(pool)
    .await?;
    info!(
        sweep_kind = outcome.sweep_kind.as_str(),
        rows_examined = outcome.rows_examined,
        rows_redacted = outcome.rows_redacted,
        rows_failed = outcome.rows_failed,
        outcome = outcome_text,
        "retention sweep logged"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_kind_as_str_matches_check_constraint() {
        // Migration 0028 has CHECK (sweep_kind IN ('prompt_redaction',
        // 'provider_raw_redaction', 'tombstone_check')). If we change
        // the enum variants here we must also update the CHECK.
        assert_eq!(SweepKind::PromptRedaction.as_str(), "prompt_redaction");
        assert_eq!(SweepKind::ProviderRawRedaction.as_str(), "provider_raw_redaction");
        assert_eq!(SweepKind::TombstoneCheck.as_str(), "tombstone_check");
    }

    #[test]
    fn redacted_marker_is_well_formed() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-05-11T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let m = redacted_marker(now);
        assert_eq!(m.get("_redacted").and_then(|v| v.as_bool()), Some(true));
        assert!(m.get("redacted_at").is_some());
    }
}
