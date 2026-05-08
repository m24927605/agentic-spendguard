//! Atomic append of a single canonical event.
//!
//! Two transactional shapes:
//!   1. `append_event(...)`: inserts into canonical_events + global_keys.
//!      Used for non-audit events and for audit.decision events.
//!      The `assert_audit_outcome_has_preceding_decision` trigger on
//!      global_keys vetoes audit.outcome inserts whose decision is missing.
//!   2. `quarantine_audit_outcome(...)`: inserts into audit_outcome_quarantine
//!      when the handler has already determined that no preceding
//!      audit.decision exists. Reaper releases later.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::{
    error::{map_pg_error, DomainError},
    event_routing::StorageClass,
};

#[derive(Debug, Clone)]
pub struct AppendInput<'a> {
    pub event_id: Uuid,
    pub tenant_id: Uuid,
    pub decision_id: Option<Uuid>,
    pub run_id: Option<Uuid>,
    pub event_type: &'a str,

    pub storage_class: StorageClass,

    pub producer_id: &'a str,
    pub producer_sequence: i64,
    pub producer_signature: &'a [u8],
    pub signing_key_id: &'a str,

    pub schema_bundle_id: Uuid,
    pub schema_bundle_hash: &'a [u8],

    pub specversion: &'a str,
    pub source: &'a str,
    pub event_time: DateTime<Utc>,
    pub datacontenttype: &'a str,
    pub payload_json: serde_json::Value,
    pub payload_blob_ref: Option<&'a str>,

    pub region_id: &'a str,
    pub ingest_shard_id: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendOutcome {
    Appended { ingest_log_offset: i64 },
    Deduped,
}

/// Insert into canonical_events + canonical_events_global_keys +
/// canonical_ingest_positions atomically.
///
/// All offset allocation, dedupe checks, partitioned insert, position
/// uniqueness, and global_keys uniqueness are inside a single Postgres
/// transaction. Aborted/deduped retries do NOT consume offsets.
///
/// Returns Deduped when event_id already exists. The
/// `assert_audit_outcome_has_preceding_decision` trigger (defense-in-depth)
/// raises P0002 when an audit.outcome reaches us without its decision —
/// the caller in handlers/append_events.rs catches AwaitingPrecedingDecision
/// and redirects to quarantine.
pub async fn append_event(
    pool: &PgPool,
    input: AppendInput<'_>,
) -> Result<AppendOutcome, DomainError> {
    let mut tx = pool.begin().await.map_err(map_pg_error)?;

    // 1) Insert into global_keys first (fires trigger). Dedupe via ON CONFLICT.
    let inserted_keys = sqlx::query(
        "INSERT INTO canonical_events_global_keys
            (event_id, tenant_id, decision_id, event_type, recorded_month)
         VALUES ($1, $2, $3, $4, date_trunc('month', $5)::DATE)
         ON CONFLICT (event_id) DO NOTHING",
    )
    .bind(input.event_id)
    .bind(input.tenant_id)
    .bind(input.decision_id)
    .bind(input.event_type)
    .bind(input.event_time)
    .execute(&mut *tx)
    .await
    .map_err(map_pg_error)?;

    if inserted_keys.rows_affected() == 0 {
        // Already in the global mirror — duplicate.
        tx.rollback().await.map_err(map_pg_error)?;
        return Ok(AppendOutcome::Deduped);
    }

    // 2) Allocate ingest offset INSIDE the tx so rollback restores it.
    let offset: i64 = sqlx::query_scalar(
        "SELECT next_ingest_offset($1, $2)",
    )
    .bind(input.region_id)
    .bind(input.ingest_shard_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_pg_error)?;

    // 3) Insert into the partitioned canonical_events table.
    sqlx::query(
        "INSERT INTO canonical_events (
            event_id, tenant_id, decision_id, run_id, event_type,
            storage_class,
            producer_id, producer_sequence, producer_signature, signing_key_id,
            schema_bundle_id, schema_bundle_hash,
            specversion, source, event_time, datacontenttype,
            payload_json, payload_blob_ref,
            region_id, ingest_shard_id, ingest_log_offset, ingest_at,
            recorded_month
         ) VALUES (
            $1, $2, $3, $4, $5,
            $6,
            $7, $8, $9, $10,
            $11, $12,
            $13, $14, $15, $16,
            $17, $18,
            $19, $20, $21, clock_timestamp(),
            date_trunc('month', $15)::DATE
         )",
    )
    .bind(input.event_id)
    .bind(input.tenant_id)
    .bind(input.decision_id)
    .bind(input.run_id)
    .bind(input.event_type)
    .bind(input.storage_class.as_db_str())
    .bind(input.producer_id)
    .bind(input.producer_sequence)
    .bind(input.producer_signature)
    .bind(input.signing_key_id)
    .bind(input.schema_bundle_id)
    .bind(input.schema_bundle_hash)
    .bind(input.specversion)
    .bind(input.source)
    .bind(input.event_time)
    .bind(input.datacontenttype)
    .bind(input.payload_json)
    .bind(input.payload_blob_ref)
    .bind(input.region_id)
    .bind(input.ingest_shard_id)
    .bind(offset)
    .execute(&mut *tx)
    .await
    .map_err(map_pg_error)?;

    // 4) Mirror position into non-partitioned table for global UNIQUE.
    sqlx::query(
        "INSERT INTO canonical_ingest_positions
            (region_id, ingest_shard_id, ingest_log_offset,
             event_id, recorded_month)
         VALUES ($1, $2, $3, $4, date_trunc('month', $5)::DATE)",
    )
    .bind(input.region_id)
    .bind(input.ingest_shard_id)
    .bind(offset)
    .bind(input.event_id)
    .bind(input.event_time)
    .execute(&mut *tx)
    .await
    .map_err(map_pg_error)?;

    tx.commit().await.map_err(map_pg_error)?;
    Ok(AppendOutcome::Appended {
        ingest_log_offset: offset,
    })
}

