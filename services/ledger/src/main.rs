use std::net::SocketAddr;

use anyhow::Context;
use tonic::transport::{
    Certificate, Identity, Server, ServerTlsConfig,
};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use spendguard_ledger::{
    config::Config,
    persistence,
    proto::ledger::v1::ledger_server::LedgerServer,
    server::LedgerService,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let cfg = Config::from_env().context("loading config")?;
    info!(
        addr = %cfg.bind_addr,
        db_max_conns = cfg.db_max_connections,
        "starting spendguard-ledger"
    );

    let pool = persistence::pool::connect(&cfg)
        .await
        .context("connecting to Postgres")?;

    persistence::pool::verify_durability_settings(&pool)
        .await
        .context("verifying Postgres durability config (Stage 2 §4.2)")?;

    // Phase 5 GA hardening S6: producer signer for ledger-server-minted
    // audit rows (currently only InvoiceReconcile's synthesized decision).
    let signer = std::sync::Arc::<dyn spendguard_signing::Signer>::from(
        spendguard_signing::signer_from_env("SPENDGUARD_LEDGER")
            .context("S6: build signer from SPENDGUARD_LEDGER_SIGNING_* env")?,
    );
    info!(
        key_id = %signer.key_id(),
        algorithm = %signer.algorithm(),
        producer = %signer.producer_identity(),
        "S6: ledger producer signer initialized"
    );

    let svc = LedgerService::new(pool, signer);

    let addr: SocketAddr = cfg.bind_addr.parse().context("parsing bind addr")?;

    let tls = build_server_tls_config(&cfg)
        .context("loading mTLS server config")?;

    info!(
        addr = %addr,
        mtls = tls.is_some(),
        "listening"
    );

    let mut builder = Server::builder();
    if let Some(tls_cfg) = tls {
        builder = builder
            .tls_config(tls_cfg)
            .context("apply server TLS config")?;
    } else {
        warn!(
            "ledger server starting WITHOUT mTLS — only acceptable in \
             POC dev mode. Set SPENDGUARD_LEDGER_TLS_{{CERT,KEY,CA}}_PEM \
             for production-correct mTLS."
        );
    }

    builder
        .add_service(LedgerServer::new(svc))
        .serve(addr)
        .await
        .context("gRPC server terminated")?;

    Ok(())
}

/// Build the server-side TLS config when all three of cert/key/ca paths
/// are configured; return None to fall back to plaintext gRPC. Partial
/// configuration (e.g., cert without ca) is rejected as an error to fail
/// closed against accidental production deployments missing CA pinning.
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
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .json()
        .init();
}
