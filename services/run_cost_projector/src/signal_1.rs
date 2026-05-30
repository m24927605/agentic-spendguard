//! Signal 1 — induced from history.
//!
//! Spec ref `run-cost-projector-spec-v1alpha1.md` §3.
//!
//! ## Algorithm
//!
//! ```text
//! predicted_remaining_steps_signal1 =
//!     max(0, run_length_p95(tenant_id, agent_id) - steps_completed_so_far)
//! ```
//!
//! `run_length_p95` reads `run_length_distribution_cache.p95_steps_30d` from
//! the canonical_ingest DB (table created by SLICE_06 migration 0017;
//! consumer wired here).
//!
//! ## Cold start
//!
//! When the bucket has no row OR `sample_size_30d` is too small (spec §3.2
//! mentions "no sample" as the trigger, treated here as NULL row), fall back
//! to the configurable cold-start default (spec default = 10).
//!
//! ## Universal coverage
//!
//! Per spec §3.3 Signal 1 fires for ANY agent — no framework cooperation
//! required. The cache writer (stats_aggregator) writes per (tenant_id,
//! agent_id) regardless of SDK.

use sqlx::{PgPool, Row};
use thiserror::Error;
use tracing::warn;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum Signal1Error {
    #[error("sqlx error: {0}")]
    Sql(#[from] sqlx::Error),
}

/// Compute Signal 1 predicted_remaining_steps. Returns `(predicted_steps,
/// is_cold_start)`. `is_cold_start = true` when the cache miss path was
/// used and the cold_start_default was substituted.
///
/// The signature takes `Option<&PgPool>` so skeleton mode (no DB) can call
/// this with `None` and always get the cold-start default (this is what the
/// production Helm gate prevents but the demo mode tolerates).
pub async fn signal_1_predicted_remaining_steps(
    pool: Option<&PgPool>,
    tenant_id: Uuid,
    agent_id: &str,
    steps_completed: i64,
    cold_start_default_run_length: i32,
) -> Result<(i32, bool), Signal1Error> {
    let p95_steps = match pool {
        Some(p) => query_p95_steps(p, tenant_id, agent_id).await?,
        None => None,
    };

    match p95_steps {
        Some(p95) => {
            // p95 is REAL (f32 wire); round to nearest int, floor at 0.
            // We use floor here (Floor + 0 saturation) — Signal 1 is a
            // conservative estimator; over-counting steps would inflate
            // projection and false-positive RUN_BUDGET_PROJECTION_EXCEEDED.
            // Floor is fine because Signal 2 augments + drift detection
            // catches uplift.
            let p95_int = p95.max(0.0) as i32;
            let remaining = (p95_int as i64 - steps_completed).max(0).min(i32::MAX as i64) as i32;
            Ok((remaining, false))
        }
        None => {
            // Cache miss → cold-start default per spec §3.2.
            // Subtract steps_completed so the "predicted REMAINING" semantic
            // holds even on cold-start (the run may already be past
            // cold_start_default).
            let remaining = (cold_start_default_run_length as i64 - steps_completed)
                .max(0)
                .min(i32::MAX as i64) as i32;
            Ok((remaining, true))
        }
    }
}

/// Look up the 30-day P95 run length from the canonical_ingest DB.
/// Returns `None` on cache miss (the bucket has no aggregation row yet).
///
/// RLS-aware: wraps the SELECT in a tx that SETs app.current_tenant_id
/// per the run_length_distribution_cache table's RLS policy
/// (services/canonical_ingest/migrations/0017_run_length_distribution_cache.sql).
async fn query_p95_steps(
    pool: &PgPool,
    tenant_id: Uuid,
    agent_id: &str,
) -> Result<Option<f32>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL app.current_tenant_id = $1")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;

    let row = sqlx::query(
        r#"
        SELECT p95_steps_30d
          FROM run_length_distribution_cache
         WHERE tenant_id = $1
           AND agent_id  = $2
         LIMIT 1
        "#,
    )
    .bind(tenant_id)
    .bind(agent_id)
    .fetch_optional(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(row.and_then(|r| r.try_get::<Option<f32>, _>("p95_steps_30d").ok().flatten()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cold-start path: no pool → default minus steps_completed.
    #[tokio::test]
    async fn cold_start_when_no_pool() {
        let (remaining, is_cold) =
            signal_1_predicted_remaining_steps(None, Uuid::new_v4(), "ag-1", 0, 10)
                .await
                .expect("ok");
        assert_eq!(remaining, 10);
        assert!(is_cold);
    }

    #[tokio::test]
    async fn cold_start_caps_at_zero_when_past_default() {
        let (remaining, is_cold) =
            signal_1_predicted_remaining_steps(None, Uuid::new_v4(), "ag-1", 15, 10)
                .await
                .expect("ok");
        assert_eq!(remaining, 0, "steps_completed=15 > default=10 → 0 remaining");
        assert!(is_cold);
    }

    /// Negative steps_completed (shouldn't happen but defense-in-depth).
    #[tokio::test]
    async fn cold_start_handles_negative_steps_completed_gracefully() {
        // steps_completed should never be negative but i64 wire allows it.
        // The implementation does `(default - steps_completed).max(0)`,
        // which for steps_completed = -5 yields default + 5 = 15.
        // Document this property here so a future change that clamps
        // steps_completed upstream doesn't accidentally regress recovery.
        let (remaining, _) =
            signal_1_predicted_remaining_steps(None, Uuid::new_v4(), "ag-1", -5, 10)
                .await
                .expect("ok");
        assert_eq!(remaining, 15);
    }
}