/// Release quarantined audit.outcome events after the matching audit.decision
/// has been appended.
///
/// All work runs in a single Postgres tx so the FOR UPDATE SKIP LOCKED
/// rows stay locked for the duration of the append + state transition.
/// Concurrent releasers/reapers running this fn for the same decision skip
/// already-locked rows.
///
/// IMPORTANT: original metadata (storage_class, schema_bundle_id +
/// schema_bundle_hash, datacontenttype, etc.) comes from the quarantined
/// row, NOT from the audit.decision caller's batch. The release MUST
/// preserve the canonical event's original envelope verbatim.
///
/// Returns the count of outcomes released.
pub async fn release_quarantined_outcomes(
    pool: &PgPool,
    tenant_id: uuid::Uuid,
    decision_id: uuid::Uuid,
    region_id: &str,
    ingest_shard_id: &str,
) -> Result<usize, DomainError> {
    let mut tx = pool.begin().await.map_err(map_pg_error)?;

    // Hold per-row FOR UPDATE locks for the duration of the tx; concurrent
    // releasers SKIP LOCKED rows we hold.
    let rows = sqlx::query_as::<
        _,
        (
            uuid::Uuid,            // event_id
            String,                // storage_class
            i64,                   // producer_sequence
            Vec<u8>,               // producer_signature
            String,                // signing_key_id
            String,                // producer_id
            String,                // event_type
            DateTime<Utc>,         // event_time
            String,                // source
            Option<serde_json::Value>, // payload_json
            Option<String>,        // payload_blob_ref
            Option<uuid::Uuid>,    // run_id
            String,                // specversion
            uuid::Uuid,            // schema_bundle_id (original)
            Vec<u8>,               // schema_bundle_hash (original)
            String,                // datacontenttype (original)
        ),
    >(
        "SELECT event_id, storage_class,
                producer_sequence, producer_signature,
                signing_key_id, producer_id,
                event_type, event_time, source,
                payload_json, payload_blob_ref, run_id,
                specversion,
                schema_bundle_id, schema_bundle_hash, datacontenttype
           FROM audit_outcome_quarantine
          WHERE tenant_id = $1
            AND decision_id = $2
            AND state = 'awaiting_decision'
          FOR UPDATE SKIP LOCKED",
    )
    .bind(tenant_id)
    .bind(decision_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(map_pg_error)?;

    let mut released = 0usize;
    for r in rows {
        let (
            event_id,
            storage_class_str,
            producer_sequence,
            producer_signature,
            signing_key_id,
            producer_id,
            event_type,
            event_time,
            source,
            payload_json_opt,
            payload_blob_ref_opt,
            run_id,
            specversion,
            orig_schema_bundle_id,
            orig_schema_bundle_hash,
            orig_datacontenttype,
        ) = r;

        let storage_class = match storage_class_str.as_str() {
            "immutable_audit_log" => StorageClass::ImmutableAuditLog,
            "canonical_raw_log" => StorageClass::CanonicalRawLog,
            "profile_payload_blob" => StorageClass::ProfilePayloadBlob,
            other => {
                return Err(DomainError::Internal(anyhow::anyhow!(
                    "unknown storage_class in quarantine row: {}",
                    other
                )))
            }
        };
        let payload_json = payload_json_opt.unwrap_or(serde_json::Value::Null);

        // Append into canonical_events + global_keys + ingest_positions in
        // the SAME tx as the quarantine SELECT/UPDATE. We inline the insert
        // SQL (rather than calling append_event which begins its own tx)
        // because we already hold a tx via `tx`.
        let inserted_keys = sqlx::query(
            "INSERT INTO canonical_events_global_keys
                (event_id, tenant_id, decision_id, event_type, recorded_month)
             VALUES ($1, $2, $3, $4, date_trunc('month', $5)::DATE)
             ON CONFLICT (event_id) DO NOTHING",
        )
        .bind(event_id)
        .bind(tenant_id)
        .bind(decision_id)
        .bind(&event_type)
        .bind(event_time)
        .execute(&mut *tx)
        .await
        .map_err(map_pg_error)?;

        if inserted_keys.rows_affected() > 0 {
            let offset: i64 = sqlx::query_scalar("SELECT next_ingest_offset($1, $2)")
                .bind(region_id)
                .bind(ingest_shard_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(map_pg_error)?;

            sqlx::query(
                "INSERT INTO canonical_events (
                    event_id, tenant_id, decision_id, run_id, event_type,
                    storage_class,
                    producer_id, producer_sequence, producer_signature, signing_key_id,
                    schema_bundle_id, schema_bundle_hash,
                    specversion, source, event_time, datacontenttype,
                    payload_json, payload_blob_ref,
                    region_id, ingest_shard_id, ingest_log_offset, ingest_at,
                    recorded_month
                 ) VALUES (
                    $1, $2, $3, $4, $5,
                    $6,
                    $7, $8, $9, $10,
                    $11, $12,
                    $13, $14, $15, $16,
                    $17, $18,
                    $19, $20, $21, clock_timestamp(),
                    date_trunc('month', $15)::DATE
                 )",
            )
            .bind(event_id)
            .bind(tenant_id)
            .bind(decision_id)
            .bind(run_id)
            .bind(&event_type)
            .bind(storage_class.as_db_str())
            .bind(&producer_id)
            .bind(producer_sequence)
            .bind(&producer_signature)
            .bind(&signing_key_id)
            .bind(orig_schema_bundle_id)
            .bind(&orig_schema_bundle_hash)
            .bind(&specversion)
            .bind(&source)
            .bind(event_time)
            .bind(&orig_datacontenttype)
            .bind(&payload_json)
            .bind(payload_blob_ref_opt.as_deref())
            .bind(region_id)
            .bind(ingest_shard_id)
            .bind(offset)
            .execute(&mut *tx)
            .await
            .map_err(map_pg_error)?;

            sqlx::query(
                "INSERT INTO canonical_ingest_positions
                    (region_id, ingest_shard_id, ingest_log_offset,
                     event_id, recorded_month)
                 VALUES ($1, $2, $3, $4, date_trunc('month', $5)::DATE)",
            )
            .bind(region_id)
            .bind(ingest_shard_id)
            .bind(offset)
            .bind(event_id)
            .bind(event_time)
            .execute(&mut *tx)
            .await
            .map_err(map_pg_error)?;
        }

        // Transition state regardless (released even if globally deduped).
        sqlx::query(
            "UPDATE audit_outcome_quarantine
                SET state = 'released',
                    state_changed_at = clock_timestamp(),
                    released_to_event_id = $2
              WHERE event_id = $1",
        )
        .bind(event_id)
        .bind(event_id)
        .execute(&mut *tx)
        .await
        .map_err(map_pg_error)?;
        released += 1;
    }

    tx.commit().await.map_err(map_pg_error)?;
    Ok(released)
}

/// Insert audit.outcome event into quarantine.
pub async fn quarantine_audit_outcome(
    pool: &PgPool,
    input: AppendInput<'_>,
    orphan_after: DateTime<Utc>,
) -> Result<(), DomainError> {
    let decision_id = input
        .decision_id
        .ok_or_else(|| DomainError::InvalidRequest("audit.outcome missing decision_id".into()))?;

    sqlx::query(
        "INSERT INTO audit_outcome_quarantine (
            quarantine_id, event_id, tenant_id, decision_id,
            storage_class, producer_id, producer_sequence,
            producer_signature, signing_key_id,
            schema_bundle_id, schema_bundle_hash,
            event_type, specversion, source, event_time, datacontenttype,
            payload_json, payload_blob_ref,
            region_id, ingest_shard_id, ingest_log_offset, run_id,
            orphan_after
         ) VALUES (
            $1, $2, $3, $4,
            $5, $6, $7,
            $8, $9,
            $10, $11,
            $12, $13, $14, $15, $16,
            $17, $18,
            $19, $20, $21, $22,
            $23
         )
         ON CONFLICT (event_id) DO NOTHING",
    )
    .bind(Uuid::now_v7()) // quarantine_id
    .bind(input.event_id)
    .bind(input.tenant_id)
    .bind(decision_id)
    .bind(input.storage_class.as_db_str())
    .bind(input.producer_id)
    .bind(input.producer_sequence)
    .bind(input.producer_signature)
    .bind(input.signing_key_id)
    .bind(input.schema_bundle_id)
    .bind(input.schema_bundle_hash)
    .bind(input.event_type)
    .bind(input.specversion)
    .bind(input.source)
    .bind(input.event_time)
    .bind(input.datacontenttype)
    .bind(input.payload_json)
    .bind(input.payload_blob_ref)
    .bind(input.region_id)
    .bind(input.ingest_shard_id)
    .bind(0i64) // placeholder offset; assigned at release time
    .bind(input.run_id)
    .bind(orphan_after)
    .execute(pool)
    .await
    .map_err(map_pg_error)?;

    Ok(())
}

/// Lookup whether a matching audit.decision exists for (tenant, decision_id).
pub async fn has_preceding_decision(
    pool: &PgPool,
    tenant_id: Uuid,
    decision_id: Uuid,
) -> Result<bool, DomainError> {
    let exists: Option<bool> = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM canonical_events_global_keys
             WHERE tenant_id = $1
               AND decision_id = $2
               AND event_type = 'spendguard.audit.decision'
        )",
    )
    .bind(tenant_id)
    .bind(decision_id)
    .fetch_one(pool)
    .await
    .map_err(map_pg_error)?;
    Ok(exists.unwrap_or(false))
}
