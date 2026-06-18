//! Phase 5 GA hardening S11: OpenAI usage poller binary.
//!
//! Background worker: leader-elected, periodic poll, idempotent
//! insert into provider_usage_records. The actual matching SP that
//! converts records into ProviderReport calls is the S10-followup;
//! this binary's job is to KEEP THE RECORDS LANDING.

use anyhow::Context;
use serde::Deserialize;
use spendguard_leases::{
    spawn_lease_loop, DisabledLease, K8sLease, LeaseConfig, LeaseManager, LeaseState, PostgresLease,
};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use spendguard_usage_poller::{
    metrics::{CycleOutcome, UsagePollerMetrics},
    poll_once, AnthropicClient, MockProviderClient, OpenAiClient, PollWindow, ProviderClient,
};

#[derive(Debug, Deserialize)]
struct Config {
    database_url: String,
    /// `mock` | `openai` (default `mock` for demo and CI).
    #[serde(default = "default_provider_kind")]
    provider_kind: String,
    /// Poll interval seconds.
    #[serde(default = "default_poll_interval_seconds")]
    poll_interval_seconds: u64,
    /// How far behind now() the poll window's right edge sits — gives
    /// late-arriving provider records time to land before the cursor
    /// advances past them.
    #[serde(default = "default_safety_lag_seconds")]
    safety_lag_seconds: u64,
    /// How far back from cursor to re-poll on each cycle to catch
    /// updates to events that landed near the cursor.
    #[serde(default = "default_overlap_minutes")]
    overlap_minutes: i64,
    /// OpenAI keys (only when provider_kind=openai).
    #[serde(default)]
    openai_api_key: Option<String>,
    #[serde(default)]
    openai_org_id: Option<String>,
    #[serde(default)]
    openai_project_id: Option<String>,
    /// Anthropic keys (only when provider_kind=anthropic).
    #[serde(default)]
    anthropic_api_key: Option<String>,
    #[serde(default)]
    anthropic_workspace_id: Option<String>,
    /// Round-2 #11: Prometheus /metrics endpoint bind addr. Defaults
    /// to `0.0.0.0:9099` per the round-2 port table. Empty disables.
    #[serde(default = "default_metrics_addr")]
    metrics_addr: String,

    // Phase 5 S1 — leader election (mirrors ttl_sweeper /
    // retention_sweeper). The poller MUST be a singleton: each replica
    // keeps its own in-memory cursor and calls the provider API
    // independently, so running >1 replica without a leader multiplies
    // provider-API load N× (cost + 429 risk). Only the leader polls.
    // Env keys are prefixed by `envy::prefixed("SPENDGUARD_USAGE_POLLER_")`
    // below, e.g. SPENDGUARD_USAGE_POLLER_LEADER_ELECTION_MODE.
    #[serde(default = "default_lease_mode")]
    leader_election_mode: String,
    #[serde(default = "default_lease_name")]
    leader_lease_name: String,
    #[serde(default)]
    workload_instance_id: String,
    #[serde(default = "default_lease_region")]
    leader_region: String,
    #[serde(default = "default_lease_ttl_ms")]
    leader_lease_ttl_ms: u64,
    #[serde(default = "default_lease_renew_ms")]
    leader_renew_interval_ms: u64,
    #[serde(default = "default_lease_retry_ms")]
    leader_retry_interval_ms: u64,
}

fn default_metrics_addr() -> String {
    "0.0.0.0:9099".to_string()
}

fn default_lease_mode() -> String {
    "postgres".into()
}
fn default_lease_name() -> String {
    "usage-poller".into()
}
fn default_lease_region() -> String {
    "demo".into()
}
fn default_lease_ttl_ms() -> u64 {
    15_000
}
fn default_lease_renew_ms() -> u64 {
    5_000
}
fn default_lease_retry_ms() -> u64 {
    1_000
}

