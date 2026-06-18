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
        CanonicalIngestDriftAlertSink, DriftAlertCooldown, DriftAlertSink, DriftDetectorConfig,
        LoggingDriftAlertSink, PostgresDriftAlertCooldownStore,
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

    // ── Validate drift thresholds (boot-time fail-closed gate) ────────
    //
    // These govern the drift-detection safety signal, not a tunable like
    // cycle_seconds, so an out-of-range value is an operator
    // misconfiguration we bail on rather than clamp.
    //
    //   * drift_z_threshold: should_emit_drift_alert treats a non-finite
    //     or <= 0 threshold as "suppress all", silently disabling ALL
    //     drift detection with only a per-bucket log. Require finite > 0.
    //   * min_samples_for_alert: a negative floor makes every bucket pass
    //     `n7d >= min_samples_for_alert`, flooding the immutable audit
    //     chain with alerts on tiny/noisy samples. Require >= 1.
    if !(cfg.drift_z_threshold.is_finite() && cfg.drift_z_threshold > 0.0) {
        anyhow::bail!(
            "SPENDGUARD_STATS_AGGREGATOR_DRIFT_Z_THRESHOLD must be a finite value > 0.0 \
             (got {}); a non-positive or non-finite threshold silently disables ALL drift \
             detection",
            cfg.drift_z_threshold
        );
    }
    if cfg.min_samples_for_alert < 1 {
        anyhow::bail!(
            "SPENDGUARD_STATS_AGGREGATOR_MIN_SAMPLES_FOR_ALERT must be >= 1 (got {}); \
             a value < 1 floods the audit chain with drift alerts on tiny samples",
            cfg.min_samples_for_alert
        );
    }

    // ── Connect to canonical_ingest DB ────────────────────────────
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&cfg.database_url)
        .await
        .context("connect to canonical_ingest DB")?;
    info!("canonical_ingest DB pool connected");

    // ── Signer (audit-routed CloudEvent producer identity) ────────
    //
    // R2 M10: do NOT std::env::set_var() — that mutation is racy on
    // multi-threaded Rust (Rust 1.80+ marks set_var unsafe in code
    // running concurrently with other env reads). The signer crate's
    // signer_from_env reads `<prefix>_SIGNING_PRODUCER_IDENTITY`
    // directly; Helm sets this env var at pod startup (see
    // charts/spendguard/templates/stats_aggregator.yaml). No runtime
    // mutation required.
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

    // ── Build drift alert sink ────────────────────────────────────
    //
    // R2 B5: production sink construction requires schema_bundle_id +
    // schema_bundle_hash_hex from config so the AppendEventsRequest
    // envelope carries the required fields. Demo profile may use the
    // LoggingDriftAlertSink which skips the envelope entirely.
    let sink: Arc<dyn DriftAlertSink> = if cfg.canonical_ingest_url.is_empty() {
        warn!(
            "SPENDGUARD_STATS_AGGREGATOR_CANONICAL_INGEST_URL not set — \
             drift_alert events will log to stdout (demo mode). \
             Production Helm profile rejects this fallback."
        );
        Arc::new(LoggingDriftAlertSink)
    } else {
        if cfg.schema_bundle_id.is_empty() {
            anyhow::bail!(
                "SPENDGUARD_STATS_AGGREGATOR_SCHEMA_BUNDLE_ID required when \
                 SPENDGUARD_STATS_AGGREGATOR_CANONICAL_INGEST_URL is set \
                 (R2 B5: canonical_ingest rejects AppendEventsRequest \
                 without schema_bundle)"
            );
        }
        if cfg.schema_bundle_hash_hex.is_empty() {
            anyhow::bail!(
                "SPENDGUARD_STATS_AGGREGATOR_SCHEMA_BUNDLE_HASH_HEX required \
                 when SPENDGUARD_STATS_AGGREGATOR_CANONICAL_INGEST_URL is set"
            );
        }
        let bundle_hash = hex::decode(&cfg.schema_bundle_hash_hex)
            .context("SPENDGUARD_STATS_AGGREGATOR_SCHEMA_BUNDLE_HASH_HEX must be hex-encoded")?;
        let schema_bundle_ref = spendguard_stats_aggregator::proto::common::v1::SchemaBundleRef {
            schema_bundle_id: cfg.schema_bundle_id.clone(),
            schema_bundle_hash: bundle_hash.into(),
            canonical_schema_version: cfg.canonical_schema_version.clone(),
        };
        let channel = build_canonical_ingest_channel(&cfg)
            .await
            .context("connect canonical_ingest channel")?;
        info!(
            url = %cfg.canonical_ingest_url,
            mtls = cfg.sink_tls_cert_pem.is_some(),
            schema_bundle_id = %cfg.schema_bundle_id,
            "canonical_ingest sink connected"
        );
        Arc::new(CanonicalIngestDriftAlertSink::new(
            channel,
            signer.producer_identity().to_string(),
            schema_bundle_ref,
            signer.key_id().to_string(),
        ))
    };

    let detector_cfg = DriftDetectorConfig {
        drift_z_threshold: cfg.drift_z_threshold,
        min_samples_for_alert: cfg.min_samples_for_alert,
    };
    let cooldown: Arc<dyn DriftAlertCooldown> =
        Arc::new(PostgresDriftAlertCooldownStore::new(pool.clone()));

    // ── Spawn metrics server ──────────────────────────────────────
    //
    // R2 M8: pass the DB pool + cycle-staleness threshold so /healthz +
    // /readyz can probe real subsystem state. /livez stays as pure
    // process liveness ("ok"). /readyz fails if no cycle has run in
    // 2× cycle_seconds (operator alert on stuck scheduler).
    if !cfg.metrics_addr.is_empty() {
        let addr: SocketAddr = cfg
            .metrics_addr
            .parse()
            .with_context(|| format!("invalid metrics_addr `{}`", cfg.metrics_addr))?;
        let pool_for_health = pool.clone();
        let max_cycle_age_secs = cycle_seconds.saturating_mul(2);
        tokio::spawn(async move {
            if let Err(e) = run_metrics_server(addr, pool_for_health, max_cycle_age_secs).await {
                error!(?e, "metrics server exited with error");
            }
        });
        info!(
            addr = %cfg.metrics_addr,
            max_cycle_age_secs,
            "metrics endpoint bound"
        );
    }

    // ── Run scheduler loop (forever, until ctrl-c) ────────────────
    let pool_for_loop = pool.clone();
    let scheduler_handle = tokio::spawn(async move {
        run_loop(
            pool_for_loop,
            cycle_seconds,
            detector_cfg,
            signer,
            sink,
            cooldown,
        )
        .await;
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
    use spendguard_stats_aggregator::scheduler::{
        CYCLES_TOTAL, CYCLE_ERROR_TOTAL, DRIFT_ALERTS_SUPPRESSED_TOTAL, DRIFT_ALERTS_TOTAL,
        LAST_CYCLE_START_UNIX_SECS, SKIPPED_LOCK_HELD_TOTAL,
    };
    use std::sync::atomic::Ordering;
    // R2 M13: render the live AtomicU64 counters.
    format!(
        "# HELP spendguard_stats_aggregator_cycles_total \
         Total aggregation cycles attempted.\n\
         # TYPE spendguard_stats_aggregator_cycles_total counter\n\
         spendguard_stats_aggregator_cycles_total {}\n\
         # HELP spendguard_stats_aggregator_skipped_lock_held_total \
         Cycles skipped because the advisory lock was held by another instance.\n\
         # TYPE spendguard_stats_aggregator_skipped_lock_held_total counter\n\
         spendguard_stats_aggregator_skipped_lock_held_total {}\n\
         # HELP spendguard_stats_aggregator_drift_alerts_total \
         Total prediction_drift_alert CloudEvents emitted.\n\
         # TYPE spendguard_stats_aggregator_drift_alerts_total counter\n\
         spendguard_stats_aggregator_drift_alerts_total {}\n\
         # HELP spendguard_stats_aggregator_drift_alerts_suppressed_total \
         Total prediction_drift_alert CloudEvents suppressed by cooldown or numeric safety guards.\n\
         # TYPE spendguard_stats_aggregator_drift_alerts_suppressed_total counter\n\
         spendguard_stats_aggregator_drift_alerts_suppressed_total {}\n\
         # HELP spendguard_stats_aggregator_cycle_error_total \
         Total per-cycle errors (tenant aggregation failures + sink failures).\n\
         # TYPE spendguard_stats_aggregator_cycle_error_total counter\n\
         spendguard_stats_aggregator_cycle_error_total {}\n\
         # HELP spendguard_stats_aggregator_last_cycle_start_unix_secs \
         Unix timestamp (seconds) of the last cycle attempt; 0 if no cycle yet.\n\
         # TYPE spendguard_stats_aggregator_last_cycle_start_unix_secs gauge\n\
         spendguard_stats_aggregator_last_cycle_start_unix_secs {}\n",
        CYCLES_TOTAL.load(Ordering::Relaxed),
        SKIPPED_LOCK_HELD_TOTAL.load(Ordering::Relaxed),
        DRIFT_ALERTS_TOTAL.load(Ordering::Relaxed),
        DRIFT_ALERTS_SUPPRESSED_TOTAL.load(Ordering::Relaxed),
        CYCLE_ERROR_TOTAL.load(Ordering::Relaxed),
        LAST_CYCLE_START_UNIX_SECS.load(Ordering::Relaxed),
    )
}

/// Minimal /metrics + /livez + /healthz + /readyz hyper server.
///
/// R2 M8 (Security F8): real subsystem probes.
///   * /livez — pure process liveness, always 200 OK
///   * /healthz — DB pool ping (unhealthy if pool can't acquire)
///   * /readyz — DB pool ping AND cycle freshness (unready if last
///     cycle attempt > max_cycle_age_secs ago)
async fn run_metrics_server(
    addr: SocketAddr,
    pool: sqlx::PgPool,
    max_cycle_age_secs: u64,
) -> Result<()> {
    use http_body_util::Full;
    use hyper::body::Bytes;
    use hyper::service::service_fn;
    use hyper::{Method, Request, Response, StatusCode};
    use hyper_util::rt::TokioIo;
    use spendguard_stats_aggregator::scheduler::LAST_CYCLE_START_UNIX_SECS;
    use std::sync::atomic::Ordering;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let pool_clone = pool.clone();
        tokio::spawn(async move {
            let svc = service_fn(move |req: Request<hyper::body::Incoming>| {
                let pool = pool_clone.clone();
                async move {
                    let (status, content_type, body): (StatusCode, &str, String) =
                        match (req.method(), req.uri().path()) {
                            (&Method::GET, "/metrics") => (
                                StatusCode::OK,
                                "text/plain; version=0.0.4; charset=utf-8",
                                render_metrics(),
                            ),
                            (&Method::GET, "/livez") => (
                                StatusCode::OK,
                                "text/plain; charset=utf-8",
                                "ok".to_string(),
                            ),
                            (&Method::GET, "/healthz") => {
                                // R2 M8: DB pool ping. SELECT 1 covers
                                // both connectivity + a trivial round-trip.
                                match sqlx::query("SELECT 1").execute(&pool).await {
                                    Ok(_) => (
                                        StatusCode::OK,
                                        "text/plain; charset=utf-8",
                                        "ok".to_string(),
                                    ),
                                    Err(e) => (
                                        StatusCode::SERVICE_UNAVAILABLE,
                                        "text/plain; charset=utf-8",
                                        format!("db ping failed: {e}"),
                                    ),
                                }
                            }
                            (&Method::GET, "/readyz") => {
                                // R2 M8: DB ping + cycle freshness.
                                let db_ok = sqlx::query("SELECT 1").execute(&pool).await.is_ok();
                                let now_secs = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_secs())
                                    .unwrap_or(0);
                                let last_cycle = LAST_CYCLE_START_UNIX_SECS.load(Ordering::Relaxed);
                                let cycle_fresh = last_cycle == 0
                                    || now_secs.saturating_sub(last_cycle) <= max_cycle_age_secs;
                                if db_ok && cycle_fresh {
                                    (
                                        StatusCode::OK,
                                        "text/plain; charset=utf-8",
                                        "ready".to_string(),
                                    )
                                } else {
                                    (
                                        StatusCode::SERVICE_UNAVAILABLE,
                                        "text/plain; charset=utf-8",
                                        format!(
                                            "not ready (db_ok={db_ok}, cycle_fresh={cycle_fresh}, \
                                             last_cycle={last_cycle}, now={now_secs}, \
                                             max_cycle_age_secs={max_cycle_age_secs})"
                                        ),
                                    )
                                }
                            }
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
                }
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
        assert!(body.contains("spendguard_stats_aggregator_drift_alerts_suppressed_total"));
    }
}
