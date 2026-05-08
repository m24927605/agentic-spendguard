//! Fencing scope acquire / renew (Sidecar §9 + Stage 2 §4.4).
//!
//! Sidecar acquires a budget_window or reservation scope from the ledger
//! at startup. The ledger's `fencing_scopes` table is the single source
//! of truth for fencing epoch (Stage 2 D11). On takeover (e.g., new pod
//! after preStop drain), the new sidecar CAS-increments the scope's
//! `current_epoch`, fencing out any previous owner.
//!
//! POC: this module exposes only the active-state cache + a stub acquire
//! flow. Real CAS happens via a dedicated Ledger RPC introduced in
//! vertical slice expansion (sidecar can't bypass-write `fencing_scopes`).

use chrono::Utc;
use uuid::Uuid;

use crate::domain::{
    error::DomainError,
    state::{ActiveFencing, SidecarState},
};

/// POC bootstrap: assume the operator has pre-provisioned a fencing scope
/// row whose current_epoch this sidecar will use. Production: call a
/// Ledger.AcquireFencingScope RPC (deferred) that performs CAS.
pub fn install_active(state: &SidecarState, scope_id: Uuid, epoch: u64, ttl_secs: i64) {
    let now = Utc::now();
    *state.inner.fencing.write() = Some(ActiveFencing {
        scope_id,
        epoch,
        acquired_at: now,
        renewed_at: now,
        ttl_expires_at: now + chrono::Duration::seconds(ttl_secs),
    });
}

/// Verify the active fencing scope is still within its TTL. Sidecars
/// fail-closed if the lease has expired locally; renewal happens on a
/// separate background task.
pub fn check_active(state: &SidecarState) -> Result<(), DomainError> {
    let f = state.inner.fencing.read();
    let active = f
        .as_ref()
        .ok_or_else(|| DomainError::FencingAcquire("no active fencing scope".into()))?;
    if Utc::now() > active.ttl_expires_at {
        return Err(DomainError::FencingEpochStale(format!(
            "fencing scope {} TTL expired at {}",
            active.scope_id, active.ttl_expires_at
        )));
    }
    Ok(())
}
