use std::net::SocketAddr;

use anyhow::Context;
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use spendguard_canonical_ingest::{
    config::Config,
    persistence,
    proto::canonical_ingest::v1::canonical_ingest_server::CanonicalIngestServer,
    server::CanonicalIngestService,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let cfg = Config::from_env().context("loading config")?;
    info!(
        addr = %cfg.bind_addr,
        region = %cfg.region,
        "starting spendguard-canonical-ingest"
    );

    let pool = persistence::pool::connect(&cfg)
        .await
        .context("connecting to Postgres")?;

    let svc = CanonicalIngestService::new(pool, cfg.clone());

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
