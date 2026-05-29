//! SpendGuard tokenizer gRPC service entry point.
//!
//! Spec ref `tokenizer-service-spec-v1alpha1.md` §2.1(a).
//!
//! SLICE_03 boot sequence:
//!
//!   1. Install rustls aws_lc_rs crypto provider (mirrors sidecar / ledger).
//!   2. Load env config via [`spendguard_tokenizer_service::config::Config`].
//!   3. Construct the in-process tokenizer (eager-loads encoder
//!      assets + verifies sha256 per spec §7.4 fail-fast).
//!   4. Spawn the /metrics hyper server on `metrics_addr`.
//!   5. Bind the tonic gRPC server on `listen_addr`.
//!   6. Block on graceful shutdown signal.
//!
//! Out of scope for SLICE_03:
//!   * mTLS bootstrap (Helm chart wires it in SLICE-extra; for the
//!     compose-based demo + unit tests we run plaintext on
//!     localhost).
//!   * UDS dual-bind (the chart can prefer the library form on the
//!     hot path; the gRPC form ships TCP only in SLICE_03).
//!   * Tier 1 shadow worker (SLICE_05).

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use tonic::transport::Server;
use tracing::{error, info};
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

    // ── Bind the gRPC server. ─────────────────────────────────────
    let listen_addr: SocketAddr = cfg
        .listen_addr
        .parse()
        .with_context(|| format!("invalid listen_addr `{}`", cfg.listen_addr))?;
    let svc = TokenizerSvc::new(Arc::clone(&tokenizer));

    info!(addr = %cfg.listen_addr, "binding tokenizer gRPC server");
    Server::builder()
        .add_service(TokenizerServer::new(svc))
        .serve_with_shutdown(listen_addr, shutdown_signal())
        .await
        .context("tonic gRPC server failed")?;

    info!("spendguard-tokenizer-service shut down cleanly");
    Ok(())
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