fn default_provider_kind() -> String {
    "mock".into()
}
fn default_poll_interval_seconds() -> u64 {
    60
}
fn default_safety_lag_seconds() -> u64 {
    300
}
fn default_overlap_minutes() -> i64 {
    5
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("failed to install rustls aws_lc_rs default provider"))?;

    let envfilter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,spendguard_usage_poller=debug"));
    tracing_subscriber::fmt()
        .with_env_filter(envfilter)
        .with_target(false)
        .json()
        .init();

    let cfg: Config = envy::prefixed("SPENDGUARD_USAGE_POLLER_")
        .from_env()
        .context("loading config")?;

    // S1: validate leader-election config before any side effect so a
    // misconfigured lease fails closed at startup (a stalled-renewal
    // window that never expires would let two replicas poll forever).
    let valid_modes = ["postgres", "k8s", "disabled"];
    if !valid_modes.contains(&cfg.leader_election_mode.as_str()) {
        anyhow::bail!(
            "SPENDGUARD_USAGE_POLLER_LEADER_ELECTION_MODE must be one of {:?}, got {}",
            valid_modes,
            cfg.leader_election_mode
        );
    }
    if cfg.leader_renew_interval_ms >= cfg.leader_lease_ttl_ms {
        anyhow::bail!(
            "leader_renew_interval_ms ({}) must be < leader_lease_ttl_ms ({})",
            cfg.leader_renew_interval_ms,
            cfg.leader_lease_ttl_ms
        );
    }

    info!(
        provider_kind = %cfg.provider_kind,
        poll_interval = cfg.poll_interval_seconds,
        safety_lag = cfg.safety_lag_seconds,
        overlap_minutes = cfg.overlap_minutes,
        leader_mode = %cfg.leader_election_mode,
        lease_name = %cfg.leader_lease_name,
        "S11: usage poller starting"
    );

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&cfg.database_url)
        .await
        .context("connecting to Postgres")?;

    // Codex round-10 P2: explicitly enumerate provider kinds. The
    // previous catch-all fell back to mock on any unrecognized value,
    // so a typo like `opneai` would silently disable real collection
    // and emit successful-poll metrics with zero records. Cost
    // accounting then stays blind until someone notices missing data.
    let client: Arc<dyn ProviderClient> = match cfg.provider_kind.as_str() {
        "openai" => {
            let api_key = cfg.openai_api_key.clone().context(
                "SPENDGUARD_USAGE_POLLER_OPENAI_API_KEY required when provider_kind=openai",
            )?;
            Arc::new(OpenAiClient::new(
                api_key,
                cfg.openai_org_id.clone(),
                cfg.openai_project_id.clone(),
            ))
        }
        "anthropic" => {
            let api_key = cfg.anthropic_api_key.clone().context(
                "SPENDGUARD_USAGE_POLLER_ANTHROPIC_API_KEY required when provider_kind=anthropic",
            )?;
            Arc::new(AnthropicClient::new(
                api_key,
                cfg.anthropic_workspace_id.clone(),
            ))
        }
        "mock" => Arc::new(MockProviderClient::new("mock")),
        other => anyhow::bail!(
            "unknown SPENDGUARD_USAGE_POLLER_PROVIDER_KIND={other:?}; \
             expected one of: mock, openai, anthropic"
        ),
    };

    // Codex round-5 P?: in-memory cursor with `now - safety_lag` seed
    // would lose every record landed during a restart longer than
    // safety_lag. The S11-followup persistent `provider_usage_poller_state`
    // table is still pending; until it ships, seed the cursor from
    // `MAX(observed_at) WHERE provider = ...` so a restart resumes
    // from where the last batch landed. ON CONFLICT on the insert
    // path makes wider re-polls idempotent, so an over-conservative
    // cursor is safe; an under-conservative one drops data.
    let max_observed: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT MAX(observed_at) FROM provider_usage_records WHERE provider = $1",
    )
    .bind(&cfg.provider_kind)
    .fetch_one(&pool)
    .await
    .unwrap_or(None);
    let now = chrono::Utc::now();
    let safety_lag_seed = now - chrono::Duration::seconds(cfg.safety_lag_seconds as i64);
    let mut cursor = match max_observed {
        Some(t) => {
            info!(
                seed = "max_observed",
                resume_from = %t,
                "S11: cursor resumed from prior provider_usage_records"
            );
            // Pick the older of (max_observed, now - safety_lag) so we
            // also catch records the previous instance was actively
            // polling around its termination.
            std::cmp::min(t, safety_lag_seed)
        }
        None => {
            info!(
                seed = "first_run",
                resume_from = %safety_lag_seed,
                "S11: no prior records; cursor seeded from now - safety_lag"
            );
            safety_lag_seed
        }
    };

    // Round-2 #11: Prometheus metrics counter store + HTTP server.
    let metrics = UsagePollerMetrics::new();
    if !cfg.metrics_addr.is_empty() {
        let metrics_addr = cfg.metrics_addr.clone();
        let metrics_handle = metrics.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_metrics(metrics_addr, metrics_handle).await {
                warn!(err = %e, "metrics server terminated");
            }
        });
        info!(addr = %cfg.metrics_addr, "metrics server bound");
    }

    // Phase 5 S1: leader election. Only the leader polls so >1 replica
    // doesn't multiply provider-API load / cursors. Mirrors
    // ttl_sweeper / retention_sweeper.
    let lease_cfg = LeaseConfig {
        lease_name: cfg.leader_lease_name.clone(),
        workload_id: if cfg.workload_instance_id.is_empty() {
            // Fallback identifier; production deployments populate via
            // the downward API.
            format!("usage-poller-{}", uuid::Uuid::new_v4())
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
        // Unreachable: validated above, but fail closed.
        other => anyhow::bail!("unknown leader_election_mode {other}"),
    };
    let guard = spawn_lease_loop(manager, lease_cfg);

    // Track whether we were leading on the previous tick. When a
    // standby is promoted, its in-memory cursor is stale (seeded once
    // at startup); re-seed from MAX(observed_at) on takeover so the
    // promoted leader resumes from where the prior leader left off.
    // Idempotent inserts make an over-conservative cursor safe.
    let mut was_leader = false;

    info!("entering poll loop");

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("ctrl-c received; exiting");
                break;
            }
            _ = tokio::time::sleep(Duration::from_secs(cfg.poll_interval_seconds)) => {
                let s = guard.state_rx.borrow().clone();
                // Round-9 P2: expiry-aware is_leader_now() — a stalled
                // renewal must not keep this replica polling after
                // another pod has taken over the lease.
                if !s.is_leader_now() {
                    metrics.inc_cycle(CycleOutcome::Skipped);
                    was_leader = false;
                    match &s {
                        LeaseState::Leader { expires_at, .. } => {
                            warn!(expires_at = %expires_at, "lease expired locally; skip poll until renewed");
                        }
                        LeaseState::Standby { holder_workload_id, .. } => {
                            tracing::debug!(held_by = %holder_workload_id, "standby — skip poll");
                        }
                        LeaseState::Unknown => {
                            warn!("lease state Unknown — skip poll");
                        }
                    }
                    continue;
                }

                if !was_leader {
                    was_leader = true;
                    let resume: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
                        "SELECT MAX(observed_at) FROM provider_usage_records WHERE provider = $1",
                    )
                    .bind(&cfg.provider_kind)
                    .fetch_one(&pool)
                    .await
                    .unwrap_or(None);
                    if let Some(t) = resume {
                        // Never advance the cursor forward on takeover —
                        // only pull it back to be safe. The overlap +
                        // idempotency cover any gap.
                        let promoted = std::cmp::min(cursor, t);
                        info!(resume_from = %promoted, "S11: promoted to leader; cursor re-seeded");
                        cursor = promoted;
                    } else {
                        info!("S11: promoted to leader; no prior records, cursor unchanged");
                    }
                }

                let now = chrono::Utc::now();
                let window_to = now - chrono::Duration::seconds(cfg.safety_lag_seconds as i64);
                let window_from = cursor - chrono::Duration::minutes(cfg.overlap_minutes);
                let window = PollWindow {
                    from: window_from,
                    to: window_to,
                };

                match poll_once(&*client, &pool, window).await {
                    Ok(outcome) => {
                        metrics.inc_cycle(CycleOutcome::Ok);
                        metrics.add_records(
                            outcome.fetched as u64,
                            outcome.inserted as u64,
                            outcome.deduped as u64,
                        );
                        info!(
                            fetched = outcome.fetched,
                            inserted = outcome.inserted,
                            deduped = outcome.deduped,
                            "S11: cycle ok (leader)"
                        );
                        cursor = window_to;
                    }
                    Err(e) => {
                        metrics.inc_cycle(CycleOutcome::Err);
                        warn!(err = %e, "S11: poll cycle failed; retaining last-success cursor");
                    }
                }
            }
        }
    }

    guard.shutdown().await;
    Ok(())
}

/// Round-2 #11: minimal HTTP /metrics endpoint.
async fn serve_metrics(addr: String, metrics: UsagePollerMetrics) -> anyhow::Result<()> {
    use http_body_util::Full;
    use hyper::body::Bytes;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper::{Request, Response};
    use hyper_util::rt::TokioIo;
    use std::convert::Infallible;
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(&addr).await?;
    info!(addr = %addr, "usage-poller metrics listening");

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
                            .header("content-type", "text/plain; version=0.0.4; charset=utf-8")
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
