//! SpendGuard output_predictor gRPC service entry point.
//!
//! Spec ref output-predictor-service-spec-v1alpha1.md §2.
//!
//! SLICE_06 boot sequence (mirrors tokenizer SLICE_03/SLICE_05 pattern):
//!
//!   1. Install rustls aws_lc_rs crypto provider.
//!   2. Load env config via [`spendguard_output_predictor::config::Config`].
//!   3. Load model_context_window.toml (Phase C populates the file;
//!      missing file is non-fatal — empty table → unknown_model default).
//!   4. Construct the OutputDistributionCache (read-only sqlx pool to
//!      canonical_ingest DB when DATABASE_URL set; None for skeleton/demo).
//!   5. Spawn the /metrics + /healthz + /readyz hyper server.
//!   6. Bind the tonic gRPC server (UDS or TCP+mTLS or TCP-plaintext-demo).
//!   7. Block on graceful shutdown signal.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use spendguard_output_predictor::{
    cache::OutputDistributionCache,
    circuit_breaker::{CircuitBreakerConfig, PluginCircuitBreaker},
    config::Config,
    context_window::ContextWindowTable,
    endpoint_cache::EndpointCache,
    plugin_client::{PluginClient, PluginClientTls},
    proto::output_predictor::v1::output_predictor_server::OutputPredictorServer,
    server::OutputPredictorSvc,
};

