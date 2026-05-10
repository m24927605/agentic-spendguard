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
    metrics::{LoopOutcome, OutboxForwarderMetrics, SkipReason},
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
        "k8s" => {
            // Followup #5: K8sLease::new is async — see ttl_sweeper
            // for context.
            let namespace = std::env::var("SPENDGUARD_LEADER_K8S_NAMESPACE")
                .unwrap_or_else(|_| "default".into());
            let ttl_seconds = std::cmp::max(
                1,
                (config.leader_lease_ttl_ms / 1000) as i32,
            );
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
        other => anyhow::bail!("unknown leader_election_mode {}", other),
    };
    let guard = spawn_lease_loop(manager, lease_cfg);

    // Round-2 #11: Prometheus metrics counter store + HTTP server.
    let metrics = OutboxForwarderMetrics::new();
    if !config.metrics_addr.is_empty() {
        let metrics_addr = config.metrics_addr.clone();
        let metrics_handle = metrics.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_metrics(metrics_addr, metrics_handle).await {
                warn!(err = %e, "metrics server terminated");
            }
        });
        info!(addr = %config.metrics_addr, "metrics server bound");
    }

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
                // Codex round-9 P2: use expiry-aware is_leader_now()
                // instead of plain pattern match. A stalled renewal
                // task could leave the watch channel holding a stale
                // Leader value; forwarding under expired leadership
                // would let two pods double-send the same outbox row
                // to the same downstream sink.
                if s.is_leader_now() {
                    match forward_batch(&mut state).await {
                        Ok(0) => {
                            metrics.inc_loop(LoopOutcome::Processed);
                            tracing::debug!("no pending rows");
                        }
                        Ok(n) => {
                            metrics.inc_loop(LoopOutcome::Processed);
                            metrics.add_rows_forwarded(n as u64, true);
                            info!(count = n, "batch processed (leader)");
                        }
                        Err(e) => {
                            metrics.inc_loop(LoopOutcome::Error);
                            metrics.add_rows_forwarded(1, false);
                            error!(error = ?e, "forward_batch failed");
                        }
                    }
                } else {
                    metrics.inc_loop(LoopOutcome::Skipped);
                    match &s {
                        LeaseState::Leader { expires_at, .. } => {
                            metrics.inc_skip(SkipReason::LeaseExpired);
                            warn!(expires_at = %expires_at, "lease expired locally; skip batch until renewed");
                        }
                        LeaseState::Standby { holder_workload_id, .. } => {
                            metrics.inc_skip(SkipReason::Standby);
                            tracing::debug!(held_by = %holder_workload_id, "standby — skip batch");
                        }
                        LeaseState::Unknown => {
                            metrics.inc_skip(SkipReason::Unknown);
                            warn!("lease state Unknown — skip batch");
                        }
                    }
                }
            }
        }
    }

    guard.shutdown().await;
    Ok(())
}

/// Round-2 #11: minimal HTTP /metrics endpoint that renders the
/// OutboxForwarderMetrics Prometheus text. Same hyper-based pattern
/// as the other services.
async fn serve_metrics(addr: String, metrics: OutboxForwarderMetrics) -> anyhow::Result<()> {
    use std::convert::Infallible;
    use hyper::body::Bytes;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper::{Request, Response};
    use hyper_util::rt::TokioIo;
    use http_body_util::Full;
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(&addr).await?;
    info!(addr = %addr, "outbox-forwarder metrics listening");

    loop {
        let (stream, _peer) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let metrics = metrics.clone();
        tokio::task::spawn(async move {
            let svc = service_fn(move |req: Request<hyper::body::Incoming>| {
                let metrics = metrics.clone();
                async move {
                    let body = if req.uri().path() == "/metrics" {
                        metrics.render()
                    } else {
                        "".to_string()
                    };
                    Ok::<_, Infallible>(
                        Response::builder()
                            .header(
                                "content-type",
                                "text/plain; version=0.0.4; charset=utf-8",
                            )
                            .body(Full::new(Bytes::from(body)))
                            .unwrap(),
                    )
                }
            });
            if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
                tracing::debug!(err = %e, "metrics conn closed");
            }
        });
    }
}
