//! Per-(tenant, agent_id) run-length distribution aggregation per spec
//! stats-aggregator-spec-v1alpha1.md §6.
//!
//! Consumed by run_cost_projector (SLICE_09 — Signal 1 cost projection).
//! SLICE_06 writes the cache; SLICE_09 wires the consumer.

use anyhow::Context;
use sqlx::postgres::PgPool;
use uuid::Uuid;

/// Aggregate run lengths for one tenant + UPSERT into
/// `run_length_distribution_cache`. Spec §6.
pub async fn aggregate_run_length(pool: &PgPool, tenant_id: Uuid) -> Result<(), anyhow::Error> {
    let mut tx = pool.begin().await.context("begin run-length tx")?;

    // R2 B1: set RLS session variable per migration 0017 FOR ALL policy.
    // Without this the UPSERT below would fail with "new row violates
    // row-level security policy". SET LOCAL flavour auto-resets at COMMIT.
    sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await
        .context("set RLS tenant_id for run_length_distribution_cache")?;

    // Spec §6 — compute per-agent run lengths from decision events,
    // then aggregate percentiles + count of distinct run_ids.
    //
    // R2 B4 (DB B1): query first-class mirror columns from 0018 —
    // agent_id, run_id_mirror (the UUID-typed run_id; mirror suffix
    // disambiguates from canonical_events.run_id audit anchor).
    // ingest_at not recorded_at. recorded_month partition-prune
    // predicate per R2 M7 keeps the planner on active partitions.
    sqlx::query(
        r#"
        WITH run_lengths AS (
          SELECT
            tenant_id,
            agent_id,
            run_id_mirror AS run_id,
            count(*)::INT AS steps_in_run
          FROM canonical_events
          WHERE event_type = 'spendguard.audit.decision'
            AND ingest_at >= now() - interval '30 days'
            AND recorded_month >= DATE_TRUNC('month', now() - interval '30 days')::DATE
            AND tenant_id = $1
            AND agent_id IS NOT NULL
            AND run_id_mirror IS NOT NULL
          GROUP BY tenant_id, agent_id, run_id_mirror
        )
        INSERT INTO run_length_distribution_cache (
          tenant_id, agent_id,
          p50_steps_30d, p95_steps_30d, p99_steps_30d,
          mean_steps_30d, stddev_steps_30d, sample_size_30d,
          computed_at, aggregation_version
        )
        SELECT
          tenant_id, agent_id,
          percentile_cont(0.50) WITHIN GROUP (ORDER BY steps_in_run)::REAL,
          percentile_cont(0.95) WITHIN GROUP (ORDER BY steps_in_run)::REAL,
          percentile_cont(0.99) WITHIN GROUP (ORDER BY steps_in_run)::REAL,
          avg(steps_in_run)::REAL,
          stddev_samp(steps_in_run)::REAL,
          count(*)::INT,
          now(), 'v1alpha1'
        FROM run_lengths
        GROUP BY tenant_id, agent_id
        ON CONFLICT (tenant_id, agent_id)
          DO UPDATE SET
            p50_steps_30d = EXCLUDED.p50_steps_30d,
            p95_steps_30d = EXCLUDED.p95_steps_30d,
            p99_steps_30d = EXCLUDED.p99_steps_30d,
            mean_steps_30d = EXCLUDED.mean_steps_30d,
            stddev_steps_30d = EXCLUDED.stddev_steps_30d,
            sample_size_30d = EXCLUDED.sample_size_30d,
            computed_at = now(),
            aggregation_version = 'v1alpha1'
        "#,
    )
    .bind(tenant_id)
    .execute(&mut *tx)
    .await
    .context("aggregate run lengths")?;

    tx.commit().await.context("commit run-length tx")?;
    Ok(())
}
