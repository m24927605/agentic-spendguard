use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info};

use spendguard_outbox_forwarder::{
    config::Config,
    forward::forward_batch,
    state::{build_canonical_client, build_pg_pool, AppState},
};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("failed to install rustls aws_lc_rs default provider"))?;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .json()
        .init();

    let config = Config::from_env()?;
    info!(
        canonical_ingest_url = %config.canonical_ingest_url,
        poll_interval_seconds = config.poll_interval_seconds,
        batch_size = config.batch_size,
        "outbox-forwarder starting"
    );

    let pg = build_pg_pool(&config.database_url).await?;
    let canonical_client = build_canonical_client(&config).await?;

    let mut state = AppState {
        config: config.clone(),
        pg,
        canonical_client,
    };

    let poll_dur = Duration::from_secs(config.poll_interval_seconds);
    info!("entering forward loop");

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("ctrl-c received; exiting");
                break;
            }
            _ = sleep(poll_dur) => {
                match forward_batch(&mut state).await {
                    Ok(0) => tracing::debug!("no pending rows"),
                    Ok(n) => info!(count = n, "batch processed"),
                    Err(e) => error!(error = ?e, "forward_batch failed"),
                }
            }
        }
    }
    Ok(())
}
