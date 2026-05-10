use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use spendguard_canonical_ingest::{
    config::Config,
    metrics::IngestMetrics,
    persistence,
    proto::canonical_ingest::v1::canonical_ingest_server::CanonicalIngestServer,
    server::CanonicalIngestService,
};
use spendguard_signing::Verifier as _; // for `.key_count()`

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let cfg = Config::from_env().context("loading config")?;
    info!(
        addr = %cfg.bind_addr,
        region = %cfg.region,
        strict_signatures = cfg.strict_signatures,
        "starting spendguard-canonical-ingest"
    );

    let pool = persistence::pool::connect(&cfg)
        .await
        .context("connecting to Postgres")?;

    // Phase 5 GA hardening S8: build trust store + metrics.
    let verifier: Option<Arc<dyn spendguard_signing::Verifier>> = match &cfg.trust_store_dir {
        Some(dir) => {
            let v = spendguard_signing::LocalEd25519Verifier::from_dir(Path::new(dir))
                .context("S8: load trust store")?;
            info!(dir = %dir, keys = v.key_count(), "S8: trust store loaded");
            Some(Arc::new(v))
        }
        None if cfg.strict_signatures => {
            anyhow::bail!(
                "strict_signatures=true but SPENDGUARD_CANONICAL_INGEST_TRUST_STORE_DIR is unset"
            );
        }
        None => {
            warn!("S8: no trust store configured; signature verification disabled");
            None
        }
    };
    let metrics = IngestMetrics::new();

    // Spawn the Prometheus metrics HTTP server in the background.
    if !cfg.metrics_addr.is_empty() {
        let metrics_addr: SocketAddr = cfg.metrics_addr.parse().context("parsing metrics addr")?;
        let metrics_handle = metrics.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_metrics(metrics_addr, metrics_handle).await {
                warn!(err = %e, "metrics server terminated");
            }
        });
        info!(addr = %metrics_addr, "metrics server bound");
    }

    let svc = CanonicalIngestService::new(pool, cfg.clone(), verifier, metrics);

    let addr: SocketAddr = cfg.bind_addr.parse().context("parsing bind addr")?;

    let tls = build_server_tls_config(&cfg)
        .context("loading mTLS server config")?;
    info!(addr = %addr, mtls = tls.is_some(), "listening");

    let mut builder = Server::builder();
    if let Some(tls_cfg) = tls {
        builder = builder
            .tls_config(tls_cfg)
            .context("apply server TLS config")?;
    } else {
        warn!(
            "canonical-ingest server starting WITHOUT mTLS — only \
             acceptable in POC dev mode. Set \
             SPENDGUARD_CANONICAL_INGEST_TLS_{{CERT,KEY,CA}}_PEM for \
             production-correct mTLS."
        );
    }

    builder
        .add_service(CanonicalIngestServer::new(svc))
        .serve(addr)
        .await
        .context("gRPC server terminated")?;

    Ok(())
}

/// Phase 5 GA hardening S8: minimal HTTP /metrics endpoint that
/// renders the IngestMetrics Prometheus text. No external prometheus
/// crate to keep the dep tree lean.
async fn serve_metrics(addr: SocketAddr, metrics: IngestMetrics) -> anyhow::Result<()> {
    use hyper::body::Bytes;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper::{Request, Response};
    use hyper_util::rt::TokioIo;
    use http_body_util::Full;
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(addr).await?;
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
                    Ok::<_, std::convert::Infallible>(
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

fn build_server_tls_config(cfg: &Config) -> anyhow::Result<Option<ServerTlsConfig>> {
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
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .json()
        .init();
}
