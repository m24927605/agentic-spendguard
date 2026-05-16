use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    PgPool,
};

use crate::config::Config;

pub async fn connect(cfg: &Config) -> Result<PgPool, sqlx::Error> {
    let opts: PgConnectOptions = cfg.database_url.parse()?;
    // 5s → 30s to match ledger / outbox_forwarder / ttl_sweeper fix.
    // Demo bring-up races: bundles-init + canonical-seed-init hold pg
    // connections during their work; canonical-ingest's first acquire
    // can take >5s when those overlap.
    PgPoolOptions::new()
        .max_connections(cfg.db_max_connections)
        .acquire_timeout(std::time::Duration::from_secs(30))
        .connect_with(opts)
        .await
}
