//! SpendGuard stats_aggregator daemon entry point.
//!
//! Spec ref stats-aggregator-spec-v1alpha1.md §2.
//!
//! SLICE_06 boot sequence:
//!
//!   1. Install rustls aws_lc_rs crypto provider.
//!   2. Load env config via [`spendguard_stats_aggregator::config::Config`].
//!   3. Connect to canonical_ingest DB (read source + cache write target).
//!   4. Connect to canonical_ingest gRPC for signed CloudEvent emission
//!      (LoggingDriftAlertSink fallback when URL is empty — demo only;
//!      production Helm gate enforces).
//!   5. Spawn metrics hyper server (/metrics + /healthz + /readyz).
//!   6. Run the scheduler loop (hourly per spec §8.1) until shutdown.
//!
//! Container security: USER 65532 + readOnlyRootFilesystem + cap_drop ALL
//! enforced by the Helm chart (Phase F). The daemon itself has no
//! privileged needs (no UDS bind, no socket bind besides the metrics TCP).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, Identity};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use spendguard_signing::Signer;
use spendguard_stats_aggregator::{
    aggregation::STATS_AGGREGATOR_ADVISORY_LOCK_ID,
    config::Config,
    drift_detector::{
        CanonicalIngestDriftAlertSink, DriftAlertSink, DriftDetectorConfig, LoggingDriftAlertSink,
    },
    scheduler::run_loop,
};

#[tokio::main]
async fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("failed to install rustls aws_lc_rs default provider"))?;

    init_tracing();

    let cfg = Config::from_env().context("loading stats_aggregator config")?;
    info!(
        cycle_seconds = cfg.cycle_seconds,
        min_samples_for_alert = cfg.min_samples_for_alert,
        drift_z_threshold = cfg.drift_z_threshold,
        metrics_addr = %cfg.metrics_addr,
        region = %cfg.region,
        profile = %cfg.profile,
        canonical_ingest_url = %cfg.canonical_ingest_url,
        database_present = !cfg.database_url.is_empty(),
        advisory_lock_id = STATS_AGGREGATOR_ADVISORY_LOCK_ID,
        "starting spendguard-stats-aggregator"
    );

    // ── Validate config gates ─────────────────────────────────────
    if cfg.database_url.is_empty() {
        anyhow::bail!(
            "SPENDGUARD_STATS_AGGREGATOR_DATABASE_URL is required (canonical_ingest read+write source)"
        );
    }
    if cfg.cycle_seconds < 60 {
        warn!(
            cycle_seconds = cfg.cycle_seconds,
            "cycle_seconds < 60 — clamping to 60 to protect Postgres"
        );
    }
    let cycle_seconds = cfg.cycle_seconds.max(60);

    // ── Connect to canonical_ingest DB ────────────────────────────
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&cfg.database_url)
        .await
        .context("connect to canonical_ingest DB")?;
    info!("canonical_ingest DB pool connected");

    // ── Build drift alert sink ────────────────────────────────────
    let sink: Arc<dyn DriftAlertSink> = if cfg.canonical_ingest_url.is_empty() {
        warn!(
            "SPENDGUARD_STATS_AGGREGATOR_CANONICAL_INGEST_URL not set — \
             drift_alert events will log to stdout (demo mode). \
             Production Helm profile rejects this fallback."
        );
        Arc::new(LoggingDriftAlertSink)
    } else {
        let channel = build_canonical_ingest_channel(&cfg)
            .await
            .context("connect canonical_ingest channel")?;
        info!(
            url = %cfg.canonical_ingest_url,
            mtls = cfg.sink_tls_cert_pem.is_some(),
            "canonical_ingest sink connected"
        );
        Arc::new(CanonicalIngestDriftAlertSink::new(channel))
    };

    // ── Signer (audit-routed CloudEvent producer identity) ────────
    let producer_identity = if cfg.event_source_override.is_empty() {
        format!("stats-aggregator:{}", cfg.region)
    } else {
        cfg.event_source_override.clone()
    };
    std::env::set_var("SPENDGUARD_STATS_AGGREGATOR_SIGNING_PRODUCER_IDENTITY", &producer_identity);
    let signer: Arc<dyn Signer> = Arc::from(
        spendguard_signing::signer_from_env("SPENDGUARD_STATS_AGGREGATOR")
            .await
            .context("load Ed25519 signer for prediction_drift_alert CloudEvent signing")?,
    );
    info!(
        producer_identity = %signer.producer_identity(),
        key_id = %signer.key_id(),
        "signer ready"
    );

    let detector_cfg = DriftDetectorConfig {
        drift_z_threshold: cfg.drift_z_threshold,
        min_samples_for_alert: cfg.min_samples_for_alert,
    };

    // ── Spawn metrics server ──────────────────────────────────────
    if !cfg.metrics_addr.is_empty() {
        let addr: SocketAddr = cfg
            .metrics_addr
            .parse()
            .with_context(|| format!("invalid metrics_addr `{}`", cfg.metrics_addr))?;
        tokio::spawn(async move {
            if let Err(e) = run_metrics_server(addr).await {
                error!(?e, "metrics server exited with error");
            }
        });
        info!(addr = %cfg.metrics_addr, "metrics endpoint bound");
    }

    // ── Run scheduler loop (forever, until ctrl-c) ────────────────
    let pool_for_loop = pool.clone();
    let scheduler_handle = tokio::spawn(async move {
        run_loop(pool_for_loop, cycle_seconds, detector_cfg, signer, sink).await;
    });

    shutdown_signal().await;
    scheduler_handle.abort();
    info!("spendguard-stats-aggregator shut down cleanly");
    Ok(())
}

