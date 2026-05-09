use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity};
use tracing::info;

use crate::{
    config::Config,
    proto::canonical_ingest::v1::canonical_ingest_client::CanonicalIngestClient,
};

pub struct AppState {
    pub config: Config,
    pub pg: PgPool,
    pub canonical_client: CanonicalIngestClient<Channel>,
}

pub async fn build_pg_pool(database_url: &str) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(database_url)
        .await?;
    Ok(pool)
}

pub async fn build_canonical_client(
    config: &Config,
) -> anyhow::Result<CanonicalIngestClient<Channel>> {
    let ca = tokio::fs::read(&config.tls_ca_pem).await?;
    let ca_cert = Certificate::from_pem(ca);
    let client_cert = tokio::fs::read(&config.tls_client_cert).await?;
    let client_key = tokio::fs::read(&config.tls_client_key).await?;
    let identity = Identity::from_pem(client_cert, client_key);

    let tls = ClientTlsConfig::new()
        .domain_name("canonical-ingest.spendguard.internal")
        .ca_certificate(ca_cert)
        .identity(identity);

    let channel = Channel::from_shared(config.canonical_ingest_url.clone())?
        .tls_config(tls)?
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .connect()
        .await?;

    info!(target = %config.canonical_ingest_url, "canonical_ingest gRPC client connected");
    Ok(CanonicalIngestClient::new(channel))
}
