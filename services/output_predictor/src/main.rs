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
    config::Config,
    context_window::ContextWindowTable,
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

    // ── Build service ─────────────────────────────────────────────
    let svc = OutputPredictorSvc::new(
        cache,
        context_window,
        cfg.unknown_model_context_window,
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
    use spendguard_output_predictor::server::UNKNOWN_CONTEXT_WINDOW_TOTAL;
    // R2 M12: wire UNKNOWN_CONTEXT_WINDOW_TOTAL into the scrape body.
    // Phase D wires the cache hit-rate counter — SLICE_06 ships the
    // scaffold + the names Phase F's Helm scrape config expects.
    format!(
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
         spendguard_output_predictor_unknown_context_window_total {}\n",
        UNKNOWN_CONTEXT_WINDOW_TOTAL.load(Ordering::Relaxed),
    )
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
    }
}
