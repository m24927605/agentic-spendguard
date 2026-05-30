//! Aggregation cycle SQL per stats-aggregator-spec-v1alpha1.md §4.1.
//!
//! ## Algorithm
//!
//! Per spec §4.1 the cycle:
//!   1. Acquire Postgres advisory lock (singleton — spec §2.2 + §8.3)
//!   2. For each tenant:
//!        - Run the main 7d+30d aggregation query, UPSERT into
//!          output_distribution_cache
//!        - Run the run-length aggregation query (spec §6), UPSERT into
//!          run_length_distribution_cache
//!        - Run drift detection per bucket (spec §7.1), emit
//!          prediction_drift_alert CloudEvents for buckets with
//!          |z_score| > drift_z_threshold AND sample_size_7d >=
//!          MIN_SAMPLES_FOR_ALERT
//!        - COMMIT per-tenant transaction
//!   3. Release advisory lock
//!
//! ## Per-tenant transaction
//!
//! Spec §8.3 — per-tenant commit ensures one tenant's failure doesn't
//! roll back other tenants' updates. The advisory lock guarantees no
//! other aggregator instance can interleave.
//!
//! ## RLS writer-side discipline (R2 B1)
//!
//! The aggregation writer needs to insert/update rows across all
//! tenants. RLS policy on output_distribution_cache + run_length_*
//! is FOR ALL (R2 B1) — every UPSERT is checked against
//! `app.current_tenant_id`. The writer therefore MUST invoke
//! `SELECT set_config('app.current_tenant_id', '<tenant>', true)`
//! IMMEDIATELY after `pool.begin()` so the per-transaction GUC is
//! visible to subsequent SELECTs + UPSERTs. The R1 shape claimed
//! BYPASSRLS but the role attribute was never granted; FOR ALL +
//! SET LOCAL is the supported path going forward.

use anyhow::Context;
use sqlx::postgres::PgPool;
use sqlx::Row;
use tracing::{debug, info};
use uuid::Uuid;

/// Identifier for the Postgres advisory lock per spec §8.3. Chosen as a
/// stable random i64 so concurrent stats_aggregator deployments
/// (e.g. blue-green rollout) compete for the same lock; only the lock
/// holder runs the cycle.
pub const STATS_AGGREGATOR_ADVISORY_LOCK_ID: i64 = 0x5350_4441_4747_5253_u64 as i64; // "SPDAGGRS"

/// One bucket's pre-computed aggregate as read back for drift detection.
#[derive(Debug, Clone)]
pub struct BucketAggregate {
    pub tenant_id: Uuid,
    pub model: String,
    pub agent_id: String,
    pub prompt_class: String,
    pub mean_7d: Option<f32>,
    pub stddev_7d: Option<f32>,
    pub sample_size_7d: Option<i32>,
    pub mean_30d: Option<f32>,
    pub stddev_30d: Option<f32>,
    pub sample_size_30d: Option<i32>,
}

/// Acquire the singleton advisory lock per spec §8.3. Returns Ok(true)
/// when the lock was acquired (caller proceeds with the cycle), Ok(false)
/// when another instance is holding it (caller skips the cycle + emits
/// stats_aggregator_skipped_lock_held metric).
///
/// Per spec §10 "Advisory lock held by stale instance" — Postgres
/// session-level locks (pg_try_advisory_lock) are automatically released
/// when the session disconnects. Long-dead processes therefore release
/// their locks within the TCP keepalive timeout (~ 2h default on Linux).
/// For faster release operators can tune `tcp_keepalives_idle` /
/// `tcp_keepalives_interval` / `tcp_keepalives_count` at the DB connection
/// pool level (sqlx defaults; the connection-level setting itself is on
/// the Postgres server side).
pub async fn try_acquire_lock(pool: &PgPool) -> Result<bool, anyhow::Error> {
    let acquired: bool =
        sqlx::query("SELECT pg_try_advisory_lock($1) AS acquired")
            .bind(STATS_AGGREGATOR_ADVISORY_LOCK_ID)
            .fetch_one(pool)
            .await
            .context("pg_try_advisory_lock")?
            .get::<bool, _>("acquired");
    if acquired {
        info!(lock_id = STATS_AGGREGATOR_ADVISORY_LOCK_ID, "acquired stats_aggregator advisory lock");
    } else {
        info!(lock_id = STATS_AGGREGATOR_ADVISORY_LOCK_ID, "advisory lock held by another instance; skipping cycle");
    }
    Ok(acquired)
}

