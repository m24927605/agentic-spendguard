//! QueryAuditChain implementation.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::error::{map_pg_error, DomainError};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ChainRow {
    pub event_id: Uuid,
    pub tenant_id: Uuid,
    pub decision_id: Option<Uuid>,
    pub run_id: Option<Uuid>,
    pub event_type: String,
    pub storage_class: String,
    pub producer_id: String,
    pub producer_sequence: i64,
    pub producer_signature: Vec<u8>,
    pub signing_key_id: String,
    pub schema_bundle_id: Uuid,
    pub schema_bundle_hash: Vec<u8>,
    pub specversion: String,
    pub source: String,
    pub event_time: DateTime<Utc>,
    pub datacontenttype: String,
    pub payload_json: Option<serde_json::Value>,
    pub payload_blob_ref: Option<String>,
    pub region_id: String,
    pub ingest_shard_id: String,
    pub ingest_log_offset: i64,
    pub ingest_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy)]
pub enum Anchor {
    DecisionId,
    RunId,
    Reservation, // will require a future denorm column; not in POC
}

/// Stream all audit-chain events for a given anchor.
///
/// POC: filters by exactly one anchor field. Production should accept
/// from/to time bounds + storage_class filters.
pub async fn query_chain_by_decision(
    pool: &PgPool,
    tenant_id: Uuid,
    decision_id: Uuid,
    storage_class_filter: Option<Vec<&str>>,
) -> Result<Vec<ChainRow>, DomainError> {
    let classes: Vec<String> = storage_class_filter
        .unwrap_or_else(|| {
            vec!["immutable_audit_log", "canonical_raw_log"]
        })
        .into_iter()
        .map(|s| s.to_string())
        .collect();

    let rows = sqlx::query_as::<_, ChainRow>(
        "SELECT event_id, tenant_id, decision_id, run_id, event_type,
                storage_class, producer_id, producer_sequence,
                producer_signature, signing_key_id, schema_bundle_id,
                schema_bundle_hash, specversion, source, event_time,
                datacontenttype, payload_json, payload_blob_ref,
                region_id, ingest_shard_id, ingest_log_offset, ingest_at
           FROM canonical_events
          WHERE tenant_id = $1
            AND decision_id = $2
            AND storage_class = ANY($3)
          ORDER BY event_time ASC, ingest_log_offset ASC",
    )
    .bind(tenant_id)
    .bind(decision_id)
    .bind(&classes)
    .fetch_all(pool)
    .await
    .map_err(map_pg_error)?;

    Ok(rows)
}

pub async fn query_chain_by_run(
    pool: &PgPool,
    tenant_id: Uuid,
    run_id: Uuid,
    storage_class_filter: Option<Vec<&str>>,
) -> Result<Vec<ChainRow>, DomainError> {
    let classes: Vec<String> = storage_class_filter
        .unwrap_or_else(|| vec!["immutable_audit_log", "canonical_raw_log"])
        .into_iter()
        .map(|s| s.to_string())
        .collect();

    let rows = sqlx::query_as::<_, ChainRow>(
        "SELECT event_id, tenant_id, decision_id, run_id, event_type,
                storage_class, producer_id, producer_sequence,
                producer_signature, signing_key_id, schema_bundle_id,
                schema_bundle_hash, specversion, source, event_time,
                datacontenttype, payload_json, payload_blob_ref,
                region_id, ingest_shard_id, ingest_log_offset, ingest_at
           FROM canonical_events
          WHERE tenant_id = $1
            AND run_id = $2
            AND storage_class = ANY($3)
          ORDER BY event_time ASC, ingest_log_offset ASC",
    )
    .bind(tenant_id)
    .bind(run_id)
    .bind(&classes)
    .fetch_all(pool)
    .await
    .map_err(map_pg_error)?;

    Ok(rows)
}

pub async fn approximate_backpressure_depth(pool: &PgPool) -> Result<i64, DomainError> {
    let depth: Option<i64> = sqlx::query_scalar(
        "SELECT COUNT(*)::BIGINT FROM audit_outcome_quarantine
          WHERE state = 'awaiting_decision'",
    )
    .fetch_one(pool)
    .await
    .map_err(map_pg_error)?;
    Ok(depth.unwrap_or(0))
}