/// Build the canonical_ingest gRPC channel with optional mTLS per spec
/// §7.2 + tokenizer SLICE_05 R2 B4 convention.
async fn build_canonical_ingest_channel(cfg: &Config) -> Result<tonic::transport::Channel> {
    let mut ep = Endpoint::from_shared(cfg.canonical_ingest_url.clone())
        .map_err(|e| anyhow::anyhow!("invalid canonical_ingest_url: {e}"))?
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .keep_alive_timeout(Duration::from_secs(20))
        .keep_alive_while_idle(true);

    match (
        cfg.sink_tls_cert_pem.as_deref(),
        cfg.sink_tls_key_pem.as_deref(),
        cfg.sink_tls_ca_pem.as_deref(),
    ) {
        (None, None, None) => {
            warn!(
                "canonical_ingest sink connecting WITHOUT mTLS — demo only; \
                 production Helm profile rejects this fallback."
            );
        }
        (Some(cert), Some(key), Some(ca)) => {
            let cert_pem = std::fs::read(cert).with_context(|| format!("read sink cert {cert}"))?;
            let key_pem = std::fs::read(key).with_context(|| format!("read sink key {key}"))?;
            let ca_pem = std::fs::read(ca).with_context(|| format!("read sink ca {ca}"))?;
            let tls = ClientTlsConfig::new()
                .ca_certificate(Certificate::from_pem(ca_pem))
                .identity(Identity::from_pem(cert_pem, key_pem))
                .domain_name(&cfg.sink_tls_sni);
            ep = ep
                .tls_config(tls)
                .map_err(|e| anyhow::anyhow!("apply sink tls config: {e}"))?;
        }
        _ => {
            anyhow::bail!(
                "partial sink mTLS config: must set all of sink_tls_cert_pem / sink_tls_key_pem / sink_tls_ca_pem, or none"
            );
        }
    }
    Ok(ep.connect_lazy())
}

fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("spendguard_stats_aggregator=info,info"));
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(true)
        .json()
        .init();
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    info!("ctrl_c received — initiating graceful shutdown");
}

fn render_metrics() -> String {
    "# HELP spendguard_stats_aggregator_cycles_total \
     Total aggregation cycles attempted.\n\
     # TYPE spendguard_stats_aggregator_cycles_total counter\n\
     spendguard_stats_aggregator_cycles_total 0\n\
     # HELP spendguard_stats_aggregator_skipped_lock_held_total \
     Cycles skipped because the advisory lock was held by another instance.\n\
     # TYPE spendguard_stats_aggregator_skipped_lock_held_total counter\n\
     spendguard_stats_aggregator_skipped_lock_held_total 0\n\
     # HELP spendguard_stats_aggregator_drift_alerts_total \
     Total prediction_drift_alert CloudEvents emitted.\n\
     # TYPE spendguard_stats_aggregator_drift_alerts_total counter\n\
     spendguard_stats_aggregator_drift_alerts_total 0\n"
        .to_string()
}

/// Minimal /metrics + /healthz + /readyz hyper server.
async fn run_metrics_server(addr: SocketAddr) -> Result<()> {
    use http_body_util::Full;
    use hyper::body::Bytes;
    use hyper::service::service_fn;
    use hyper::{Method, Request, Response, StatusCode};
    use hyper_util::rt::TokioIo;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        tokio::spawn(async move {
            let svc = service_fn(|req: Request<hyper::body::Incoming>| async move {
                let (status, content_type, body): (StatusCode, &str, String) =
                    match (req.method(), req.uri().path()) {
                        (&Method::GET, "/metrics") => (
                            StatusCode::OK,
                            "text/plain; version=0.0.4; charset=utf-8",
                            render_metrics(),
                        ),
                        (&Method::GET, "/healthz") => (
                            StatusCode::OK,
                            "text/plain; charset=utf-8",
                            "ok".to_string(),
                        ),
                        (&Method::GET, "/readyz") => (
                            StatusCode::OK,
                            "text/plain; charset=utf-8",
                            "ready".to_string(),
                        ),
                        _ => (
                            StatusCode::NOT_FOUND,
                            "text/plain; charset=utf-8",
                            "not found".to_string(),
                        ),
                    };
                Ok::<_, std::convert::Infallible>(
                    Response::builder()
                        .status(status)
                        .header("content-type", content_type)
                        .body(Full::new(Bytes::from(body)))
                        .unwrap(),
                )
            });
            if let Err(err) = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, svc)
                .await
            {
                error!(?err, "metrics conn error");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_metrics_includes_known_names() {
        let body = render_metrics();
        assert!(body.contains("spendguard_stats_aggregator_cycles_total"));
        assert!(body.contains("spendguard_stats_aggregator_skipped_lock_held_total"));
        assert!(body.contains("spendguard_stats_aggregator_drift_alerts_total"));
    }
}