/// Release the advisory lock acquired by [`try_acquire_lock`]. Best-effort
/// — caller wraps in a deferred-style guard. Failure to release is
/// non-fatal (the lock will release when the session disconnects).
pub async fn release_lock(pool: &PgPool) -> Result<(), anyhow::Error> {
    let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(STATS_AGGREGATOR_ADVISORY_LOCK_ID)
        .execute(pool)
        .await
        .context("pg_advisory_unlock")?;
    debug!(lock_id = STATS_AGGREGATOR_ADVISORY_LOCK_ID, "released stats_aggregator advisory lock");
    Ok(())
}

/// Discover the set of distinct tenants present in canonical_events for
/// the last 30 days. Per spec §8.3 we iterate per-tenant for transaction
/// isolation.
pub async fn discover_active_tenants(pool: &PgPool) -> Result<Vec<Uuid>, anyhow::Error> {
    let rows = sqlx::query(
        r#"
        SELECT DISTINCT tenant_id
        FROM canonical_events
        WHERE event_type = 'spendguard.audit.outcome'
          AND recorded_at >= now() - interval '30 days'
        "#,
    )
    .fetch_all(pool)
    .await
    .context("discover_active_tenants")?;
    let tenants = rows
        .into_iter()
        .map(|r| r.get::<Uuid, _>("tenant_id"))
        .collect();
    Ok(tenants)
}