#[tokio::main]
async fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("failed to install rustls aws_lc_rs default provider"))?;

    init_tracing();

    let cfg = Config::from_env().context("loading output_predictor config")?;
    info!(
        listen = %cfg.listen_addr,
        uds = ?cfg.uds_path,
        mtls = cfg.tls_cert_pem.is_some(),
        metrics = %cfg.metrics_addr,
        region = %cfg.region,
        profile = %cfg.profile,
        database_present = !cfg.database_url.is_empty(),
        cache_ttl_seconds = %cfg.cache_ttl_seconds,
        "starting spendguard-output-predictor"
    );

    // ── Load model_context_window.toml ────────────────────────────
    let context_window = Arc::new(ContextWindowTable::load_from_path(
        &cfg.context_window_toml_path,
    ));
    info!(
        path = %cfg.context_window_toml_path,
        "model_context_window.toml loaded"
    );

    // ── Construct the output_distribution_cache ───────────────────
    // When DATABASE_URL is set we open a read-only pool; otherwise the
    // cache runs in skeleton mode and every Predict falls to L1
    // cold-start. Production Helm gate (Phase F) rejects the empty
    // database_url under chart.profile=production.
    let pool = if cfg.database_url.is_empty() {
        warn!(
            "DATABASE_URL not set — output_predictor running in skeleton mode \
             (Strategy B will always fall to L1 cold-start; demo only). \
             Production Helm profile rejects this fallback."
        );
        None
    } else {
        let p = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(5))
            .connect(&cfg.database_url)
            .await
            .context("connect to canonical_ingest DB for output_distribution_cache lookup")?;
        info!("output_distribution_cache pool connected");
        Some(p)
    };
    let cache = OutputDistributionCache::new(
        pool.clone(),
        Duration::from_secs(cfg.cache_ttl_seconds),
    );

    // ── SLICE_07 Phase D: Strategy C wiring ───────────────────────
    //
    // Per output-predictor-plugin-contract-v1alpha1.md §8: plugin
    // endpoint registry lives in the control_plane DB. The endpoint
    // cache reads from a SEPARATE pool from the output_distribution_cache
    // (which lives in canonical_ingest DB) to avoid accidental
    // cross-DB DSN reuse.
    //
    // Empty plugin_endpoint_database_url = skeleton mode: every
    // tenant lookup returns NotConfigured so strategy_c falls to
    // B silently. Production Helm gate (Phase D below) rejects
    // the empty case when chart.profile=production.
    let plugin_endpoint_pool = if cfg.plugin_endpoint_database_url.is_empty() {
        warn!(
            "SPENDGUARD_OUTPUT_PREDICTOR_PLUGIN_ENDPOINT_DATABASE_URL not set — \
             Strategy C running in skeleton mode (every tenant lookup returns \
             NotConfigured; Predict falls to Strategy B). Production Helm profile \
             rejects this fallback when chart.profile=production AND at least \
             one tenant has registered a plugin endpoint."
        );
        None
    } else {
        let p = sqlx::postgres::PgPoolOptions::new()
            .max_connections(3)
            .acquire_timeout(Duration::from_secs(5))
            .connect(&cfg.plugin_endpoint_database_url)
            .await
            .context("connect to control_plane DB for predictor_plugin_endpoints lookup")?;
        info!("predictor_plugin_endpoints pool connected");
        Some(p)
    };
    let endpoint_cache = EndpointCache::new(
        plugin_endpoint_pool,
        Duration::from_secs(cfg.plugin_endpoint_cache_ttl_seconds),
    );

    // Plugin client TLS — all-or-none per the same pattern the
    // server-side mTLS uses (build_server_tls_config). Partial
    // config is a hard boot failure to fail-closed against
    // accidental production plaintext.
    let plugin_client_tls = build_plugin_client_tls_config(&cfg)
        .context("loading plugin client mTLS config")?;
    // R2 B1: PluginClient::new now eagerly reads + parses cert/key/CA
    // PEMs (so a typo in the path / bad PEM byte ordering surfaces at
    // boot rather than on the first Predict call). `?` here propagates
    // the failure up to `main()` which exits non-zero — fail-closed.
    let plugin_client = PluginClient::new(plugin_client_tls)
        .context("initialise plugin client (mTLS materials)")?;

    let plugin_breaker = PluginCircuitBreaker::new(CircuitBreakerConfig::default());

    // ── Build service ─────────────────────────────────────────────
    let svc = OutputPredictorSvc::new(
        cache,
        context_window,
        cfg.unknown_model_context_window,
        endpoint_cache,
        plugin_client,
        plugin_breaker,
    );
    let tonic_svc = OutputPredictorServer::new(svc)
        // Match the tokenizer's DoS posture: 1 MiB decoded message
        // cap. PredictRequest is ~200 bytes typical; 1 MiB is generous
        // headroom for unusual classifier overrides.
        .max_decoding_message_size(1 << 20);

    // ── Spawn metrics server (best-effort) ────────────────────────
    //
    // R2 M8: pass the DB pool (Option clone) so /healthz + /readyz can
    // probe real connectivity. Skeleton-mode (no pool) returns 200 OK
    // on /healthz + /readyz because the service is intentionally
    // running without DB — production gates that off in Helm.
    if !cfg.metrics_addr.is_empty() {
        let metrics_addr: SocketAddr = cfg
            .metrics_addr
            .parse()
            .with_context(|| format!("invalid metrics_addr `{}`", cfg.metrics_addr))?;
        let pool_for_health = pool.clone();
        tokio::spawn(async move {
            if let Err(e) = run_metrics_server(metrics_addr, pool_for_health).await {
                error!(?e, "metrics server exited with error");
            }
        });
        info!(addr = %cfg.metrics_addr, "metrics endpoint bound");
    }

    // ── Bind gRPC ──────────────────────────────────────────────────
    if let Some(uds_path) = cfg.uds_path.as_deref() {
        bind_uds(uds_path, tonic_svc).await?;
    } else {
        bind_tcp(&cfg, tonic_svc).await?;
    }

    info!("spendguard-output-predictor shut down cleanly");
    Ok(())
}

