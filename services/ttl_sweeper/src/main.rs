use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info};
use uuid::Uuid;

use spendguard_ttl_sweeper::{
    config::Config,
    poll::fetch_expired,
    sequence::{recover_max_seq, SequenceAllocator},
    state::{build_ledger_client, build_pg_pool, AppState},
    sweep::sweep_one,
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
        ledger_url = %config.ledger_url,
        poll_interval_seconds = config.poll_interval_seconds,
        batch_size = config.batch_size,
        "ttl-sweeper starting"
    );

    let pg = build_pg_pool(&config.database_url).await?;

    let tenant_uuid = Uuid::parse_str(&config.tenant_id)?;
    let max_seq = recover_max_seq(&pg, tenant_uuid, &config.workload_instance_id).await?;
    let seq_start = max_seq + 1;
    info!(workload = %config.workload_instance_id, max_seq, seq_start, "producer_sequence recovered");
    let seq = SequenceAllocator::new(seq_start);

    let ledger_client = build_ledger_client(&config).await?;

    let mut state = AppState {
        config: config.clone(),
        pg,
        ledger_client,
        seq,
    };

    let poll_dur = Duration::from_secs(config.poll_interval_seconds);
    info!("entering sweep loop");

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("ctrl-c received; exiting");
                break;
            }
            _ = sleep(poll_dur) => {
                match fetch_expired(&state.pg, tenant_uuid, state.config.batch_size).await {
                    Ok(rows) => {
                        if rows.is_empty() {
                            tracing::debug!("no expired reservations");
                        } else {
                            info!(count = rows.len(), "found expired reservations");
                            for row in rows {
                                if let Err(e) = sweep_one(&mut state, row).await {
                                    error!(error = ?e, "sweep_one failed");
                                }
                            }
                        }
                    }
                    Err(e) => error!(error = ?e, "fetch_expired failed"),
                }
            }
        }
    }
    Ok(())
}
