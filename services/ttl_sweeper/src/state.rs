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
    // acquire_timeout bumped from 5s → 30s to match the gRPC retry
    // deadline below: docker compose startup occasionally finds
    // postgres busy with init scripts (PKI / bundles / canonical
    // seed) when the first sweep-cycle query arrives. The leader
    // election lease loop also holds 1 connection long-term out of
    // max_connections, so the effective pool for sweep work is 9,
    // not 10. 30s gives postgres time to settle without crashing
    // the service.
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(30))
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

    // Build the endpoint once; reuse across retry attempts. URI parse
    // + TLS config errors are eager (fail-fast on misconfig).
    let endpoint = Channel::from_shared(config.ledger_url.clone())?
        .tls_config(tls)?
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10));

    // Retry-with-backoff: docker compose's `depends_on: service_started`
    // for ledger only guarantees the container has been created — NOT
    // that DNS for its hostname has settled, NOR that the gRPC server
    // has bound the port. ledger doesn't expose a healthcheck (tonic
    // gRPC reflection isn't enabled per compose.yaml comment), so
    // adding `service_healthy` upstream isn't an option. We instead
    // tolerate transient connect failures at startup with a bounded
    // retry. Deadline 30s, exponential 250ms → 4s. Mirrors the same
    // wrapper in services/outbox_forwarder/src/state.rs::
    // build_canonical_client; if a 3rd service needs the same shape,
    // extract to a shared util at that point.
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
                        "ledger gRPC connect to {} failed after {} attempts; last error: {}",
                        config.ledger_url, attempt, e
                    ));
                }
                tracing::warn!(
                    target = %config.ledger_url,
                    attempt,
                    err = %e,
                    next_backoff_ms = backoff.as_millis() as u64,
                    "ledger gRPC connect failed; retrying with backoff"
                );
                tokio::time::sleep(backoff).await;
                backoff = std::cmp::min(backoff * 2, Duration::from_secs(4));
            }
        }
    };

    info!(
        target = %config.ledger_url,
        attempts = attempt,
        "ledger gRPC client connected"
    );
    Ok(LedgerClient::new(channel))
}
