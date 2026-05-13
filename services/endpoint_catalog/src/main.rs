use anyhow::Context;
use std::net::SocketAddr;
use tracing::info;
use tracing_subscriber::EnvFilter;

use spendguard_endpoint_catalog::{
    config::ServerConfig,
    persistence::store::make_store,
    server::router,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("failed to install rustls aws_lc_rs default provider"))?;

    init_tracing();

    let cfg = ServerConfig::from_env().context("loading server config")?;
    info!(
        addr = %cfg.bind_addr,
        backend = %cfg.storage.storage_backend,
        region = %cfg.storage.region,
        "starting spendguard-endpoint-catalog"
    );

    let store = make_store(&cfg.storage).context("init store")?;
    let app = router(store, cfg.clone());

    let addr: SocketAddr = cfg.bind_addr.parse().context("parse bind_addr")?;
    let listener = tokio::net::TcpListener::bind(addr).await.context("bind")?;
    info!("listening on {}", addr);

    axum::serve(listener, app).await.context("serve")?;
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .json()
        .init();
}
