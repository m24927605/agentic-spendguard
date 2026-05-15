//! `spendguard-bundle-registry` — Cost Advisor closed-loop applicator.
//!
//! Listens on PgListener channel `approval_requests_state_change`.
//! When a cost_advisor proposal transitions to `approved`, fetches
//! the row's `proposed_dsl_patch`, applies it to the active contract
//! bundle (`/var/lib/spendguard/bundles/contract_bundle/<id>.tgz`),
//! re-bundles deterministically, and updates the runtime.env hash.
//!
//! **Sidecar reload semantics** (codex CA-P3.5 r1 P1): the sidecar
//! in v0.1 loads bundles ONLY at startup; there is no hot-reload.
//! bundle_registry's job is to publish the new bundle bytes + hash
//! into the bundles volume. Picking up the new contract requires
//! the operator's CD pipeline to restart the sidecar (or for
//! production, a hot-reload feature that is NOT in v0.1 scope —
//! see services/sidecar/src/decision/transaction.rs comment "POC
//! has no hot-reload"). The demo verifies bundle bytes land; it
//! does NOT verify a running sidecar starts using them.
//!
//! **Durability + recovery** (codex CA-P3.5 r1 P1): PostgreSQL
//! LISTEN/NOTIFY is NOT durable across listener disconnects.
//! Notifications fired while bundle_registry is down are lost.
//! On startup we scan for state=approved + proposal_source=
//! cost_advisor approvals and (re-)apply each one before entering
//! the listener loop. Re-application is safe: the RFC-6902 patch
//! is idempotent (test+replace on the same value produces
//! bit-identical output), and the apply path skips disk writes
//! when sha256 is unchanged.
//!
//! **Singleton enforcement**: PostgreSQL `NOTIFY` delivers to ALL
//! listening sessions, so running multiple bundle_registry
//! instances would race on the apply. compose enforces singleton
//! via container_name; production deployments should add a
//! leader-lease (out of v0.1 scope).

use anyhow::{Context, Result};
use serde::Deserialize;
use sqlx::postgres::PgPoolOptions;
use std::path::PathBuf;
use tracing::{error, info};

mod apply;
mod bundle;
mod listener;

#[derive(Debug, Deserialize)]
struct Config {
    /// Postgres URL for the ledger DB (where approval_requests lives).
    ledger_db: String,

    /// Path to the contract bundle directory the sidecar reads from.
    /// The bundle_registry mutates files in this directory:
    ///   - <CONTRACT_BUNDLE_ID>.tgz             (new bytes)
    ///   - <CONTRACT_BUNDLE_ID>.tgz.sig         (re-stamped placeholder)
    /// And the sibling runtime.env so the sidecar's CONTRACT_BUNDLE_HASH_HEX
    /// pin matches the on-disk bytes.
    contract_bundle_dir: PathBuf,

    /// UUID of the contract bundle file (filename stem). Pinned at
    /// startup so re-runs of bundle_registry against the same demo
    /// don't accidentally target a different bundle.
    contract_bundle_id: String,

    /// Path to runtime.env that the sidecar reads ONCE at startup
    /// (v0.1 has no hot-reload, see module-level doc). bundle_registry
    /// rewrites the SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX line
    /// atomically (write tempfile, fsync, rename) so a sidecar
    /// restart picks up the new hash + matching .tgz bytes.
    runtime_env_path: PathBuf,

    /// Channel to LISTEN on. Pinned for explicitness; defaults to
    /// the channel the trigger in migration 0043 fires on.
    #[serde(default = "default_channel")]
    notify_channel: String,
}

fn default_channel() -> String {
    "approval_requests_state_change".to_string()
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,spendguard_bundle_registry=debug")),
        )
        .json()
        .with_writer(std::io::stderr)
        .init();

    let config: Config = envy::prefixed("SPENDGUARD_BUNDLE_REGISTRY_")
        .from_env()
        .context("read SPENDGUARD_BUNDLE_REGISTRY_* env vars")?;

    info!(
        ledger_db_redacted = ?redact_db_url(&config.ledger_db),
        contract_bundle_dir = %config.contract_bundle_dir.display(),
        contract_bundle_id = %config.contract_bundle_id,
        runtime_env_path = %config.runtime_env_path.display(),
        notify_channel = %config.notify_channel,
        "bundle_registry starting"
    );

    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&config.ledger_db)
        .await
        .context("connect ledger DB")?;

    if let Err(e) = listener::run(pool, &config).await {
        error!(error = %e, "listener loop exited with error");
        return Err(e);
    }
    Ok(())
}

fn redact_db_url(url: &str) -> String {
    // Strip any inline password so logs don't leak it.
    if let Some(at) = url.rfind('@') {
        if let Some(scheme_end) = url.find("://") {
            let host = &url[at + 1..];
            return format!("{}://<redacted>@{}", &url[..scheme_end], host);
        }
    }
    url.to_string()
}
