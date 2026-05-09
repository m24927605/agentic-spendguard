//! AppState + bootstrap helpers (PG pool, ledger gRPC client).

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity};
use tracing::info;

use crate::{
    config::Config,
    proto::ledger::v1::ledger_client::LedgerClient,
    sequence::SequenceAllocator,
};

pub struct AppState {
    pub config: Config,
    pub pg: PgPool,
    pub ledger_client: LedgerClient<Channel>,
    pub seq: SequenceAllocator,
    /// Phase 5 GA hardening S6: producer signer.
    pub signer: std::sync::Arc<dyn spendguard_signing::Signer>,
}

pub async fn build_pg_pool(database_url: &str) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(database_url)
        .await?;
    Ok(pool)
}

pub async fn build_ledger_client(config: &Config) -> anyhow::Result<LedgerClient<Channel>> {
    let ca = tokio::fs::read(&config.tls_ca_pem).await?;
    let ca_cert = Certificate::from_pem(ca);
    let client_cert = tokio::fs::read(&config.tls_client_cert).await?;
    let client_key = tokio::fs::read(&config.tls_client_key).await?;
    let identity = Identity::from_pem(client_cert, client_key);

    let tls = ClientTlsConfig::new()
        .domain_name("ledger.spendguard.internal")
        .ca_certificate(ca_cert)
        .identity(identity);

    let channel = Channel::from_shared(config.ledger_url.clone())?
        .tls_config(tls)?
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .connect()
        .await?;

    info!(target = %config.ledger_url, "ledger gRPC client connected");
    Ok(LedgerClient::new(channel))
}
