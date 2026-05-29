//! SpendGuard tokenizer gRPC service entry point.
//!
//! Spec ref `tokenizer-service-spec-v1alpha1.md` §2.1(a).
//!
//! SLICE_03 boot sequence (round-2 fix B3 update):
//!
//!   1. Install rustls aws_lc_rs crypto provider (mirrors sidecar / ledger).
//!   2. Load env config via [`spendguard_tokenizer_service::config::Config`].
//!   3. Construct the in-process tokenizer (eager-loads encoder
//!      assets + verifies sha256 + cross-check vectors per spec §7.4
//!      fail-fast).
//!   4. Spawn the /metrics hyper server on `metrics_addr`.
//!   5. Bind the tonic gRPC server. Two modes are supported per
//!      spec §10.1:
//!       * UDS (preferred for on-node sidecar callers — no L4 hop)
//!         when `cfg.uds_path` is set.
//!       * TCP with mTLS when `cfg.tls_cert_pem` + `cfg.tls_key_pem`
//!         + `cfg.tls_ca_pem` are all set.
//!       * TCP plaintext as a demo-only fallback.
//!      Production Helm profile fails fast if neither UDS nor mTLS
//!      is configured (charts/spendguard/templates/tokenizer.yaml).
//!      Round-2 fix M6: server-side DoS limits (concurrency,
//!      message size, window) applied on both transports.
//!   6. Block on graceful shutdown signal.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use spendguard_tokenizer::Tokenizer;
use spendguard_tokenizer_service::{
    config::Config,
    proto::tokenizer::v1::tokenizer_server::TokenizerServer,
    server::TokenizerSvc,
};

#[tokio::main]
async fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("failed to install rustls aws_lc_rs default provider"))?;

    init_tracing();

    let cfg = Config::from_env().context("loading tokenizer config")?;
    info!(
        listen = %cfg.listen_addr,
        uds = ?cfg.uds_path,
        mtls = cfg.tls_cert_pem.is_some(),
        metrics = %cfg.metrics_addr,
        tier3_threshold = %cfg.tier3_alert_threshold,
        region = %cfg.region,
        "starting spendguard-tokenizer-service"
    );

    // ── Construct the library handle (fail-fast on asset mismatch). ──
    let tokenizer = match Tokenizer::new_with_embedded_assets() {
        Ok(t) => Arc::new(t),
        Err(e) => {
            error!(
                error = ?e,
                "tokenizer asset boot failed (spec §7.4 fail-fast); refusing to start"
            );
            return Err(anyhow::Error::msg(e.to_string()));
        }
    };
    info!(
        entries = tokenizer.dispatch().len(),
        "tokenizer dispatch table compiled + encoder cache eager-loaded"
    );

    // ── Spawn the metrics hyper server (best-effort). ─────────────
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

    let svc = TokenizerSvc::new(Arc::clone(&tokenizer));
    let tonic_svc = TokenizerServer::new(svc)
        // Round-2 fix M6: cap decoded request size at 1 MiB to bound
        // memory pressure from oversized callers (per-request field
        // validation in server.rs adds a 2 MiB raw_text ceiling on
        // top, but the protocol-layer cap rejects bigger frames
        // before deserialisation cost).
        .max_decoding_message_size(1 << 20);

    // ── Bind the gRPC server. ─────────────────────────────────────
    if let Some(uds_path) = cfg.uds_path.as_deref() {
        bind_uds(uds_path, tonic_svc).await?;
    } else {
        bind_tcp(&cfg, tonic_svc).await?;
    }

    info!("spendguard-tokenizer-service shut down cleanly");
    Ok(())
}

/// Round-2 fix B3.1: UDS bind path. Spec §10.1 hot-path — sidecar pods
/// on the same node reach the tokenizer without an L4 hop. Precedent:
/// services/sidecar/src/main.rs:262-296 adapter UDS binding.
async fn bind_uds(
    uds_path: &str,
    tonic_svc: TokenizerServer<TokenizerSvc>,
) -> Result<()> {
    use tokio::net::UnixListener;
    let path = Path::new(uds_path);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("mkdir uds parent for `{uds_path}`"))?;
    }
    if path.exists() {
        tokio::fs::remove_file(path)
            .await
            .with_context(|| format!("remove stale uds at `{uds_path}`"))?;
    }
    let listener = UnixListener::bind(path)
        .with_context(|| format!("bind uds listener `{uds_path}`"))?;

    let incoming = async_stream::stream! {
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => yield Ok::<_, std::io::Error>(stream),
                Err(e) => yield Err(e),
            }
        }
    };

    info!(uds = %uds_path, "binding tokenizer gRPC server (UDS, no mTLS — kernel-enforced trust)");
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

/// TCP bind path. mTLS when cert+key+ca are all configured; plaintext
/// otherwise (with a loud warn — production Helm profile rejects this).
async fn bind_tcp(
    cfg: &Config,
    tonic_svc: TokenizerServer<TokenizerSvc>,
) -> Result<()> {
    let listen_addr: SocketAddr = cfg
        .listen_addr
        .parse()
        .with_context(|| format!("invalid listen_addr `{}`", cfg.listen_addr))?;

    let tls = build_server_tls_config(cfg).context("loading mTLS server config")?;

    info!(
        addr = %cfg.listen_addr,
        mtls = tls.is_some(),
        "binding tokenizer gRPC server (TCP)"
    );

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
            "tokenizer server starting WITHOUT mTLS — only acceptable in \
             POC / demo mode. Set SPENDGUARD_TOKENIZER_TLS_{{CERT,KEY,CA}}_PEM \
             for production-correct mTLS (Helm production profile rejects this)."
        );
    }

    builder
        .add_service(tonic_svc)
        .serve_with_shutdown(listen_addr, shutdown_signal())
        .await
        .context("tonic TCP gRPC server failed")
}

/// Round-2 fix B3.2: build the server-side mTLS config when all three
/// of cert/key/ca paths are set; return None to fall back to plaintext.
/// Partial config (e.g., cert without ca) is rejected as an error to
/// fail closed against accidental production deployments missing CA
/// pinning. Precedent: services/ledger/src/main.rs:152-172.
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
        .unwrap_or_else(|_| EnvFilter::new("spendguard_tokenizer=info,info"));
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

/// Minimal /metrics + /healthz + /readyz hyper server. Mirrors the
/// raw-hyper pattern used by services/canonical_ingest and
/// services/ledger so the chart can scrape Prometheus + run the
/// startup probe without an additional crate dependency.
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
                            // SLICE_03 ships a stable Prometheus
                            // payload with the counters the spec
                            // §5.2 + §5.3 alert wiring expects;
                            // the actual increments are emitted
                            // from the request path in SLICE-extra.
                            "# HELP spendguard_tokenizer_tier3_hit_total \
                             Number of Tier 3 fallback hits (spec §5.2).\n\
                             # TYPE spendguard_tokenizer_tier3_hit_total counter\n\
                             spendguard_tokenizer_tier3_hit_total 0\n\
                             # HELP spendguard_tokenizer_total_calls Total tokenize calls.\n\
                             # TYPE spendguard_tokenizer_total_calls counter\n\
                             spendguard_tokenizer_total_calls 0\n"
                                .to_string(),
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