/// Symlink-safe stale UDS socket cleanup (SLICE_03 R3 N2 convention).
/// `Path::exists` follows symlinks; `symlink_metadata` returns metadata
/// for the link itself. Refuse to remove anything that is not a regular
/// socket file → blocks the symlink TOCTOU attack vector.
async fn cleanup_stale_uds(path: &Path) -> Result<()> {
    use std::os::unix::fs::FileTypeExt;
    match tokio::fs::symlink_metadata(path).await {
        Ok(m) if m.file_type().is_socket() => {
            tokio::fs::remove_file(path)
                .await
                .with_context(|| format!("remove stale uds socket `{}`", path.display()))?;
            info!(uds_path = %path.display(), "removed stale uds socket");
            Ok(())
        }
        Ok(m) if m.file_type().is_symlink() => {
            anyhow::bail!(
                "uds path `{}` is a symlink; refusing to follow (symlink attack defense per SLICE_03 R3 N2)",
                path.display()
            );
        }
        Ok(_) => {
            anyhow::bail!(
                "uds path `{}` exists and is not a socket; refusing to overwrite",
                path.display()
            );
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

async fn bind_uds(
    uds_path: &str,
    tonic_svc: OutputPredictorServer<OutputPredictorSvc>,
) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    use tokio::net::UnixListener;

    let path = Path::new(uds_path);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("mkdir uds parent for `{uds_path}`"))?;
    }

    cleanup_stale_uds(path).await?;

    let listener = UnixListener::bind(path)
        .with_context(|| format!("bind uds listener `{uds_path}`"))?;

    // SLICE_03 R3 N1: socket file perms 0660. Default umask leaves the
    // socket world-readable; under hostPath mount this lets any UID on
    // the host speak gRPC. 0660 = rw for owner + group; requires the
    // caller pod to share fsGroup: 65532 (set by Helm pod-level
    // securityContext).
    let perms = std::fs::Permissions::from_mode(0o660);
    std::fs::set_permissions(path, perms)
        .with_context(|| format!("set perms on uds `{}`", path.display()))?;
    info!(uds_path = %path.display(), mode = "0660", "output_predictor UDS perms set");

    let incoming = async_stream::stream! {
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => yield Ok::<_, std::io::Error>(stream),
                Err(e) => yield Err(e),
            }
        }
    };

    info!(uds = %uds_path, "binding output_predictor gRPC server (UDS, no mTLS — kernel-enforced trust)");
    Server::builder()
        .concurrency_limit_per_connection(32)
        .max_concurrent_streams(64)
        .initial_connection_window_size(8 << 20)
        .initial_stream_window_size(2 << 20)
        .add_service(tonic_svc)
        .serve_with_incoming_shutdown(incoming, shutdown_signal())
        .await
        .context("tonic UDS gRPC server failed")
}

async fn bind_tcp(
    cfg: &Config,
    tonic_svc: OutputPredictorServer<OutputPredictorSvc>,
) -> Result<()> {
    let listen_addr: SocketAddr = cfg
        .listen_addr
        .parse()
        .with_context(|| format!("invalid listen_addr `{}`", cfg.listen_addr))?;

    let tls = build_server_tls_config(cfg).context("loading mTLS server config")?;

    info!(addr = %cfg.listen_addr, mtls = tls.is_some(), "binding output_predictor gRPC server (TCP)");

    let mut builder = Server::builder()
        .concurrency_limit_per_connection(32)
        .max_concurrent_streams(64)
        .initial_connection_window_size(8 << 20)
        .initial_stream_window_size(2 << 20);
    if let Some(tls_cfg) = tls {
        builder = builder
            .tls_config(tls_cfg)
            .context("apply server TLS config")?;
    } else {
        warn!(
            "output_predictor server starting WITHOUT mTLS — only acceptable in \
             POC / demo mode. Set SPENDGUARD_OUTPUT_PREDICTOR_TLS_{{CERT,KEY,CA}}_PEM \
             for production-correct mTLS (Helm production profile rejects this)."
        );
    }

    builder
        .add_service(tonic_svc)
        .serve_with_shutdown(listen_addr, shutdown_signal())
        .await
        .context("tonic TCP gRPC server failed")
}

