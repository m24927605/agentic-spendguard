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

use bigdecimal::BigDecimal;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::{
    error::{map_pg_error, DomainError},
    event_routing::StorageClass,
};

/// Round-3 fix B2: prediction-extension column bundle.
///
/// Bundled as one struct so AppendInput stays readable and SLICE_06
/// producers can fill it from a single decode of the CloudEvent
/// tag 300-317 fields. Every field maps 1:1 onto an audit_outbox /
/// canonical_events / audit_outcome_quarantine column.
///
/// All fields are Option-typed because:
///   * Legacy (pre-SLICE_06) producers leave them None (SQL NULL).
///   * Strategy-conditional fields (predicted_b_tokens etc.) are None
///     when the strategy is unconfigured per spec §6.3 sentinel mapping.
///
/// Sentinel translation between proto wire (e.g., 0 / -1 / "") and
/// SQL NULL happens in `crates/spendguard-prediction-mirror`, NOT here.
/// This struct holds the post-translation SQL-side representation.
#[derive(Debug, Clone, Default)]
pub struct PredictionColumns<'a> {
    // Decision-side (11 per spec §2.1).
    pub predicted_a_tokens: Option<i64>,
    pub predicted_b_tokens: Option<i64>,
    pub predicted_c_tokens: Option<i64>,
    pub reserved_strategy: Option<&'a str>,
    pub prediction_strategy_used: Option<&'a str>,
    pub prediction_policy_used: Option<&'a str>,
    pub tokenizer_tier: Option<&'a str>,
    pub tokenizer_version_id: Option<Uuid>,
    pub prediction_confidence: Option<BigDecimal>,
    pub prediction_sample_size: Option<i64>,
    pub cold_start_layer_used: Option<&'a str>,
    // Run-level projection (3 per spec §2.2).
    pub run_projection_at_decision_atomic: Option<BigDecimal>,
    pub run_predicted_remaining_steps: Option<i32>,
    pub run_steps_completed_so_far: Option<i64>,
    // Commit-side actual (4 per spec §2.3).
    pub actual_input_tokens: Option<i64>,
    pub actual_output_tokens: Option<i64>,
    pub delta_b_ratio: Option<f32>,
    pub delta_c_ratio: Option<f32>,
}

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

    /// Cost Advisor P1.5 (issue #51, spec §5.1.2): failure
    /// classification for audit.outcome events. NULL when (a) the
    /// event is NOT an audit.outcome, (b) the payload couldn't be
    /// decoded, or (c) the classifier didn't match any rule (which
    /// is `Some("unknown")` actually, not None — see classify.rs).
    /// Caller computes via `crate::classify::classify_audit_outcome`.
    pub failure_class: Option<&'a str>,

    /// Round-3 fix B2: tag 300-317 prediction-extension column mirror.
    /// SLICE_01 callers pass Default::default() (all None → SQL NULL);
    /// SLICE_06 callers populate from the decoded CloudEvent.
    pub prediction: PredictionColumns<'a>,
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
    // failure_class column added in CA-P0 (migration 0011). Classifier
    // populates it for audit.outcome events; NULL for all others.
    //
    // Round-3 fix B2: bind the 18 tag 300-317 prediction columns from
    // input.prediction. SLICE_01 callers pass Default (all None →
    // SQL NULL); SLICE_06 callers populate from decoded CloudEvent.
    sqlx::query(
        "INSERT INTO canonical_events (
            event_id, tenant_id, decision_id, run_id, event_type,
            storage_class,
            producer_id, producer_sequence, producer_signature, signing_key_id,
            schema_bundle_id, schema_bundle_hash,
            specversion, source, event_time, datacontenttype,
            payload_json, payload_blob_ref,
            region_id, ingest_shard_id, ingest_log_offset, ingest_at,
            recorded_month, failure_class,
            -- 18 prediction columns (round-3 B2)
            predicted_a_tokens, predicted_b_tokens, predicted_c_tokens,
            reserved_strategy, prediction_strategy_used,
            prediction_policy_used, tokenizer_tier, tokenizer_version_id,
            prediction_confidence, prediction_sample_size,
            cold_start_layer_used,
            run_projection_at_decision_atomic,
            run_predicted_remaining_steps, run_steps_completed_so_far,
            actual_input_tokens, actual_output_tokens,
            delta_b_ratio, delta_c_ratio
         ) VALUES (
            $1, $2, $3, $4, $5,
            $6,
            $7, $8, $9, $10,
            $11, $12,
            $13, $14, $15, $16,
            $17, $18,
            $19, $20, $21, clock_timestamp(),
            date_trunc('month', $15)::DATE, $22,
            $23, $24, $25,
            $26, $27,
            $28, $29, $30,
            $31, $32,
            $33,
            $34,
            $35, $36,
            $37, $38,
            $39, $40
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
    .bind(input.failure_class)
    .bind(input.prediction.predicted_a_tokens)
    .bind(input.prediction.predicted_b_tokens)
    .bind(input.prediction.predicted_c_tokens)
    .bind(input.prediction.reserved_strategy)
    .bind(input.prediction.prediction_strategy_used)
    .bind(input.prediction.prediction_policy_used)
    .bind(input.prediction.tokenizer_tier)
    .bind(input.prediction.tokenizer_version_id)
    .bind(input.prediction.prediction_confidence.as_ref())
    .bind(input.prediction.prediction_sample_size)
    .bind(input.prediction.cold_start_layer_used)
    .bind(input.prediction.run_projection_at_decision_atomic.as_ref())
    .bind(input.prediction.run_predicted_remaining_steps)
    .bind(input.prediction.run_steps_completed_so_far)
    .bind(input.prediction.actual_input_tokens)
    .bind(input.prediction.actual_output_tokens)
    .bind(input.prediction.delta_b_ratio)
    .bind(input.prediction.delta_c_ratio)
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
    //
    // Round-3 fix B2: extend the SELECT tuple with the 18 prediction
    // columns so the release path can re-hydrate them into canonical_events.
    // Without these the canonical_events_outcome_required_cols_chk CHECK
    // would fail on outcomes past the 2027-01-01 cutoff. Type wrapper
    // QuarantinedRow used to keep the tuple readable (Rust caps generic
    // tuple impls at 16 elements; we need 34).
    let rows: Vec<QuarantinedRow> = sqlx::query_as::<_, QuarantinedRow>(
        "SELECT event_id, storage_class,
                producer_sequence, producer_signature,
                signing_key_id, producer_id,
                event_type, event_time, source,
                payload_json, payload_blob_ref, run_id,
                specversion,
                schema_bundle_id, schema_bundle_hash, datacontenttype,
                predicted_a_tokens, predicted_b_tokens, predicted_c_tokens,
                reserved_strategy, prediction_strategy_used,
                prediction_policy_used, tokenizer_tier, tokenizer_version_id,
                prediction_confidence, prediction_sample_size,
                cold_start_layer_used,
                run_projection_at_decision_atomic,
                run_predicted_remaining_steps, run_steps_completed_so_far,
                actual_input_tokens, actual_output_tokens,
                delta_b_ratio, delta_c_ratio
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
        // Copy-typed scalars destructured up front; owned String / Vec
        // remain on `r` for the INSERT bind below to borrow.
        let event_id = r.event_id;
        let producer_sequence = r.producer_sequence;
        let event_time = r.event_time;
        let run_id = r.run_id;
        let orig_schema_bundle_id = r.schema_bundle_id;
        let payload_json_opt = r.payload_json.clone();
        let payload_blob_ref_opt = r.payload_blob_ref.clone();

        let storage_class = match r.storage_class.as_str() {
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

        // Codex CA-P1.5 r2 P2: release-from-quarantine path lost
        // failure_class — the quarantine table didn't carry it, so
        // a previously-classified outcome would land with
        // failure_class=NULL and disappear from rule queries. Re-
        // classify here from the same payload we're about to
        // persist (idempotent: classify.rs is pure).
        let decoded_data = crate::classify::decode_payload_data(&payload_json);
        let release_failure_class = crate::classify::classify_audit_outcome(
            &r.event_type,
            decoded_data.as_ref(),
        )
        .map(|c| c.as_db_str());

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
        .bind(&r.event_type)
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

            // Round-3 fix B2: extend INSERT with 18 prediction columns
            // carried forward from the quarantined row. Without this the
            // canonical_events_outcome_required_cols_chk CHECK constraint
            // would fail on outcomes past the 2027-01-01 cutoff, because
            // actual_input_tokens / actual_output_tokens would be NULL
            // even though the payload (CloudEvent proto bytes) carries
            // the values. The producer's signature still covers the
            // proto payload — the first-class columns are SQL
            // accelerators; preserving them through release keeps the
            // mirror cross-check (verify-chain §11.2) consistent.
            sqlx::query(
                "INSERT INTO canonical_events (
                    event_id, tenant_id, decision_id, run_id, event_type,
                    storage_class,
                    producer_id, producer_sequence, producer_signature, signing_key_id,
                    schema_bundle_id, schema_bundle_hash,
                    specversion, source, event_time, datacontenttype,
                    payload_json, payload_blob_ref,
                    region_id, ingest_shard_id, ingest_log_offset, ingest_at,
                    recorded_month, failure_class,
                    predicted_a_tokens, predicted_b_tokens, predicted_c_tokens,
                    reserved_strategy, prediction_strategy_used,
                    prediction_policy_used, tokenizer_tier, tokenizer_version_id,
                    prediction_confidence, prediction_sample_size,
                    cold_start_layer_used,
                    run_projection_at_decision_atomic,
                    run_predicted_remaining_steps, run_steps_completed_so_far,
                    actual_input_tokens, actual_output_tokens,
                    delta_b_ratio, delta_c_ratio
                 ) VALUES (
                    $1, $2, $3, $4, $5,
                    $6,
                    $7, $8, $9, $10,
                    $11, $12,
                    $13, $14, $15, $16,
                    $17, $18,
                    $19, $20, $21, clock_timestamp(),
                    date_trunc('month', $15)::DATE, $22,
                    $23, $24, $25,
                    $26, $27,
                    $28, $29, $30,
                    $31, $32,
                    $33,
                    $34,
                    $35, $36,
                    $37, $38,
                    $39, $40
                 )",
            )
            .bind(event_id)
            .bind(tenant_id)
            .bind(decision_id)
            .bind(run_id)
            .bind(&r.event_type)
            .bind(storage_class.as_db_str())
            .bind(&r.producer_id)
            .bind(producer_sequence)
            .bind(&r.producer_signature)
            .bind(&r.signing_key_id)
            .bind(orig_schema_bundle_id)
            .bind(&r.schema_bundle_hash)
            .bind(&r.specversion)
            .bind(&r.source)
            .bind(event_time)
            .bind(&r.datacontenttype)
            .bind(&payload_json)
            .bind(payload_blob_ref_opt.as_deref())
            .bind(region_id)
            .bind(ingest_shard_id)
            .bind(offset)
            .bind(release_failure_class)
            .bind(r.predicted_a_tokens)
            .bind(r.predicted_b_tokens)
            .bind(r.predicted_c_tokens)
            .bind(&r.reserved_strategy)
            .bind(&r.prediction_strategy_used)
            .bind(&r.prediction_policy_used)
            .bind(&r.tokenizer_tier)
            .bind(r.tokenizer_version_id)
            .bind(r.prediction_confidence.as_ref())
            .bind(r.prediction_sample_size)
            .bind(&r.cold_start_layer_used)
            .bind(r.run_projection_at_decision_atomic.as_ref())
            .bind(r.run_predicted_remaining_steps)
            .bind(r.run_steps_completed_so_far)
            .bind(r.actual_input_tokens)
            .bind(r.actual_output_tokens)
            .bind(r.delta_b_ratio)
            .bind(r.delta_c_ratio)
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

    // Round-3 fix B2: bind 18 prediction columns into quarantine so the
    // release path can re-hydrate them. SLICE_01 callers pass Default
    // (all None → SQL NULL); SLICE_06 callers populate from decoded
    // CloudEvent.
    sqlx::query(
        "INSERT INTO audit_outcome_quarantine (
            quarantine_id, event_id, tenant_id, decision_id,
            storage_class, producer_id, producer_sequence,
            producer_signature, signing_key_id,
            schema_bundle_id, schema_bundle_hash,
            event_type, specversion, source, event_time, datacontenttype,
            payload_json, payload_blob_ref,
            region_id, ingest_shard_id, ingest_log_offset, run_id,
            orphan_after,
            predicted_a_tokens, predicted_b_tokens, predicted_c_tokens,
            reserved_strategy, prediction_strategy_used,
            prediction_policy_used, tokenizer_tier, tokenizer_version_id,
            prediction_confidence, prediction_sample_size,
            cold_start_layer_used,
            run_projection_at_decision_atomic,
            run_predicted_remaining_steps, run_steps_completed_so_far,
            actual_input_tokens, actual_output_tokens,
            delta_b_ratio, delta_c_ratio
         ) VALUES (
            $1, $2, $3, $4,
            $5, $6, $7,
            $8, $9,
            $10, $11,
            $12, $13, $14, $15, $16,
            $17, $18,
            $19, $20, $21, $22,
            $23,
            $24, $25, $26,
            $27, $28,
            $29, $30, $31,
            $32, $33,
            $34,
            $35,
            $36, $37,
            $38, $39,
            $40, $41
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
    .bind(input.prediction.predicted_a_tokens)
    .bind(input.prediction.predicted_b_tokens)
    .bind(input.prediction.predicted_c_tokens)
    .bind(input.prediction.reserved_strategy)
    .bind(input.prediction.prediction_strategy_used)
    .bind(input.prediction.prediction_policy_used)
    .bind(input.prediction.tokenizer_tier)
    .bind(input.prediction.tokenizer_version_id)
    .bind(input.prediction.prediction_confidence.as_ref())
    .bind(input.prediction.prediction_sample_size)
    .bind(input.prediction.cold_start_layer_used)
    .bind(input.prediction.run_projection_at_decision_atomic.as_ref())
    .bind(input.prediction.run_predicted_remaining_steps)
    .bind(input.prediction.run_steps_completed_so_far)
    .bind(input.prediction.actual_input_tokens)
    .bind(input.prediction.actual_output_tokens)
    .bind(input.prediction.delta_b_ratio)
    .bind(input.prediction.delta_c_ratio)
    .execute(pool)
    .await
    .map_err(map_pg_error)?;

    Ok(())
}

// ============================================================================
// Round-3 fix B2: typed quarantine row for release_quarantined_outcomes.
//
// sqlx's tuple FromRow impl caps at 16 elements; we now SELECT 34 columns
// from audit_outcome_quarantine. The named struct also makes the call
// site readable (no more positional destructuring with 34 elements).
// ============================================================================
#[derive(Debug, Clone, sqlx::FromRow)]
struct QuarantinedRow {
    event_id: Uuid,
    storage_class: String,
    producer_sequence: i64,
    producer_signature: Vec<u8>,
    signing_key_id: String,
    producer_id: String,
    event_type: String,
    event_time: DateTime<Utc>,
    source: String,
    payload_json: Option<serde_json::Value>,
    payload_blob_ref: Option<String>,
    run_id: Option<Uuid>,
    specversion: String,
    schema_bundle_id: Uuid,
    schema_bundle_hash: Vec<u8>,
    datacontenttype: String,
    // Round-3 B2 prediction columns.
    predicted_a_tokens: Option<i64>,
    predicted_b_tokens: Option<i64>,
    predicted_c_tokens: Option<i64>,
    reserved_strategy: Option<String>,
    prediction_strategy_used: Option<String>,
    prediction_policy_used: Option<String>,
    tokenizer_tier: Option<String>,
    tokenizer_version_id: Option<Uuid>,
    prediction_confidence: Option<BigDecimal>,
    prediction_sample_size: Option<i64>,
    cold_start_layer_used: Option<String>,
    run_projection_at_decision_atomic: Option<BigDecimal>,
    run_predicted_remaining_steps: Option<i32>,
    run_steps_completed_so_far: Option<i64>,
    actual_input_tokens: Option<i64>,
    actual_output_tokens: Option<i64>,
    delta_b_ratio: Option<f32>,
    delta_c_ratio: Option<f32>,
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
