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
use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgPool, Postgres};
use sqlx::Row;
use tracing::{debug, info};
use uuid::Uuid;

/// Identifier for the Postgres advisory lock per spec §8.3. Chosen as a
/// stable random i64 so concurrent stats_aggregator deployments
/// (e.g. blue-green rollout) compete for the same lock; only the lock
/// holder runs the cycle.
pub const STATS_AGGREGATOR_ADVISORY_LOCK_ID: i64 = 0x5350_4441_4747_5253_u64 as i64; // "SPDAGGRS"

/// One bucket's pre-computed aggregate as read back for drift detection.
///
/// R2 M2 (Software F5): drift_detector::compute_z_score must use a
/// baseline window that EXCLUDES the current 7d window. Spec §7.1:
///
/// ```text
/// baseline_mean = mean over [now - 30d, now - 7d]
/// baseline_stddev = stddev over [now - 30d, now - 7d]
/// current_mean = mean_7d
/// z = (current_mean - baseline_mean) / baseline_stddev
/// ```
///
/// Previously we naively used mean_30d (which INCLUDES the last 7 days)
/// as the baseline — the current window thus contributes to its own
/// reference, biasing z toward 0 and masking real drift. The R2 fix
/// adds dedicated baseline_* fields filled from a separate window query.
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
    /// R2 M2: window over [now - 30d, now - 7d] — baseline used by
    /// drift detector. Distinct from mean_30d (which is the full 30d
    /// window for Strategy B's cache lookup).
    pub baseline_mean: Option<f32>,
    pub baseline_stddev: Option<f32>,
    pub baseline_sample_size: Option<i32>,
}

/// Acquire the singleton advisory lock per spec §8.3 on a *pinned*
/// PoolConnection.
///
/// R2 B2 (Backend + Software + DB B3): `pg_try_advisory_lock` is
/// session-bound — releasing it on a different connection is a no-op.
/// The R1 shape called `pool.fetch_one()` (arbitrary checkout) for the
/// acquire and `pool.execute()` (different checkout) for the release,
/// so the lock leaked and every subsequent cycle skipped the cycle.
///
/// R2 returns the PoolConnection so the scheduler can hold it for the
/// entire cycle and explicitly release on the same connection. The
/// connection also serves any cycle-wide control queries (per-tenant
/// transactions are still checked out from the pool — `pool.begin()` —
/// because each tenant transaction needs its own commit boundary; the
/// advisory lock itself only needs to live on the pinned connection).
///
/// Returns:
///   * `Ok(Some(conn))` — lock acquired; caller proceeds with the cycle.
///     Caller MUST call [`release_lock_conn`] on the returned connection
///     when done.
///   * `Ok(None)` — another instance holds the lock; caller skips the
///     cycle and emits the stats_aggregator_skipped_lock_held metric.
///
/// Per spec §10 — Postgres session-level locks auto-release on session
/// disconnect, so a panicking scheduler doesn't permanently leak. TCP
/// keepalives govern stale-session detection (~ 2h Linux default;
/// operators tune via tcp_keepalives_idle on the Postgres side).
pub async fn try_acquire_lock_conn(
    pool: &PgPool,
) -> Result<Option<PoolConnection<Postgres>>, anyhow::Error> {
    let mut conn = pool
        .acquire()
        .await
        .context("acquire pool conn for advisory lock")?;
    let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(STATS_AGGREGATOR_ADVISORY_LOCK_ID)
        .fetch_one(&mut *conn)
        .await
        .context("pg_try_advisory_lock")?;
    if acquired {
        info!(
            lock_id = STATS_AGGREGATOR_ADVISORY_LOCK_ID,
            "acquired stats_aggregator advisory lock (pinned)"
        );
        Ok(Some(conn))
    } else {
        info!(
            lock_id = STATS_AGGREGATOR_ADVISORY_LOCK_ID,
            "advisory lock held by another instance; skipping cycle"
        );
        // Drop the connection — return it to the pool. Postgres
        // pg_try_advisory_lock returns FALSE without taking the lock,
        // so there is nothing to release.
        drop(conn);
        Ok(None)
    }
}

/// Release the advisory lock on the connection that acquired it.
///
/// R2 B2: must run on the same session that called
/// `pg_try_advisory_lock` — Postgres bookkeeping is per-session. The
/// connection is then dropped (returned to the pool).
///
/// Failure to release is non-fatal: the lock will release when the
/// session disconnects (immediately on drop here, since we drop the
/// connection at the end). We still surface the error so operators see
/// any anomalies in the cycle metrics.
pub async fn release_lock_conn(mut conn: PoolConnection<Postgres>) -> Result<(), anyhow::Error> {
    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(STATS_AGGREGATOR_ADVISORY_LOCK_ID)
        .execute(&mut *conn)
        .await
        .context("pg_advisory_unlock")?;
    debug!(
        lock_id = STATS_AGGREGATOR_ADVISORY_LOCK_ID,
        "released stats_aggregator advisory lock"
    );
    Ok(())
}

