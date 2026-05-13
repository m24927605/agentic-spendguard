//! retention-sweeper binary entry-point.
//!
//! Mirrors ttl_sweeper / outbox_forwarder pattern: leader-elected
//! poll loop. Only the leader sweeps so two replicas don't redact
//! the same rows concurrently. Round-9 `is_leader_now()` gating
//! ensures a stalled lease renewal doesn't keep the worker thinking
//! it's still leader.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use serde::Deserialize;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tokio::time::sleep;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use spendguard_leases::{
    spawn_lease_loop, DisabledLease, K8sLease, LeaseConfig, LeaseManager, LeaseState,
    PostgresLease,
};

use spendguard_retention_sweeper::{log_sweep, sweep_audit_outbox_prompts, sweep_provider_usage_raw};

#[derive(Debug, Deserialize)]
struct Config {
    database_url: String,
    #[serde(default = "default_poll_interval_seconds")]
    poll_interval_seconds: u64,
    #[serde(default = "default_batch_size")]
    batch_size: i64,

    /// Leader election (mirrors ttl_sweeper / outbox_forwarder).
    #[serde(default = "default_lease_mode")]
    leader_election_mode: String,
    #[serde(default = "default_lease_name")]
    leader_lease_name: String,
    #[serde(default)]
    workload_instance_id: String,
    #[serde(default = "default_region")]
    leader_region: String,
    #[serde(default = "default_lease_ttl_ms")]
    leader_lease_ttl_ms: u64,
    #[serde(default = "default_renew_interval_ms")]
    leader_renew_interval_ms: u64,
    #[serde(default = "default_retry_interval_ms")]
    leader_retry_interval_ms: u64,
}

fn default_poll_interval_seconds() -> u64 {
    600
}
fn default_batch_size() -> i64 {
    500
}
fn default_lease_mode() -> String {
    "postgres".into()
}
fn default_lease_name() -> String {
    "retention-sweeper".into()
}
fn default_region() -> String {
    "demo".into()
}
fn default_lease_ttl_ms() -> u64 {
    30_000
}
fn default_renew_interval_ms() -> u64 {
    10_000
}
fn default_retry_interval_ms() -> u64 {
    5_000
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("failed to install rustls aws_lc_rs default provider"))?;

    let envfilter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,spendguard_retention_sweeper=debug"));
    tracing_subscriber::fmt()
        .with_env_filter(envfilter)
        .with_target(false)
        .json()
        .init();

    let cfg: Config = envy::prefixed("SPENDGUARD_RETENTION_SWEEPER_")
        .from_env()
        .context("loading config")?;

    info!(
        poll_interval = cfg.poll_interval_seconds,
        batch_size = cfg.batch_size,
        leader_mode = %cfg.leader_election_mode,
        "retention-sweeper starting"
    );

    let pool: PgPool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&cfg.database_url)
        .await
        .context("connecting to Postgres")?;

    let lease_cfg = LeaseConfig {
        lease_name: cfg.leader_lease_name.clone(),
        workload_id: if cfg.workload_instance_id.is_empty() {
            // Fallback identifier; production deployments populate via
            // downward API.
            format!("retention-sweeper-{}", uuid::Uuid::new_v4())
        } else {
            cfg.workload_instance_id.clone()
        },
        region: cfg.leader_region.clone(),
        ttl: Duration::from_millis(cfg.leader_lease_ttl_ms),
        renew_interval: Duration::from_millis(cfg.leader_renew_interval_ms),
        retry_interval: Duration::from_millis(cfg.leader_retry_interval_ms),
    };

    let manager: Arc<dyn LeaseManager> = match cfg.leader_election_mode.as_str() {
        "postgres" => Arc::new(PostgresLease::new(pool.clone(), lease_cfg.clone())?),
        "k8s" => {
            let namespace = std::env::var("SPENDGUARD_LEADER_K8S_NAMESPACE")
                .unwrap_or_else(|_| "default".into());
            let ttl_seconds = std::cmp::max(1, (cfg.leader_lease_ttl_ms / 1000) as i32);
            Arc::new(
                K8sLease::new(
                    namespace,
                    lease_cfg.lease_name.clone(),
                    lease_cfg.workload_id.clone(),
                    ttl_seconds,
                )
                .await?,
            )
        }
        "disabled" => Arc::new(DisabledLease {
            lease_name: lease_cfg.lease_name.clone(),
            workload_id: lease_cfg.workload_id.clone(),
        }),
        other => anyhow::bail!("unknown leader_election_mode {other}"),
    };

    let guard = spawn_lease_loop(manager, lease_cfg);
    let poll_dur = Duration::from_secs(cfg.poll_interval_seconds);

    info!("entering retention sweep loop");
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("ctrl-c received; exiting");
                break;
            }
            _ = sleep(poll_dur) => {
                let s = guard.state_rx.borrow().clone();
                if !s.is_leader_now() {
                    match &s {
                        LeaseState::Leader { expires_at, .. } => {
                            warn!(expires_at = %expires_at, "lease expired locally; skip sweep");
                        }
                        LeaseState::Standby { holder_workload_id, .. } => {
                            tracing::debug!(held_by = %holder_workload_id, "standby — skip sweep");
                        }
                        LeaseState::Unknown => {
                            warn!("lease state Unknown — skip sweep");
                        }
                    }
                    continue;
                }

                // Two passes per cycle: prompt redaction, then provider_raw.
                match sweep_audit_outbox_prompts(&pool, cfg.batch_size).await {
                    Ok(out) => {
                        if let Err(e) = log_sweep(&pool, &out).await {
                            error!(err = %e, "log_sweep failed for audit_outbox");
                        }
                    }
                    Err(e) => error!(err = %e, "sweep_audit_outbox_prompts failed"),
                }
                match sweep_provider_usage_raw(&pool, cfg.batch_size).await {
                    Ok(out) => {
                        if let Err(e) = log_sweep(&pool, &out).await {
                            error!(err = %e, "log_sweep failed for provider_usage_records");
                        }
                    }
                    Err(e) => error!(err = %e, "sweep_provider_usage_raw failed"),
                }
            }
        }
    }

    guard.shutdown().await;
    Ok(())
}