/// SLICE_07 Phase D: build the SpendGuard side of the plugin mTLS
/// (cert + key + customer trust CA). All-or-none like
/// `build_server_tls_config` — partial config is a hard boot failure
/// because plaintext to a customer endpoint violates spec §3.1
/// (mTLS-only auth contract).
///
/// Empty config = `Ok(None)`: skeleton/demo mode where strategy_c.rs
/// still works (the plugin_client logs a warn at boot and tonic
/// channels are plaintext). Production Helm profile rejects this
/// fallback via the chart's required-input gate (Phase D values).
fn build_plugin_client_tls_config(cfg: &Config) -> Result<Option<PluginClientTls>> {
    use std::path::PathBuf;
    match (
        &cfg.plugin_client_cert_pem,
        &cfg.plugin_client_key_pem,
        &cfg.plugin_trust_ca_pem,
    ) {
        (None, None, None) => Ok(None),
        (Some(cert), Some(key), Some(ca)) => Ok(Some(PluginClientTls {
            client_cert_pem: PathBuf::from(cert),
            client_key_pem: PathBuf::from(key),
            trust_ca_pem: PathBuf::from(ca),
        })),
        _ => Err(anyhow::anyhow!(
            "partial plugin client mTLS config: must set all of \
             SPENDGUARD_OUTPUT_PREDICTOR_PLUGIN_CLIENT_CERT_PEM / \
             PLUGIN_CLIENT_KEY_PEM / PLUGIN_TRUST_CA_PEM, or none. Spec §3.1 \
             requires mTLS for customer plugin endpoints; plaintext to a \
             customer service is a security policy violation."
        )),
    }
}

/// Build server TLS config when cert+key+ca paths are all set; partial
/// config is rejected to fail-closed against accidental production
/// plaintext (mirrors tokenizer SLICE_03 R2 B3.2).
fn build_server_tls_config(cfg: &Config) -> Result<Option<ServerTlsConfig>> {
    match (&cfg.tls_cert_pem, &cfg.tls_key_pem, &cfg.tls_ca_pem) {
        (None, None, None) => Ok(None),
        (Some(cert_path), Some(key_path), Some(ca_path)) => {
            let cert = std::fs::read(cert_path)
                .with_context(|| format!("read tls cert {cert_path}"))?;
            let key = std::fs::read(key_path)
                .with_context(|| format!("read tls key {key_path}"))?;
            let ca = std::fs::read(ca_path)
                .with_context(|| format!("read tls ca {ca_path}"))?;
            Ok(Some(
                ServerTlsConfig::new()
                    .identity(Identity::from_pem(cert, key))
                    .client_ca_root(Certificate::from_pem(ca)),
            ))
        }
        _ => Err(anyhow::anyhow!(
            "partial mTLS config: must set all of tls_cert_pem / tls_key_pem / tls_ca_pem, or none"
        )),
    }
}

fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("spendguard_output_predictor=info,info"));
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
    use std::sync::atomic::Ordering;
    use spendguard_output_predictor::server::{
        UNKNOWN_CONTEXT_WINDOW_TOTAL,
        CUSTOMER_PREDICTOR_CALL_SUCCESS_TOTAL,
        CUSTOMER_PREDICTOR_CALL_FALL_TO_B_TOTAL,
        CUSTOMER_PREDICTOR_TENANT_ISOLATION_VIOLATION_TOTAL,
        FAILURE_BY_MODE_TIMEOUT, FAILURE_BY_MODE_GRPC_ERROR,
        FAILURE_BY_MODE_INVALID_ZERO_OR_NEGATIVE, FAILURE_BY_MODE_INVALID_OVERFLOW,
        FAILURE_BY_MODE_INVALID_CONFIDENCE, FAILURE_BY_MODE_DESERIALIZATION_ERROR,
        FAILURE_BY_MODE_TLS_ERROR, FAILURE_BY_MODE_NOT_SERVING,
        FAILURE_BY_MODE_NOT_CONFIGURED, FAILURE_BY_MODE_BREAKER_OPEN,
    };
    // SLICE_07 Phase E: surface the spec §9.1 customer_predictor_* counters.
    // The 10 per-mode counters cover the spec §5.1 8 documented failure
    // modes + the 2 SLICE_07 metric-only modes (not_configured + breaker_open).
    let body = format!(
        "# HELP spendguard_output_predictor_predict_total \
         Total Predict RPCs handled.\n\
         # TYPE spendguard_output_predictor_predict_total counter\n\
         spendguard_output_predictor_predict_total 0\n\
         # HELP spendguard_output_predictor_cache_hit_rate \
         Phase-D cache hit rate (count of L4 hits / total predict calls).\n\
         # TYPE spendguard_output_predictor_cache_hit_rate gauge\n\
         spendguard_output_predictor_cache_hit_rate 0\n\
         # HELP spendguard_output_predictor_unknown_context_window_total \
         Predict calls that fell back to the unknown model_context_window default per spec §3.3.\n\
         # TYPE spendguard_output_predictor_unknown_context_window_total counter\n\
         spendguard_output_predictor_unknown_context_window_total {unknown}\n\
         # HELP customer_predictor_call_total \
         Strategy C call outcomes per output-predictor-plugin-contract-v1alpha1.md §9.1. outcome=success | fall_to_b.\n\
         # TYPE customer_predictor_call_total counter\n\
         customer_predictor_call_total{{outcome=\"success\"}} {success}\n\
         customer_predictor_call_total{{outcome=\"fall_to_b\"}} {fall_to_b}\n\
         # HELP customer_predictor_tenant_isolation_violation_total \
         Spec §7.3 tenant binding violation count — RLS bypass suspect; should ALWAYS be zero in production.\n\
         # TYPE customer_predictor_tenant_isolation_violation_total counter\n\
         customer_predictor_tenant_isolation_violation_total {violation}\n\
         # HELP customer_predictor_failure_mode_total \
         Per-mode breakdown of Strategy C fall-to-B outcomes per spec §5.1.\n\
         # TYPE customer_predictor_failure_mode_total counter\n\
         customer_predictor_failure_mode_total{{mode=\"timeout\"}} {m_timeout}\n\
         customer_predictor_failure_mode_total{{mode=\"grpc_error\"}} {m_grpc}\n\
         customer_predictor_failure_mode_total{{mode=\"invalid_zero_or_negative\"}} {m_zero}\n\
         customer_predictor_failure_mode_total{{mode=\"invalid_overflow\"}} {m_overflow}\n\
         customer_predictor_failure_mode_total{{mode=\"invalid_confidence\"}} {m_conf}\n\
         customer_predictor_failure_mode_total{{mode=\"deserialization_error\"}} {m_deser}\n\
         customer_predictor_failure_mode_total{{mode=\"tls_error\"}} {m_tls}\n\
         customer_predictor_failure_mode_total{{mode=\"not_serving\"}} {m_ns}\n\
         customer_predictor_failure_mode_total{{mode=\"not_configured\"}} {m_nc}\n\
         customer_predictor_failure_mode_total{{mode=\"breaker_open\"}} {m_brk}\n",
        unknown = UNKNOWN_CONTEXT_WINDOW_TOTAL.load(Ordering::Relaxed),
        success = CUSTOMER_PREDICTOR_CALL_SUCCESS_TOTAL.load(Ordering::Relaxed),
        fall_to_b = CUSTOMER_PREDICTOR_CALL_FALL_TO_B_TOTAL.load(Ordering::Relaxed),
        violation = CUSTOMER_PREDICTOR_TENANT_ISOLATION_VIOLATION_TOTAL.load(Ordering::Relaxed),
        m_timeout = FAILURE_BY_MODE_TIMEOUT.load(Ordering::Relaxed),
        m_grpc = FAILURE_BY_MODE_GRPC_ERROR.load(Ordering::Relaxed),
        m_zero = FAILURE_BY_MODE_INVALID_ZERO_OR_NEGATIVE.load(Ordering::Relaxed),
        m_overflow = FAILURE_BY_MODE_INVALID_OVERFLOW.load(Ordering::Relaxed),
        m_conf = FAILURE_BY_MODE_INVALID_CONFIDENCE.load(Ordering::Relaxed),
        m_deser = FAILURE_BY_MODE_DESERIALIZATION_ERROR.load(Ordering::Relaxed),
        m_tls = FAILURE_BY_MODE_TLS_ERROR.load(Ordering::Relaxed),
        m_ns = FAILURE_BY_MODE_NOT_SERVING.load(Ordering::Relaxed),
        m_nc = FAILURE_BY_MODE_NOT_CONFIGURED.load(Ordering::Relaxed),
        m_brk = FAILURE_BY_MODE_BREAKER_OPEN.load(Ordering::Relaxed),
    );
    body
}

