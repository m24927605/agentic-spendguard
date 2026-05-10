use std::sync::Arc;
use std::time::Duration;

use spendguard_leases::{
    spawn_lease_loop, DisabledLease, K8sLease, LeaseConfig, LeaseManager,
    LeaseState, PostgresLease,
};
use tokio::time::sleep;
use tracing::{error, info, warn};
use uuid::Uuid;

use spendguard_ttl_sweeper::{
    config::Config,
    metrics::{LoopOutcome, SkipReason, TtlSweeperMetrics},
    poll::fetch_expired,
    sequence::{recover_max_seq, SequenceAllocator},
    state::{build_ledger_client, build_pg_pool, AppState},
    sweep::sweep_one,
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
        ledger_url = %config.ledger_url,
        poll_interval_seconds = config.poll_interval_seconds,
        batch_size = config.batch_size,
        leader_mode = %config.leader_election_mode,
        lease_name = %config.leader_lease_name,
        "ttl-sweeper starting"
    );

    let pg = build_pg_pool(&config.database_url).await?;

    let tenant_uuid = Uuid::parse_str(&config.tenant_id)?;
    let max_seq = recover_max_seq(&pg, tenant_uuid, &config.workload_instance_id).await?;
    let seq_start = max_seq + 1;
    info!(workload = %config.workload_instance_id, max_seq, seq_start, "producer_sequence recovered");
    let seq = SequenceAllocator::new(seq_start);

    let ledger_client = build_ledger_client(&config).await?;

    // Phase 5 GA hardening S6: producer signer.
    let signer = std::sync::Arc::<dyn spendguard_signing::Signer>::from(
        spendguard_signing::signer_from_env("SPENDGUARD_TTL_SWEEPER")
            .map_err(|e| anyhow::anyhow!("S6: build signer: {e}"))?,
    );
    info!(
        key_id = %signer.key_id(),
        algorithm = %signer.algorithm(),
        producer = %signer.producer_identity(),
        "S6: producer signer initialized"
    );

    let mut state = AppState {
        config: config.clone(),
        pg: pg.clone(),
        ledger_client,
        seq,
        signer,
    };

    // Phase 5 S1: leader election. Only the leader sweeps expired reservations.
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
            // Followup #5: K8sLease::new is async (calls
            // kube::Client::try_default for in-cluster
            // ServiceAccount). leaderElection.ttlMs is the
            // operator-supplied lease duration; convert to seconds
            // for the k8s API.
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
    let metrics = TtlSweeperMetrics::new();
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
    info!("entering sweep loop");

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("ctrl-c received; exiting");
                break;
            }
            _ = sleep(poll_dur) => {
                let s = guard.state_rx.borrow().clone();
                // Codex round-9 P2: use expiry-aware is_leader_now()
                // instead of plain pattern match. If lease renewal has
                // stalled past expires_at, the watch channel still
                // holds the last Leader value; sweeping under stale
                // leadership lets two pods concurrently UPDATE the
                // same expired-reservation row.
                if s.is_leader_now() {
                    match fetch_expired(&state.pg, tenant_uuid, state.config.batch_size).await {
                        Ok(rows) => {
                            metrics.inc_loop(LoopOutcome::Processed);
                            if rows.is_empty() {
                                tracing::debug!("no expired reservations");
                            } else {
                                info!(count = rows.len(), "found expired reservations (leader)");
                                for row in rows {
                                    match sweep_one(&mut state, row).await {
                                        Ok(_) => metrics.add_swept(1, true),
                                        Err(e) => {
                                            metrics.add_swept(1, false);
                                            error!(error = ?e, "sweep_one failed");
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            metrics.inc_loop(LoopOutcome::Error);
                            error!(error = ?e, "fetch_expired failed");
                        }
                    }
                } else {
                    metrics.inc_loop(LoopOutcome::Skipped);
                    match &s {
                        LeaseState::Leader { expires_at, .. } => {
                            metrics.inc_skip(SkipReason::LeaseExpired);
                            warn!(expires_at = %expires_at, "lease expired locally; skip sweep until renewed");
                        }
                        LeaseState::Standby { holder_workload_id, .. } => {
                            metrics.inc_skip(SkipReason::Standby);
                            tracing::debug!(held_by = %holder_workload_id, "standby — skip sweep");
                        }
                        LeaseState::Unknown => {
                            metrics.inc_skip(SkipReason::Unknown);
                            warn!("lease state Unknown — skip sweep");
                        }
                    }
                }
            }
        }
    }

    guard.shutdown().await;
    Ok(())
}

/// Round-2 #11: minimal HTTP /metrics endpoint.
async fn serve_metrics(addr: String, metrics: TtlSweeperMetrics) -> anyhow::Result<()> {
    use std::convert::Infallible;
    use hyper::body::Bytes;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper::{Request, Response};
    use hyper_util::rt::TokioIo;
    use http_body_util::Full;
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(&addr).await?;
    info!(addr = %addr, "ttl-sweeper metrics listening");

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
