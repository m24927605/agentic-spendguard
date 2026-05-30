//! SpendGuard run_cost_projector gRPC service entry point.
//!
//! Spec ref `run-cost-projector-spec-v1alpha1.md` §2.
//!
//! SLICE_09 boot sequence (mirrors output_predictor SLICE_06 pattern):
//!
//!   1. Install rustls aws_lc_rs crypto provider.
//!   2. Load env config via [`spendguard_run_cost_projector::config::Config`].
//!   3. Phase B: open canonical_ingest DB pool (when DATABASE_URL set) for
//!      run_length_distribution_cache + audit_outbox replay.
//!   4. Phase B: construct RunStateCache (bounded LRU + TTL).
//!   5. Spawn the /metrics + /livez + /healthz + /readyz hyper server.
//!   6. Bind the tonic gRPC server (UDS or TCP+mTLS or TCP-plaintext-demo).
//!   7. Block on graceful shutdown signal.

use std::net::SocketAddr;
use std::path::Path;

use anyhow::{Context, Result};
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use spendguard_run_cost_projector::{
    config::Config, proto::run_cost_projector::v1::run_cost_projector_server::RunCostProjectorServer,
    server::RunCostProjectorSvc,
};

#[tokio::main]
async fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("failed to install rustls aws_lc_rs default provider"))?;

    init_tracing();

    let cfg = Config::from_env().context("loading run_cost_projector config")?;
    info!(
        listen = %cfg.listen_addr,
        uds = ?cfg.uds_path,
        mtls = cfg.tls_cert_pem.is_some(),
        metrics = %cfg.metrics_addr,
        region = %cfg.region,
        profile = %cfg.profile,
        database_present = !cfg.database_url.is_empty(),
        state_cache_ttl_seconds = %cfg.state_cache_ttl_seconds,
        state_cache_capacity = %cfg.state_cache_capacity,
        replay_window_minutes = %cfg.replay_window_minutes,
        cold_start_run_length = %cfg.cold_start_run_length,
        "starting spendguard-run-cost-projector"
    );

    // Phase A: Service skeleton — Phases B/C/D wire cache + signals + DB.
    let svc = RunCostProjectorSvc::new();
    let tonic_svc = RunCostProjectorServer::new(svc).max_decoding_message_size(1 << 20);

    // Metrics server (best-effort, mirrors output_predictor pattern).
    if !cfg.metrics_addr.is_empty() {
        let metrics_addr: SocketAddr = cfg
            .metrics_addr
            .parse()
            .with_context(|| format!("invalid metrics_addr `{}`", cfg.metrics_addr))?;
        tokio::spawn(async move {
            if let Err(e) = run_metrics_server(metrics_addr).await {
                error!(?e, "metrics server exited with error");
            }
        });
        info!(addr = %cfg.metrics_addr, "metrics endpoint bound");
    }

    if let Some(uds_path) = cfg.uds_path.as_deref() {
        bind_uds(uds_path, tonic_svc).await?;
    } else {
        bind_tcp(&cfg, tonic_svc).await?;
    }

    info!("spendguard-run-cost-projector shut down cleanly");
    Ok(())
}

/// Symlink-safe stale UDS socket cleanup (SLICE_03 R3 N2 convention).
/// `Path::exists` follows symlinks; `symlink_metadata` returns metadata for
/// the link itself. Refuse to remove anything that is not a regular socket
/// file — blocks the symlink TOCTOU attack vector.
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
    tonic_svc: RunCostProjectorServer<RunCostProjectorSvc>,
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

    // SLICE_03 R3 N1: socket file perms 0660. Default umask leaves the socket
    // world-readable; under hostPath mount this lets any UID on the host
    // speak gRPC. 0660 = rw for owner + group; the caller pod must share
    // fsGroup: 65532 (set by Helm pod-level securityContext).
    let perms = std::fs::Permissions::from_mode(0o660);
    std::fs::set_permissions(path, perms)
        .with_context(|| format!("set perms on uds `{}`", path.display()))?;
    info!(uds_path = %path.display(), mode = "0660", "run_cost_projector UDS perms set");

    let incoming = async_stream::stream! {
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => yield Ok::<_, std::io::Error>(stream),
                Err(e) => yield Err(e),
            }
        }
    };

    info!(uds = %uds_path, "binding run_cost_projector gRPC server (UDS, no mTLS — kernel-enforced trust)");
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
    tonic_svc: RunCostProjectorServer<RunCostProjectorSvc>,
) -> Result<()> {
    let listen_addr: SocketAddr = cfg
        .listen_addr
        .parse()
        .with_context(|| format!("invalid listen_addr `{}`", cfg.listen_addr))?;

    let tls = build_server_tls_config(cfg).context("loading mTLS server config")?;

    info!(addr = %cfg.listen_addr, mtls = tls.is_some(), "binding run_cost_projector gRPC server (TCP)");

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
            "run_cost_projector server starting WITHOUT mTLS — only acceptable in \
             POC / demo mode. Set SPENDGUARD_RUN_COST_PROJECTOR_TLS_{{CERT,KEY,CA}}_PEM \
             for production-correct mTLS (Helm production profile rejects this)."
        );
    }

    builder
        .add_service(tonic_svc)
        .serve_with_shutdown(listen_addr, shutdown_signal())
        .await
        .context("tonic TCP gRPC server failed")
}

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
        .unwrap_or_else(|_| EnvFilter::new("spendguard_run_cost_projector=info,info"));
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
    // Phase A: minimal metrics (extended in Phase D/F with real counters).
    "# HELP spendguard_run_cost_projector_project_total \
     Total Project RPCs handled.\n\
     # TYPE spendguard_run_cost_projector_project_total counter\n\
     spendguard_run_cost_projector_project_total 0\n\
     # HELP spendguard_run_cost_projector_terminate_run_total \
     Total TerminateRun RPCs handled.\n\
     # TYPE spendguard_run_cost_projector_terminate_run_total counter\n\
     spendguard_run_cost_projector_terminate_run_total 0\n"
        .to_string()
}

/// Minimal /metrics + /livez + /healthz + /readyz hyper server.
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
            let svc = service_fn(move |req: Request<hyper::body::Incoming>| async move {
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
            state_cache_ttl_seconds: 1800,
            state_cache_capacity: 10_000,
            replay_window_minutes: 30,
            cold_start_run_length: 10,
            drift_consecutive_threshold: 3,
            drift_ratio_threshold: 0.5,
        };
        assert!(build_server_tls_config(&cfg).expect("ok").is_none());
        cfg.tls_cert_pem = Some("/tmp/cert.pem".into());
        let err = build_server_tls_config(&cfg).expect_err("partial rejected");
        let msg = format!("{err:#}");
        assert!(msg.contains("partial mTLS config"), "got: {msg}");
    }

    #[test]
    fn render_metrics_contains_known_names() {
        let body = render_metrics();
        assert!(body.contains("spendguard_run_cost_projector_project_total"));
        assert!(body.contains("spendguard_run_cost_projector_terminate_run_total"));
    }
}
