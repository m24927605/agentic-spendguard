//! Server bootstrap: AppState, axum routers, ledger gRPC client.

use axum::{routing::get, routing::post, Router};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity};
use tower_http::limit::RequestBodyLimitLayer;
use tracing::info;

use crate::{
    config::Config,
    handlers::{health, webhook},
    metrics::{record_metrics, WebhookReceiverMetrics},
    persistence::sequence::SequenceAllocator,
    proto::ledger::v1::ledger_client::LedgerClient,
};

use axum::middleware::from_fn_with_state;

const REQUEST_BODY_LIMIT_BYTES: usize = 64 * 1024;

pub struct AppState {
    pub config: Config,
    pub pg: PgPool,
    pub ledger_client: LedgerClient<Channel>,
    pub seq: SequenceAllocator,
    /// Phase 5 GA hardening S6: producer signing. Constructed at
    /// startup from `SPENDGUARD_WEBHOOK_RECEIVER_SIGNING_*` env vars.
    pub signer: Arc<dyn spendguard_signing::Signer>,
}

pub async fn build_pg_pool(database_url: &str) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
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

pub fn build_https_router(state: Arc<AppState>, metrics: WebhookReceiverMetrics) -> Router {
    Router::new()
        .route("/v1/webhook/:provider", post(webhook::handle_webhook))
        .layer(RequestBodyLimitLayer::new(REQUEST_BODY_LIMIT_BYTES))
        .layer(from_fn_with_state(metrics, record_metrics))
        .with_state(state)
}

pub fn build_health_router(state: Arc<AppState>, metrics: WebhookReceiverMetrics) -> Router {
    Router::new()
        .route("/healthz", get(health::healthz))
        .layer(from_fn_with_state(metrics, record_metrics))
        .with_state(state)
}
