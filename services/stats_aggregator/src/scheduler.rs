//! Cycle scheduler per spec stats-aggregator-spec-v1alpha1.md §8.
//!
//! ## Loop shape
//!
//! ```text
//! loop {
//!   sleep(cycle_seconds);
//!   if try_acquire_lock() {
//!     for tenant in discover_active_tenants():
//!       aggregate_output_distribution(tenant)
//!       aggregate_run_length(tenant)
//!     detect_and_emit(aggregates)
//!     release_lock()
//!   } else {
//!     metric: stats_aggregator_skipped_lock_held
//!   }
//! }
//! ```
//!
//! Per spec §8.1 default cadence is hourly. Per-tenant cadence override
//! (spec §8.2) is deferred to SLICE-extra; the scheduler currently
//! treats every tenant uniformly.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use sqlx::postgres::PgPool;
use tokio::time::interval;
use tracing::{error, info, warn};

use crate::aggregation::{
    aggregate_output_distribution, discover_active_tenants, release_lock_conn,
    try_acquire_lock_conn,
};
use crate::drift_detector::{detect_and_emit, DriftAlertSink, DriftDetectorConfig};
use crate::run_length::aggregate_run_length;
use spendguard_signing::Signer;

// R2 M13 (Software F15): real counters incremented from the cycle
// loop. Rendered into the /metrics body. Exposed as `pub static` so
// render_metrics in main.rs reads the live values without a global
// registry.
pub static CYCLES_TOTAL: AtomicU64 = AtomicU64::new(0);
pub static SKIPPED_LOCK_HELD_TOTAL: AtomicU64 = AtomicU64::new(0);
pub static DRIFT_ALERTS_TOTAL: AtomicU64 = AtomicU64::new(0);
pub static CYCLE_ERROR_TOTAL: AtomicU64 = AtomicU64::new(0);
pub static LAST_CYCLE_START_UNIX_SECS: AtomicU64 = AtomicU64::new(0);

/// One cycle's outcome surfaced for metric counters.
#[derive(Debug, Clone)]
pub struct CycleOutcome {
    pub lock_acquired: bool,
    pub tenants_processed: usize,
    pub alerts_emitted: usize,
    pub error_count: usize,
}

/// Run one full aggregation cycle. Exposed separately from the loop so
/// integration tests can invoke a single cycle.
pub async fn run_one_cycle(
    pool: &PgPool,
    cfg: &DriftDetectorConfig,
    signer: Arc<dyn Signer>,
    sink: Arc<dyn DriftAlertSink>,
) -> Result<CycleOutcome, anyhow::Error> {
    let mut outcome = CycleOutcome {
        lock_acquired: false,
        tenants_processed: 0,
        alerts_emitted: 0,
        error_count: 0,
    };

    // R2 M13: stamp cycle-start so /healthz + /readyz can compute
    // last_cycle_age for staleness alerting (used by M8 wiring).
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    LAST_CYCLE_START_UNIX_SECS.store(now_unix, Ordering::Relaxed);
    CYCLES_TOTAL.fetch_add(1, Ordering::Relaxed);

    // R2 B2: pin a PoolConnection for the entire cycle so the advisory
    // lock acquire + release run on the same session. Per-tenant
    // transactions are still checked out separately via pool.begin().
    let lock_conn = match try_acquire_lock_conn(pool).await.context("acquire lock")? {
        Some(c) => c,
        None => {
            SKIPPED_LOCK_HELD_TOTAL.fetch_add(1, Ordering::Relaxed);
            return Ok(outcome);
        }
    };
    outcome.lock_acquired = true;

    // Run the cycle body — capture errors so we can still release the
    // lock before returning. Per spec §8.3 the lock must release even
    // if any tenant aggregation panics (Postgres auto-releases on
    // session disconnect, but we want a clean release in normal flow).
    let cycle_result = run_cycle_body(pool, cfg, signer, sink, &mut outcome).await;

    if let Err(e) = release_lock_conn(lock_conn).await {
        warn!(error = %e, "release_lock failed (Postgres will auto-release on session disconnect)");
    }

    cycle_result?;
    Ok(outcome)
}

/// Inner cycle body — separated so the outer wrapper can release the
/// advisory lock on both Ok + Err paths.
async fn run_cycle_body(
    pool: &PgPool,
    cfg: &DriftDetectorConfig,
    signer: Arc<dyn Signer>,
    sink: Arc<dyn DriftAlertSink>,
    outcome: &mut CycleOutcome,
) -> Result<(), anyhow::Error> {
    let tenants = match discover_active_tenants(pool).await {
        Ok(t) => t,
        Err(e) => {
            error!(error = %e, "discover_active_tenants failed");
            outcome.error_count += 1;
            return Ok(());
        }
    };
    info!(tenant_count = tenants.len(), "discovered active tenants");

    let mut all_aggregates = Vec::new();
    for tenant_id in tenants {
        match aggregate_output_distribution(pool, tenant_id).await {
            Ok(aggs) => {
                outcome.tenants_processed += 1;
                all_aggregates.extend(aggs);
            }
            Err(e) => {
                warn!(tenant_id = %tenant_id, error = %e, "aggregate_output_distribution failed; other tenants continue");
                outcome.error_count += 1;
            }
        }
        if let Err(e) = aggregate_run_length(pool, tenant_id).await {
            warn!(tenant_id = %tenant_id, error = %e, "aggregate_run_length failed; other tenants continue");
            outcome.error_count += 1;
        }
    }

    match detect_and_emit(&all_aggregates, cfg, signer.as_ref(), sink.as_ref()).await {
        Ok(n) => {
            outcome.alerts_emitted = n;
            // R2 M13: live counter for alert emission rate.
            DRIFT_ALERTS_TOTAL.fetch_add(n as u64, Ordering::Relaxed);
            info!(alerts = n, "drift detection complete");
        }
        Err(e) => {
            error!(error = %e, "detect_and_emit failed; alerts may be lost");
            outcome.error_count += 1;
        }
    }

    // R2 M13: aggregate cycle-level error count → global counter
    // (gauges per-cycle errors via outcome; counter aggregates over the
    // process lifetime).
    if outcome.error_count > 0 {
        CYCLE_ERROR_TOTAL.fetch_add(outcome.error_count as u64, Ordering::Relaxed);
    }

    Ok(())
}

/// Long-lived scheduler loop. Returns only on graceful shutdown.
pub async fn run_loop(
    pool: PgPool,
    cycle_seconds: u64,
    cfg: DriftDetectorConfig,
    signer: Arc<dyn Signer>,
    sink: Arc<dyn DriftAlertSink>,
) {
    let mut ticker = interval(Duration::from_secs(cycle_seconds));
    // Tick once immediately so the first cycle runs at startup, then
    // wait `cycle_seconds` between subsequent ticks.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;
        match run_one_cycle(&pool, &cfg, Arc::clone(&signer), Arc::clone(&sink)).await {
            Ok(outcome) => {
                info!(
                    lock_acquired = outcome.lock_acquired,
                    tenants_processed = outcome.tenants_processed,
                    alerts_emitted = outcome.alerts_emitted,
                    error_count = outcome.error_count,
                    "cycle complete"
                );
            }
            Err(e) => {
                error!(error = %e, "run_one_cycle returned error");
            }
        }
    }
}