/// Minimal /metrics + /livez + /healthz + /readyz hyper server.
///
/// R2 M8 (Security F8): real subsystem probes.
///   * /livez — pure process liveness, always 200 OK
///   * /healthz — DB pool ping when configured (skeleton mode: OK)
///   * /readyz — same as /healthz currently; future per-route gates
///
/// Mirrors the raw-hyper pattern used by services/canonical_ingest and
/// services/ledger so the chart can scrape Prometheus without adding a
/// `prometheus` crate dependency.
async fn run_metrics_server(
    addr: SocketAddr,
    pool: Option<sqlx::PgPool>,
) -> Result<()> {
    use http_body_util::Full;
    use hyper::body::Bytes;
    use hyper::service::service_fn;
    use hyper::{Method, Request, Response, StatusCode};
    use hyper_util::rt::TokioIo;

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
                            (&Method::GET, "/healthz") => match pool {
                                Some(ref p) => {
                                    match sqlx::query("SELECT 1").execute(p).await {
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
                                None => (
                                    // Skeleton mode — no DB to ping; healthz
                                    // is about the *process* health, return OK.
                                    StatusCode::OK,
                                    "text/plain; charset=utf-8",
                                    "ok (skeleton mode)".to_string(),
                                ),
                            },
                            (&Method::GET, "/readyz") => match pool {
                                Some(ref p) => {
                                    match sqlx::query("SELECT 1").execute(p).await {
                                        Ok(_) => (
                                            StatusCode::OK,
                                            "text/plain; charset=utf-8",
                                            "ready".to_string(),
                                        ),
                                        Err(e) => (
                                            StatusCode::SERVICE_UNAVAILABLE,
                                            "text/plain; charset=utf-8",
                                            format!("db not ready: {e}"),
                                        ),
                                    }
                                }
                                None => (
                                    StatusCode::OK,
                                    "text/plain; charset=utf-8",
                                    "ready (skeleton mode)".to_string(),
                                ),
                            },
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
    use tempfile::TempDir;
    use tokio::net::UnixListener;

    #[tokio::test]
    async fn cleanup_uds_notfound_is_noop() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("does_not_exist.sock");
        cleanup_stale_uds(&path).await.expect("noop");
        assert!(!path.exists(), "noop should not create");
    }

    #[tokio::test]
    async fn cleanup_uds_real_socket_is_unlinked() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("real.sock");
        let _listener = UnixListener::bind(&path).expect("bind");
        drop(_listener);
        assert!(path.exists());
        cleanup_stale_uds(&path).await.expect("unlink stale socket");
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn cleanup_uds_rejects_symlink_attack() {
        use std::os::unix::fs::symlink;
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("sensitive.txt");
        std::fs::write(&target, b"do-not-delete").expect("write");
        let link_path = dir.path().join("attack.sock");
        symlink(&target, &link_path).expect("symlink");

        let err = cleanup_stale_uds(&link_path)
            .await
            .expect_err("symlink path must be rejected");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("symlink attack defense"),
            "error must mention attack defense, got: {msg}"
        );
        assert!(target.exists(), "symlink target must not be unlinked");
        assert_eq!(std::fs::read(&target).expect("read"), b"do-not-delete");
        assert!(link_path.is_symlink());
    }

    #[tokio::test]
    async fn cleanup_uds_rejects_regular_file() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("not_a_socket.txt");
        std::fs::write(&path, b"i am a regular file").expect("write");
        let err = cleanup_stale_uds(&path)
            .await
            .expect_err("regular file must be rejected");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("refusing to overwrite"),
            "error must mention refusing to overwrite, got: {msg}"
        );
        assert!(path.exists());
    }

    #[test]
    fn build_plugin_client_tls_config_all_or_none() {
        let mut cfg = Config {
            listen_addr: "127.0.0.1:0".into(),
            uds_path: None,
            tls_cert_pem: None,
            tls_key_pem: None,
            tls_ca_pem: None,
            metrics_addr: "".into(),
            region: "test".into(),
            profile: "demo".into(),
            database_url: "".into(),
            cache_ttl_seconds: 300,
            unknown_model_context_window: 8000,
            context_window_toml_path: "data/model_context_window.toml".into(),
            plugin_endpoint_database_url: "".into(),
            plugin_endpoint_cache_ttl_seconds: 60,
            plugin_client_cert_pem: None,
            plugin_client_key_pem: None,
            plugin_trust_ca_pem: None,
        };
        // None set → Ok(None): demo / skeleton mode.
        assert!(build_plugin_client_tls_config(&cfg).expect("ok").is_none());
        // Partial → Err (cert only).
        cfg.plugin_client_cert_pem = Some("/tmp/plugin-cert.pem".into());
        let err = build_plugin_client_tls_config(&cfg).expect_err("partial rejected");
        let msg = format!("{err:#}");
        assert!(msg.contains("partial plugin client mTLS config"), "got: {msg}");
        // All three → Ok(Some).
        cfg.plugin_client_key_pem = Some("/tmp/plugin-key.pem".into());
        cfg.plugin_trust_ca_pem = Some("/tmp/plugin-ca.pem".into());
        let tls = build_plugin_client_tls_config(&cfg).expect("ok");
        let tls = tls.expect("should be Some");
        assert_eq!(tls.client_cert_pem.to_string_lossy(), "/tmp/plugin-cert.pem");
        assert_eq!(tls.client_key_pem.to_string_lossy(), "/tmp/plugin-key.pem");
        assert_eq!(tls.trust_ca_pem.to_string_lossy(), "/tmp/plugin-ca.pem");
    }

    #[test]
    fn build_server_tls_config_all_or_none() {
        let mut cfg = Config {
            listen_addr: "127.0.0.1:0".into(),
            uds_path: None,
            tls_cert_pem: None,
            tls_key_pem: None,
            tls_ca_pem: None,
            metrics_addr: "".into(),
            region: "test".into(),
            profile: "demo".into(),
            database_url: "".into(),
            cache_ttl_seconds: 300,
            unknown_model_context_window: 8000,
            context_window_toml_path: "data/model_context_window.toml".into(),
            plugin_endpoint_database_url: "".into(),
            plugin_endpoint_cache_ttl_seconds: 60,
            plugin_client_cert_pem: None,
            plugin_client_key_pem: None,
            plugin_trust_ca_pem: None,
        };
        // None set → Ok(None).
        assert!(build_server_tls_config(&cfg).expect("ok").is_none());
        // Partial → Err.
        cfg.tls_cert_pem = Some("/tmp/cert.pem".into());
        let err = build_server_tls_config(&cfg).expect_err("partial rejected");
        let msg = format!("{err:#}");
        assert!(msg.contains("partial mTLS config"), "got: {msg}");
    }

    #[test]
    fn render_metrics_contains_known_names() {
        let body = render_metrics();
        assert!(body.contains("spendguard_output_predictor_predict_total"));
        assert!(body.contains("spendguard_output_predictor_cache_hit_rate"));
        // SLICE_07 Phase E: customer_predictor_* metric surface per spec §9.1.
        assert!(body.contains("customer_predictor_call_total"));
        assert!(body.contains("customer_predictor_tenant_isolation_violation_total"));
        assert!(body.contains("customer_predictor_failure_mode_total"));
        // Each of the 10 failure modes is represented.
        assert!(body.contains("mode=\"timeout\""));
        assert!(body.contains("mode=\"grpc_error\""));
        assert!(body.contains("mode=\"invalid_zero_or_negative\""));
        assert!(body.contains("mode=\"invalid_overflow\""));
        assert!(body.contains("mode=\"invalid_confidence\""));
        assert!(body.contains("mode=\"deserialization_error\""));
        assert!(body.contains("mode=\"tls_error\""));
        assert!(body.contains("mode=\"not_serving\""));
        assert!(body.contains("mode=\"not_configured\""));
        assert!(body.contains("mode=\"breaker_open\""));
    }
}