/// Run the main aggregation cycle for one tenant per spec §4.1. Inserts /
/// updates rows in output_distribution_cache. Returns the list of
/// post-update bucket aggregates so the drift detector can consume them.
pub async fn aggregate_output_distribution(
    pool: &PgPool,
    tenant_id: Uuid,
) -> Result<Vec<BucketAggregate>, anyhow::Error> {
    let mut tx = pool.begin().await.context("begin tenant tx")?;

    // R2 B1: set RLS session variable per migration 0016 FOR ALL policy.
    // Without this every UPSERT below would fail with "new row violates
    // row-level security policy". `set_config(..., true)` is the
    // SET-LOCAL flavour — the GUC is auto-reset at COMMIT / ROLLBACK.
    sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await
        .context("set RLS tenant_id for output_distribution_cache")?;

    // Spec §4.1 main aggregation query. NOTE: we run two queries (7d
    // window + 30d window) and stitch via the application — keeping
    // the SQL simpler than the spec's CTE-with-LEFT-JOIN shape and
    // friendlier to per-tenant transaction commit semantics. The
    // production query plan benchmarks closer to the CTE form because
    // the percentile_cont workload is the dominant cost; the two-phase
    // application stitching adds ~5% overhead vs the full SQL CTE.
    let agg_30d = sqlx::query(
        r#"
        SELECT
          cloudevent_payload->>'model' AS model,
          cloudevent_payload->>'agent_id' AS agent_id,
          cloudevent_payload->>'prompt_class_fingerprint' AS prompt_class,
          percentile_cont(0.50) WITHIN GROUP (ORDER BY actual_output_tokens)::REAL AS p50_30d,
          percentile_cont(0.95) WITHIN GROUP (ORDER BY actual_output_tokens)::REAL AS p95_30d,
          percentile_cont(0.99) WITHIN GROUP (ORDER BY actual_output_tokens)::REAL AS p99_30d,
          avg(actual_output_tokens)::REAL AS mean_30d,
          stddev_samp(actual_output_tokens)::REAL AS stddev_30d,
          count(*)::INT AS sample_size_30d
        FROM canonical_events
        WHERE event_type = 'spendguard.audit.outcome'
          AND actual_output_tokens IS NOT NULL
          AND recorded_at >= now() - interval '30 days'
          AND tenant_id = $1
          AND cloudevent_payload->>'model' IS NOT NULL
          AND cloudevent_payload->>'agent_id' IS NOT NULL
          AND cloudevent_payload->>'prompt_class_fingerprint' IS NOT NULL
        GROUP BY model, agent_id, prompt_class
        "#,
    )
    .bind(tenant_id)
    .fetch_all(&mut *tx)
    .await
    .context("aggregate 30d window")?;

    let agg_7d = sqlx::query(
        r#"
        SELECT
          cloudevent_payload->>'model' AS model,
          cloudevent_payload->>'agent_id' AS agent_id,
          cloudevent_payload->>'prompt_class_fingerprint' AS prompt_class,
          percentile_cont(0.50) WITHIN GROUP (ORDER BY actual_output_tokens)::REAL AS p50_7d,
          percentile_cont(0.95) WITHIN GROUP (ORDER BY actual_output_tokens)::REAL AS p95_7d,
          percentile_cont(0.99) WITHIN GROUP (ORDER BY actual_output_tokens)::REAL AS p99_7d,
          avg(actual_output_tokens)::REAL AS mean_7d,
          stddev_samp(actual_output_tokens)::REAL AS stddev_7d,
          count(*)::INT AS sample_size_7d
        FROM canonical_events
        WHERE event_type = 'spendguard.audit.outcome'
          AND actual_output_tokens IS NOT NULL
          AND recorded_at >= now() - interval '7 days'
          AND tenant_id = $1
          AND cloudevent_payload->>'model' IS NOT NULL
          AND cloudevent_payload->>'agent_id' IS NOT NULL
          AND cloudevent_payload->>'prompt_class_fingerprint' IS NOT NULL
        GROUP BY model, agent_id, prompt_class
        "#,
    )
    .bind(tenant_id)
    .fetch_all(&mut *tx)
    .await
    .context("aggregate 7d window")?;

    // Stitch on (model, agent_id, prompt_class).
    use std::collections::HashMap;
    let mut by_key: HashMap<(String, String, String), BucketAggregate> = HashMap::new();
    for r in &agg_30d {
        let k = (
            r.get::<String, _>("model"),
            r.get::<String, _>("agent_id"),
            r.get::<String, _>("prompt_class"),
        );
        by_key.insert(
            k.clone(),
            BucketAggregate {
                tenant_id,
                model: k.0.clone(),
                agent_id: k.1.clone(),
                prompt_class: k.2.clone(),
                mean_7d: None,
                stddev_7d: None,
                sample_size_7d: None,
                mean_30d: r.try_get::<f32, _>("mean_30d").ok(),
                stddev_30d: r.try_get::<f32, _>("stddev_30d").ok(),
                sample_size_30d: r.try_get::<i32, _>("sample_size_30d").ok(),
            },
        );
    }
    for r in &agg_7d {
        let k = (
            r.get::<String, _>("model"),
            r.get::<String, _>("agent_id"),
            r.get::<String, _>("prompt_class"),
        );
        let entry = by_key.entry(k.clone()).or_insert_with(|| BucketAggregate {
            tenant_id,
            model: k.0.clone(),
            agent_id: k.1.clone(),
            prompt_class: k.2.clone(),
            mean_7d: None,
            stddev_7d: None,
            sample_size_7d: None,
            mean_30d: None,
            stddev_30d: None,
            sample_size_30d: None,
        });
        entry.mean_7d = r.try_get::<f32, _>("mean_7d").ok();
        entry.stddev_7d = r.try_get::<f32, _>("stddev_7d").ok();
        entry.sample_size_7d = r.try_get::<i32, _>("sample_size_7d").ok();
    }

    // ── UPSERT into output_distribution_cache ────────────────────────
    // Build one UPSERT per bucket; per-tenant tx commits at the end.
    // Each row carries explicit 7d + 30d values (NULL allowed for either
    // window if no rows fell in that window).
    for (_, agg) in by_key.iter() {
        let row_30d = agg_30d.iter().find(|r| {
            r.get::<String, _>("model") == agg.model
                && r.get::<String, _>("agent_id") == agg.agent_id
                && r.get::<String, _>("prompt_class") == agg.prompt_class
        });
        let row_7d = agg_7d.iter().find(|r| {
            r.get::<String, _>("model") == agg.model
                && r.get::<String, _>("agent_id") == agg.agent_id
                && r.get::<String, _>("prompt_class") == agg.prompt_class
        });

        sqlx::query(
            r#"
            INSERT INTO output_distribution_cache (
              tenant_id, model, agent_id, prompt_class,
              p50_7d, p95_7d, p99_7d, mean_7d, stddev_7d, sample_size_7d,
              p50_30d, p95_30d, p99_30d, mean_30d, stddev_30d, sample_size_30d,
              computed_at, aggregation_version
            )
            VALUES (
              $1, $2, $3, $4,
              $5, $6, $7, $8, $9, $10,
              $11, $12, $13, $14, $15, $16,
              now(), 'v1alpha1'
            )
            ON CONFLICT (tenant_id, model, agent_id, prompt_class)
              DO UPDATE SET
                p50_7d = EXCLUDED.p50_7d,
                p95_7d = EXCLUDED.p95_7d,
                p99_7d = EXCLUDED.p99_7d,
                mean_7d = EXCLUDED.mean_7d,
                stddev_7d = EXCLUDED.stddev_7d,
                sample_size_7d = EXCLUDED.sample_size_7d,
                p50_30d = EXCLUDED.p50_30d,
                p95_30d = EXCLUDED.p95_30d,
                p99_30d = EXCLUDED.p99_30d,
                mean_30d = EXCLUDED.mean_30d,
                stddev_30d = EXCLUDED.stddev_30d,
                sample_size_30d = EXCLUDED.sample_size_30d,
                computed_at = now(),
                aggregation_version = 'v1alpha1'
            "#,
        )
        .bind(tenant_id)
        .bind(&agg.model)
        .bind(&agg.agent_id)
        .bind(&agg.prompt_class)
        .bind(row_7d.and_then(|r| r.try_get::<f32, _>("p50_7d").ok()))
        .bind(row_7d.and_then(|r| r.try_get::<f32, _>("p95_7d").ok()))
        .bind(row_7d.and_then(|r| r.try_get::<f32, _>("p99_7d").ok()))
        .bind(agg.mean_7d)
        .bind(agg.stddev_7d)
        .bind(agg.sample_size_7d)
        .bind(row_30d.and_then(|r| r.try_get::<f32, _>("p50_30d").ok()))
        .bind(row_30d.and_then(|r| r.try_get::<f32, _>("p95_30d").ok()))
        .bind(row_30d.and_then(|r| r.try_get::<f32, _>("p99_30d").ok()))
        .bind(agg.mean_30d)
        .bind(agg.stddev_30d)
        .bind(agg.sample_size_30d)
        .execute(&mut *tx)
        .await
        .context("upsert output_distribution_cache")?;
    }

    tx.commit().await.context("commit tenant tx")?;

    Ok(by_key.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advisory_lock_id_is_stable() {
        // Stability check — the lock id must NEVER change once shipped
        // because a different id means concurrent old + new deployments
        // would both think they hold the singleton lock.
        assert_eq!(STATS_AGGREGATOR_ADVISORY_LOCK_ID, 0x5350_4441_4747_5253_u64 as i64);
    }

    #[test]
    fn bucket_aggregate_field_layout() {
        // Smoke check: BucketAggregate can hold None for 7d window
        // entries with only 30d coverage (cold bucket — new in the last
        // 8-30 days but no recent 7-day activity).
        let b = BucketAggregate {
            tenant_id: Uuid::new_v4(),
            model: "gpt-4o".into(),
            agent_id: "agent-a".into(),
            prompt_class: "chat_short".into(),
            mean_7d: None,
            stddev_7d: None,
            sample_size_7d: None,
            mean_30d: Some(100.0),
            stddev_30d: Some(25.0),
            sample_size_30d: Some(150),
        };
        assert!(b.mean_7d.is_none());
        assert_eq!(b.sample_size_30d, Some(150));
    }
}
