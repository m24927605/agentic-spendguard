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
    // Symmetry with services/ttl_sweeper/src/state.rs::build_pg_pool —
    // bump max_connections + acquire_timeout so docker compose startup
    // doesn't crash the forwarder when postgres is briefly busy with
    // init scripts. See that file's comment for rationale.
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(30))
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

    // Build the endpoint once; reuse across retry attempts. URI parse
    // + TLS config errors are eager (fail-fast on misconfig).
    let endpoint = Channel::from_shared(config.canonical_ingest_url.clone())?
        .tls_config(tls)?
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10));

    // Retry-with-backoff: docker compose's `depends_on: service_started`
    // only guarantees the upstream container has been created — NOT that
    // DNS for its hostname has settled, NOR that its gRPC server has
    // bound the port. canonical_ingest doesn't expose a healthcheck
    // (tonic gRPC reflection isn't enabled per compose.yaml comment),
    // so adding `service_healthy` upstream isn't an option. We instead
    // tolerate transient connect failures at startup with a bounded
    // retry. Deadline 30s, exponential 250ms → 4s. Logs every attempt
    // at warn for operator visibility.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut backoff = Duration::from_millis(250);
    let mut attempt: u32 = 0;
    let channel = loop {
        attempt += 1;
        match endpoint.connect().await {
            Ok(c) => break c,
            Err(e) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(anyhow::anyhow!(
                        "canonical_ingest gRPC connect to {} failed after {} attempts; last error: {}",
                        config.canonical_ingest_url, attempt, e
                    ));
                }
                tracing::warn!(
                    target = %config.canonical_ingest_url,
                    attempt,
                    err = %e,
                    next_backoff_ms = backoff.as_millis() as u64,
                    "canonical_ingest gRPC connect failed; retrying with backoff"
                );
                tokio::time::sleep(backoff).await;
                backoff = std::cmp::min(backoff * 2, Duration::from_secs(4));
            }
        }
    };

    info!(
        target = %config.canonical_ingest_url,
        attempts = attempt,
        "canonical_ingest gRPC client connected"
    );
    Ok(CanonicalIngestClient::new(channel))
}
