//! PgListener loop + startup recovery scan.
//!
//! Subscribes to `approval_requests_state_change` (per migration
//! 0043's `approval_requests_state_change_notify` trigger) and
//! dispatches new_state='approved' + proposal_source='cost_advisor'
//! transitions to the bundle-apply path.
//!
//! NOTIFY delivery semantics (codex CA-P3.5 r1 P1):
//!   * Live: at-most-once delivery to any actively-listening
//!     session at the time the NOTIFY commits.
//!   * Cross-restart: NOT durable. Notifications fired while
//!     bundle_registry is down are lost. `recover_pending()` on
//!     startup catches missed approvals by scanning the DB.
//!   * Re-apply safety: the patch's `test` op pins identity;
//!     re-running the same patch on already-patched bundle bytes
//!     produces bit-identical output, so the apply path skips disk
//!     writes when sha256 is unchanged.

use anyhow::{Context, Result};
use serde::Deserialize;
use sqlx::postgres::PgListener;
use sqlx::PgPool;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::Config;

/// Startup recovery: re-apply every approved cost_advisor proposal.
/// Required because LISTEN/NOTIFY is not durable across listener
/// disconnects; missed approvals would otherwise stay stranded as
/// state=approved with no contract update.
///
/// v0.1 limitations (codex CA-P3.5 r3 P2 acknowledged):
///   * Replays EVERY approved cost_advisor approval on every startup
///     — there's no applied-cursor or watermark. For multi-approval
///     histories on the same tenant + budget, the older patches'
///     `test` ops may fail against the current bundle state, but
///     the loop continues to apply the rest. The FINAL bundle state
///     converges to the latest-resolved approval's intended TTL/limit
///     because rows are ordered by `resolved_at ASC` + the latest
///     row's `replace` op overwrites earlier values.
///   * Per-approval apply errors are logged STRUCTURED but the loop
///     does NOT abort. An apply failure on a single approval doesn't
///     block others from being processed. The function returns Ok
///     even when individual applies fail; the operator monitors
///     bundle_registry logs for `recovery_apply_failed` events.
pub async fn recover_pending(pool: &PgPool, config: &crate::Config) -> Result<()> {
    let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
        r#"
        SELECT approval_id, tenant_id
          FROM approval_requests
         WHERE state = 'approved'
           AND proposal_source = 'cost_advisor'
         ORDER BY resolved_at ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .context("startup recovery scan")?;

    if rows.is_empty() {
        info!("startup recovery: no queued approvals");
        return Ok(());
    }
    info!(count = rows.len(), "startup recovery: applying queued approvals");

    let mut applied = 0u32;
    let mut failed = 0u32;
    for (approval_id, tenant_id) in rows {
        match crate::apply::process_approval(pool, approval_id, tenant_id, config).await {
            Ok(_) => {
                info!(approval_id = %approval_id, "startup recovery: applied");
                applied += 1;
            }
            Err(e) => {
                // Loud structured log so operators can grep for the
                // event. We do NOT abort — one bad approval shouldn't
                // block subsequent ones from being processed.
                warn!(
                    event = "recovery_apply_failed",
                    approval_id = %approval_id,
                    tenant_id = %tenant_id,
                    error = %format!("{:#}", e),
                    "startup recovery: apply failed (approval stays approved, no bundle update)"
                );
                failed += 1;
            }
        }
    }
    info!(applied, failed, "startup recovery: done");
    Ok(())
}

/// Payload structure emitted by `approval_requests_notify_state_change()`
/// in migration 0043. Mirrors the trigger's json_build_object shape.
#[derive(Debug, Deserialize)]
struct NotificationPayload {
    approval_id: Uuid,
    tenant_id: Uuid,
    proposal_source: String,
    old_state: String,
    new_state: String,
    #[serde(default)]
    resolved_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn run(pool: PgPool, config: &Config) -> Result<()> {
    // Order of operations matters (codex CA-P3.5 r2 P1):
    //   1. Open the listener + LISTEN on the channel FIRST. Any
    //      NOTIFY committed from this point on is queued for us.
    //   2. THEN run recovery scan. Approvals committed during the
    //      gap BEFORE step 1 are caught by the scan; approvals
    //      committed AFTER step 1 are caught by the NOTIFY queue.
    //      Either path applies the same patch idempotently, so
    //      duplicate delivery (recovery + NOTIFY for the same row)
    //      is safe.
    //   3. THEN enter recv() loop to drain queued + future NOTIFYs.
    // The previous order (recover before LISTEN) had a lost-notify
    // race for approvals committed in the gap.
    let mut listener = PgListener::connect_with(&pool)
        .await
        .context("PgListener connect")?;
    listener
        .listen(&config.notify_channel)
        .await
        .context("listen on channel")?;
    info!(channel = %config.notify_channel, "listener active");

    recover_pending(&pool, config)
        .await
        .context("startup recovery scan")?;

    loop {
        // try_recv() returns Ok(None) when PgListener transparently
        // reconnected after a transient connection drop — any
        // NOTIFYs that fired during the disconnect window are
        // dropped (sqlx docs:
        // https://docs.rs/sqlx/latest/sqlx/postgres/struct.PgListener.html#method.try_recv).
        // Re-run recovery scan on each reconnect so approvals
        // committed in that window are still applied (codex
        // CA-P3.5 r4 P2).
        let notif = match listener.try_recv().await {
            Ok(Some(n)) => n,
            Ok(None) => {
                info!("PgListener reconnected; replaying recovery scan to catch missed NOTIFYs");
                if let Err(e) = recover_pending(&pool, config).await {
                    warn!(
                        error = %format!("{:#}", e),
                        "post-reconnect recovery scan failed; continuing"
                    );
                }
                continue;
            }
            Err(e) => {
                warn!(error = %e, "listener.try_recv error; retrying");
                continue;
            }
        };

        let payload: NotificationPayload = match serde_json::from_str(notif.payload()) {
            Ok(p) => p,
            Err(e) => {
                warn!(
                    error = %e,
                    raw = %notif.payload(),
                    "could not parse NOTIFY payload; skipping"
                );
                continue;
            }
        };

        debug!(
            approval_id = %payload.approval_id,
            tenant_id = %payload.tenant_id,
            proposal_source = %payload.proposal_source,
            old_state = %payload.old_state,
            new_state = %payload.new_state,
            "received state-change notification"
        );

        if payload.proposal_source != "cost_advisor" {
            debug!(proposal_source = %payload.proposal_source, "not a cost_advisor proposal; skipping");
            continue;
        }
        if payload.new_state != "approved" {
            debug!(new_state = %payload.new_state, "not an approve transition; skipping");
            continue;
        }

        info!(
            approval_id = %payload.approval_id,
            tenant_id = %payload.tenant_id,
            "processing approved cost_advisor proposal"
        );

        match crate::apply::process_approval(
            &pool,
            payload.approval_id,
            payload.tenant_id,
            config,
        )
        .await
        {
            Ok(result) => info!(
                approval_id = %payload.approval_id,
                new_bundle_hash = %result.new_bundle_hash,
                "bundle applied + manifest updated"
            ),
            Err(e) => {
                // Don't kill the listener loop on a single failure;
                // log + continue so subsequent approvals still get
                // processed. The operator can re-trigger by manually
                // toggling state if needed (out of v0.1 scope).
                warn!(
                    approval_id = %payload.approval_id,
                    error = %format!("{:#}", e),
                    "bundle apply failed; leaving approval in approved state"
                );
            }
        }
    }
}
