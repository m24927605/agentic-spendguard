use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    PgPool,
};

use crate::config::Config;

pub async fn connect(cfg: &Config) -> Result<PgPool, sqlx::Error> {
    let opts: PgConnectOptions = cfg.database_url.parse()?;
    PgPoolOptions::new()
        .max_connections(cfg.db_max_connections)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect_with(opts)
        .await
}
