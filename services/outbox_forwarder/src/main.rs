use std::sync::Arc;
use std::time::Duration;

use spendguard_leases::{
    spawn_lease_loop, DisabledLease, K8sLease, LeaseConfig, LeaseManager,
    LeaseState, PostgresLease,
};
use tokio::time::sleep;
use tracing::{error, info, warn};

use spendguard_outbox_forwarder::{
    config::Config,
    forward::forward_batch,
    state::{build_canonical_client, build_pg_pool, AppState},
};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("failed to install rustls aws_lc_rs default provider"))?;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .json()
        .init();

    let config = Config::from_env()?;
    info!(
        canonical_ingest_url = %config.canonical_ingest_url,
        poll_interval_seconds = config.poll_interval_seconds,
        batch_size = config.batch_size,
        leader_mode = %config.leader_election_mode,
        lease_name = %config.leader_lease_name,
        "outbox-forwarder starting"
    );

    let pg = build_pg_pool(&config.database_url).await?;
    let canonical_client = build_canonical_client(&config).await?;

    let mut state = AppState {
        config: config.clone(),
        pg: pg.clone(),
        canonical_client,
    };

    // Phase 5 S1: leader election. Only the leader processes batches.
    let lease_cfg = LeaseConfig {
        lease_name: config.leader_lease_name.clone(),
        workload_id: config.workload_instance_id.clone(),
        region: config.leader_region.clone(),
        ttl: Duration::from_millis(config.leader_lease_ttl_ms),
        renew_interval: Duration::from_millis(config.leader_renew_interval_ms),
        retry_interval: Duration::from_millis(config.leader_retry_interval_ms),
    };
    let manager: Arc<dyn LeaseManager> = match config.leader_election_mode.as_str() {
        "postgres" => Arc::new(PostgresLease::new(pg.clone(), lease_cfg.clone())?),
        "k8s" => Arc::new(K8sLease {
            namespace: std::env::var("SPENDGUARD_LEADER_K8S_NAMESPACE")
                .unwrap_or_else(|_| "default".into()),
            lease_name: lease_cfg.lease_name.clone(),
            workload_id: lease_cfg.workload_id.clone(),
        }),
        "disabled" => Arc::new(DisabledLease {
            lease_name: lease_cfg.lease_name.clone(),
            workload_id: lease_cfg.workload_id.clone(),
        }),
        other => anyhow::bail!("unknown leader_election_mode {}", other),
    };
    let guard = spawn_lease_loop(manager, lease_cfg);

    let poll_dur = Duration::from_secs(config.poll_interval_seconds);
    info!("entering forward loop");

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("ctrl-c received; exiting");
                break;
            }
            _ = sleep(poll_dur) => {
                // S1 invariant: only the leader processes work. Standby
                // pods sleep + wait for state changes.
                let s = guard.state_rx.borrow().clone();
                match s {
                    LeaseState::Leader { .. } => {
                        match forward_batch(&mut state).await {
                            Ok(0) => tracing::debug!("no pending rows"),
                            Ok(n) => info!(count = n, "batch processed (leader)"),
                            Err(e) => error!(error = ?e, "forward_batch failed"),
                        }
                    }
                    LeaseState::Standby { holder_workload_id, .. } => {
                        tracing::debug!(held_by = %holder_workload_id, "standby — skip batch");
                    }
                    LeaseState::Unknown => {
                        warn!("lease state Unknown — skip batch");
                    }
                }
            }
        }
    }

    guard.shutdown().await;
    Ok(())
}