/// Discover the set of distinct tenants present in canonical_events for
/// the last 30 days. Per spec §8.3 we iterate per-tenant for transaction
/// isolation.
///
/// R2 B4: column is `ingest_at` (matches canonical_events.ingest_at —
/// insertion-time stamp) not `recorded_at`. R2 M7: add the
/// recorded_month partition-pruning predicate so the planner stays on
/// the active monthly partitions instead of scanning the default
/// partition.
pub async fn discover_active_tenants(pool: &PgPool) -> Result<Vec<Uuid>, anyhow::Error> {
    let rows = sqlx::query(
        r#"
        SELECT DISTINCT tenant_id
        FROM canonical_events
        WHERE event_type = 'spendguard.audit.outcome'
          AND ingest_at >= now() - interval '30 days'
          AND recorded_month >= DATE_TRUNC('month', now() - interval '30 days')::DATE
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
    //
    // R2 B3 (Software B4): GROUP BY the 7-class `prompt_class` enum —
    // NOT prompt_class_fingerprint. output-predictor spec §8.2 closing
    // paragraph: "Aggregator key uses class itself, not fingerprint —
    // fingerprint is the audit identifier, class is the aggregation
    // bucket." Stitching by the enum means the predictor's L4 cache
    // lookup (keyed on req.prompt_class) hits real rows.
    //
    // R2 B4 (DB B1): columns referenced are the first-class mirror
    // columns added in 0018 (model, agent_id, prompt_class) — NOT the
    // base64-decoded JSON path. ingest_at not recorded_at (matches the
    // real canonical_events.ingest_at). recorded_month partition-prune
    // predicate (R2 M7) pins the planner to active partitions.
    //
    // GA_07 R5 arbitration: sidecar commit/outcome rows are intentionally
    // sparse; the paired decision row is the authoritative source for
    // predictor bucket mirrors. Aggregate actual output tokens from
    // outcome rows, then join by decision_id to recover the decision's
    // (model, agent_id, prompt_class) when the outcome did not duplicate
    // those fields.
    let agg_30d = sqlx::query(
        r#"
        SELECT
          COALESCE(outcome.model, decision.model) AS model,
          COALESCE(outcome.agent_id, decision.agent_id) AS agent_id,
          COALESCE(outcome.prompt_class, decision.prompt_class) AS prompt_class,
          percentile_cont(0.50) WITHIN GROUP (ORDER BY outcome.actual_output_tokens)::REAL AS p50_30d,
          percentile_cont(0.95) WITHIN GROUP (ORDER BY outcome.actual_output_tokens)::REAL AS p95_30d,
          percentile_cont(0.99) WITHIN GROUP (ORDER BY outcome.actual_output_tokens)::REAL AS p99_30d,
          avg(outcome.actual_output_tokens)::REAL AS mean_30d,
          stddev_samp(outcome.actual_output_tokens)::REAL AS stddev_30d,
          count(*)::INT AS sample_size_30d
        FROM canonical_events outcome
        LEFT JOIN canonical_events decision
          ON decision.tenant_id = outcome.tenant_id
         AND decision.decision_id = outcome.decision_id
         AND decision.event_type = 'spendguard.audit.decision'
         AND decision.recorded_month >= DATE_TRUNC('month', now() - interval '30 days')::DATE
        WHERE outcome.event_type = 'spendguard.audit.outcome'
          AND outcome.actual_output_tokens IS NOT NULL
          AND outcome.ingest_at >= now() - interval '30 days'
          AND outcome.recorded_month >= DATE_TRUNC('month', now() - interval '30 days')::DATE
          AND outcome.tenant_id = $1
          AND COALESCE(outcome.model, decision.model) IS NOT NULL
          AND COALESCE(outcome.agent_id, decision.agent_id) IS NOT NULL
          AND COALESCE(outcome.prompt_class, decision.prompt_class) IS NOT NULL
        GROUP BY 1, 2, 3
        "#,
    )
    .bind(tenant_id)
    .fetch_all(&mut *tx)
    .await
    .context("aggregate 30d window")?;

    let agg_7d = sqlx::query(
        r#"
        SELECT
          COALESCE(outcome.model, decision.model) AS model,
          COALESCE(outcome.agent_id, decision.agent_id) AS agent_id,
          COALESCE(outcome.prompt_class, decision.prompt_class) AS prompt_class,
          percentile_cont(0.50) WITHIN GROUP (ORDER BY outcome.actual_output_tokens)::REAL AS p50_7d,
          percentile_cont(0.95) WITHIN GROUP (ORDER BY outcome.actual_output_tokens)::REAL AS p95_7d,
          percentile_cont(0.99) WITHIN GROUP (ORDER BY outcome.actual_output_tokens)::REAL AS p99_7d,
          avg(outcome.actual_output_tokens)::REAL AS mean_7d,
          stddev_samp(outcome.actual_output_tokens)::REAL AS stddev_7d,
          count(*)::INT AS sample_size_7d
        FROM canonical_events outcome
        LEFT JOIN canonical_events decision
          ON decision.tenant_id = outcome.tenant_id
         AND decision.decision_id = outcome.decision_id
         AND decision.event_type = 'spendguard.audit.decision'
         AND decision.recorded_month >= DATE_TRUNC('month', now() - interval '7 days')::DATE
        WHERE outcome.event_type = 'spendguard.audit.outcome'
          AND outcome.actual_output_tokens IS NOT NULL
          AND outcome.ingest_at >= now() - interval '7 days'
          AND outcome.recorded_month >= DATE_TRUNC('month', now() - interval '7 days')::DATE
          AND outcome.tenant_id = $1
          AND COALESCE(outcome.model, decision.model) IS NOT NULL
          AND COALESCE(outcome.agent_id, decision.agent_id) IS NOT NULL
          AND COALESCE(outcome.prompt_class, decision.prompt_class) IS NOT NULL
        GROUP BY 1, 2, 3
        "#,
    )
    .bind(tenant_id)
    .fetch_all(&mut *tx)
    .await
    .context("aggregate 7d window")?;

    // R2 M2: drift baseline — distinct window [now-30d, now-7d] that
    // EXCLUDES the current 7d so the baseline doesn't contaminate the
    // z-score. Same column shape as the other two queries.
    let agg_baseline = sqlx::query(
        r#"
        SELECT
          COALESCE(outcome.model, decision.model) AS model,
          COALESCE(outcome.agent_id, decision.agent_id) AS agent_id,
          COALESCE(outcome.prompt_class, decision.prompt_class) AS prompt_class,
          avg(outcome.actual_output_tokens)::REAL AS baseline_mean,
          stddev_samp(outcome.actual_output_tokens)::REAL AS baseline_stddev,
          count(*)::INT AS baseline_sample_size
        FROM canonical_events outcome
        LEFT JOIN canonical_events decision
          ON decision.tenant_id = outcome.tenant_id
         AND decision.decision_id = outcome.decision_id
         AND decision.event_type = 'spendguard.audit.decision'
         AND decision.recorded_month >= DATE_TRUNC('month', now() - interval '30 days')::DATE
        WHERE outcome.event_type = 'spendguard.audit.outcome'
          AND outcome.actual_output_tokens IS NOT NULL
          AND outcome.ingest_at < now() - interval '7 days'
          AND outcome.ingest_at >= now() - interval '30 days'
          AND outcome.recorded_month >= DATE_TRUNC('month', now() - interval '30 days')::DATE
          AND outcome.tenant_id = $1
          AND COALESCE(outcome.model, decision.model) IS NOT NULL
          AND COALESCE(outcome.agent_id, decision.agent_id) IS NOT NULL
          AND COALESCE(outcome.prompt_class, decision.prompt_class) IS NOT NULL
        GROUP BY 1, 2, 3
        "#,
    )
    .bind(tenant_id)
    .fetch_all(&mut *tx)
    .await
    .context("aggregate drift baseline window [now-30d, now-7d]")?;

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
                baseline_mean: None,
                baseline_stddev: None,
                baseline_sample_size: None,
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
            baseline_mean: None,
            baseline_stddev: None,
            baseline_sample_size: None,
        });
        entry.mean_7d = r.try_get::<f32, _>("mean_7d").ok();
        entry.stddev_7d = r.try_get::<f32, _>("stddev_7d").ok();
        entry.sample_size_7d = r.try_get::<i32, _>("sample_size_7d").ok();
    }
    // R2 M2: stitch baseline window in. Buckets without any baseline
    // data (new bucket; first activity is in the last 7 days) keep
    // baseline_* None — drift_detector treats this as insufficient
    // signal and skips the alert per the existing None-guard.
    for r in &agg_baseline {
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
            baseline_mean: None,
            baseline_stddev: None,
            baseline_sample_size: None,
        });
        entry.baseline_mean = r.try_get::<f32, _>("baseline_mean").ok();
        entry.baseline_stddev = r.try_get::<f32, _>("baseline_stddev").ok();
        entry.baseline_sample_size = r.try_get::<i32, _>("baseline_sample_size").ok();
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
        assert_eq!(
            STATS_AGGREGATOR_ADVISORY_LOCK_ID,
            0x5350_4441_4747_5253_u64 as i64
        );
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
            baseline_mean: Some(95.0),
            baseline_stddev: Some(20.0),
            baseline_sample_size: Some(120),
        };
        assert!(b.mean_7d.is_none());
        assert_eq!(b.sample_size_30d, Some(150));
        assert_eq!(b.baseline_sample_size, Some(120));
    }
}
