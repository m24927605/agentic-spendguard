//! Postgres pool + durability config verification.

use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    PgPool, Row,
};
use tracing::{info, warn};

use crate::config::Config;

pub async fn connect(cfg: &Config) -> Result<PgPool, sqlx::Error> {
    let opts: PgConnectOptions = cfg.database_url.parse()?;

    // acquire_timeout bumped from 5s → 30s to match the parallel bump in
    // services/outbox_forwarder + services/ttl_sweeper (commit a539c27).
    // Ledger startup races with bundles-init + canonical-seed-init for
    // postgres connections during the demo bring-up burst; 5s wasn't
    // enough for ledger to acquire its first connection when those init
    // scripts hold connections.
    PgPoolOptions::new()
        .max_connections(cfg.db_max_connections)
        .acquire_timeout(std::time::Duration::from_secs(30))
        .connect_with(opts)
        .await
}

/// Verify Postgres durability config matches Stage 2 §4.2 + Ledger §19.
///
/// Hard-fails startup unless either:
///   - `SPENDGUARD_LEDGER_ALLOW_UNSAFE_DURABILITY=true` is set (POC unit
///     tests / CI without a sync replica), or
///   - `synchronous_commit` is `on`/`remote_apply`/`remote_write` AND
///     `synchronous_standby_names` is non-empty AND
///     `default_transaction_isolation` is `serializable`.
///
/// The audit invariant ("no audit, no effect") cannot be honored if the
/// ReserveSet response is allowed to return before sync-replica ack.
pub async fn verify_durability_settings(pool: &PgPool) -> Result<(), DurabilityError> {
    let row = sqlx::query(
        "SELECT
            current_setting('synchronous_commit')             AS sc,
            current_setting('synchronous_standby_names', true) AS sn,
            current_setting('default_transaction_isolation')   AS iso",
    )
    .fetch_one(pool)
    .await
    .map_err(DurabilityError::Db)?;

    let sc: String = row.get("sc");
    let sn: Option<String> = row.try_get("sn").ok();
    let iso: String = row.get("iso");

    info!(
        synchronous_commit = %sc,
        synchronous_standby_names = ?sn,
        default_transaction_isolation = %iso,
        "Postgres durability settings"
    );

    let allow_unsafe = std::env::var("SPENDGUARD_LEDGER_ALLOW_UNSAFE_DURABILITY")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let mut violations = Vec::new();

    if sc != "on" && sc != "remote_apply" && sc != "remote_write" {
        violations.push(format!(
            "synchronous_commit = '{}'; expected 'on' / 'remote_apply' / 'remote_write'",
            sc
        ));
    }
    match sn.as_deref() {
        Some(s) if !s.is_empty() => {}
        _ => violations.push(
            "synchronous_standby_names is empty; expected at least 1 sync replica \
             (e.g., 'ANY 1 (replica_b, replica_c)')"
                .to_string(),
        ),
    }
    if iso != "serializable" {
        violations.push(format!(
            "default_transaction_isolation = '{}'; expected 'serializable' (Ledger §19)",
            iso
        ));
    }

    if !violations.is_empty() {
        if allow_unsafe {
            for v in &violations {
                warn!("UNSAFE durability acknowledged via env var: {}", v);
            }
            warn!(
                "SPENDGUARD_LEDGER_ALLOW_UNSAFE_DURABILITY=true — service will \
                 NOT honor the audit invariant under failure (Stage 2 §4)"
            );
            return Ok(());
        }
        return Err(DurabilityError::Misconfigured(violations.join("; ")));
    }

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum DurabilityError {
    #[error("durability misconfigured: {0}")]
    Misconfigured(String),

    #[error("postgres error reading durability settings: {0}")]
    Db(#[from] sqlx::Error),
}
