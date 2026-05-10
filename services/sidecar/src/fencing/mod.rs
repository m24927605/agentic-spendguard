//! Fencing scope acquire / renew (Sidecar §9 + Stage 2 §4.4).
//!
//! Phase 5 S4: real `Ledger.AcquireFencingLease` RPC integration.
//! Sidecar acquires a budget_window or reservation scope at startup
//! BEFORE serving any decision RPC. A background task renews the
//! lease at a fraction of TTL. Lost lease → `state.mark_draining()`
//! so subsequent decisions return `DomainError::Draining`.
//!
//! POC fallback: if `SPENDGUARD_SIDECAR_LEASE_MODE = static`, sidecar
//! installs the seeded `(scope_id, fencing_initial_epoch)` directly
//! without calling the RPC. Used by demo modes that don't have a
//! live ledger seeded with takeover-ready scope rows. Default
//! `lease_mode = rpc` for production-shape behavior.

use chrono::Utc;
use parking_lot::RwLock;
use std::time::Duration;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::{
    clients::ledger::LedgerClient,
    domain::{
        error::DomainError,
        state::{ActiveFencing, SidecarState},
    },
    proto::ledger::v1::{
        acquire_fencing_lease_response::Outcome as LeaseOutcome,
        AcquireFencingLeaseRequest, AcquireFencingLeaseResponse,
    },
};

/// Bootstrap path used by demo modes (and any operator who explicitly
/// opts out of the RPC flow via `lease_mode = static`). Pre-seeds the
/// active fencing state with values from env config.
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

/// Phase 5 S4: real RPC-backed acquire. Caller passes the same
/// (scope_id, tenant_id, workload_id) the sidecar uses elsewhere.
/// On success, installs ActiveFencing into state. On Denied or
/// Error, returns the typed DomainError so the caller can fail-closed
/// at startup (sidecar refuses to come up without a valid lease).
pub async fn rpc_acquire(
    state: &SidecarState,
    ledger: &LedgerClient,
    scope_id: Uuid,
    tenant_id: &str,
    workload_id: &str,
    ttl_seconds: u32,
) -> Result<(), DomainError> {
    let req = AcquireFencingLeaseRequest {
        fencing_scope_id: scope_id.to_string(),
        tenant_id: tenant_id.to_string(),
        workload_instance_id: workload_id.to_string(),
        ttl_seconds,
        force: false,
        audit_event_id: String::new(),
    };
    let resp = ledger.acquire_fencing_lease(req).await?;
    apply_lease_response(&state.inner.fencing, scope_id, workload_id, ttl_seconds, resp)
}

/// Pure response-handling logic split out of `rpc_acquire` so it can be
/// unit-tested without a live tonic channel. Translates the
/// AcquireFencingLeaseResponse oneof into either an `ActiveFencing`
/// installed into the lock, or a typed `DomainError`.
pub(crate) fn apply_lease_response(
    fencing_lock: &RwLock<Option<ActiveFencing>>,
    scope_id: Uuid,
    workload_id: &str,
    ttl_seconds: u32,
    resp: AcquireFencingLeaseResponse,
) -> Result<(), DomainError> {
    match resp.outcome {
        Some(LeaseOutcome::Success(s)) => {
            let now = Utc::now();
            let ttl_expires = s
                .ttl_expires_at
                .as_ref()
                .and_then(|t| chrono::DateTime::<Utc>::from_timestamp(t.seconds, t.nanos as u32))
                .unwrap_or_else(|| now + chrono::Duration::seconds(ttl_seconds as i64));
            *fencing_lock.write() = Some(ActiveFencing {
                scope_id,
                epoch: s.epoch,
                acquired_at: now,
                renewed_at: now,
                ttl_expires_at: ttl_expires,
            });
            info!(
                scope = %scope_id,
                workload = %workload_id,
                epoch = s.epoch,
                action = %s.action,
                ttl_secs = ttl_seconds,
                "fencing lease acquired"
            );
            Ok(())
        }
        Some(LeaseOutcome::Denied(d)) => Err(DomainError::FencingAcquire(format!(
            "fencing lease denied: held by {} at epoch {} until {:?}",
            d.current_holder_instance_id, d.current_epoch, d.current_ttl_expires_at
        ))),
        Some(LeaseOutcome::Error(e)) => Err(DomainError::FencingAcquire(format!(
            "fencing lease error code={} msg={}",
            e.code, e.message
        ))),
        None => Err(DomainError::FencingAcquire(
            "AcquireFencingLease response empty oneof".into(),
        )),
    }
}

/// Background renewal loop. Spawns a tokio task that renews the lease
/// at `renew_interval`, retries on transient errors, and marks the
/// sidecar as draining if renewal fails past `grace_window`.
///
/// The task exits when state is draining OR when the lease loop's
/// shutdown channel fires (via state.is_draining()).
pub fn spawn_renewer(
    state: SidecarState,
    ledger: LedgerClient,
    scope_id: Uuid,
    tenant_id: String,
    workload_id: String,
    ttl_seconds: u32,
    renew_interval: Duration,
    grace_window: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut last_success = Utc::now();
        loop {
            tokio::time::sleep(renew_interval).await;
            if state.is_draining() {
                info!("renewer exiting (sidecar draining)");
                return;
            }
            match rpc_acquire(&state, &ledger, scope_id, &tenant_id, &workload_id, ttl_seconds).await {
                Ok(()) => {
                    last_success = Utc::now();
                }
                Err(e) => {
                    warn!(error = ?e, "fencing renewal failed");
                    let elapsed = Utc::now() - last_success;
                    if elapsed > chrono::Duration::from_std(grace_window).unwrap_or_default() {
                        error!(
                            elapsed_ms = elapsed.num_milliseconds(),
                            "fencing renewal past grace window — entering draining"
                        );
                        state.mark_draining();
                        return;
                    }
                }
            }
        }
    })
}

/// Verify the active fencing scope is still within its TTL. Sidecars
/// fail-closed if the lease has expired locally; renewal happens on a
/// separate background task.
pub fn check_active(state: &SidecarState) -> Result<(), DomainError> {
    check_active_lock(&state.inner.fencing)
}

/// Pure TTL check split out for unit testing without a SidecarState.
pub(crate) fn check_active_lock(
    fencing_lock: &RwLock<Option<ActiveFencing>>,
) -> Result<(), DomainError> {
    let f = fencing_lock.read();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{
        common::v1::Error as ProtoError,
        ledger::v1::{AcquireFencingLeaseDenied, AcquireFencingLeaseSuccess},
    };
    use prost_types::Timestamp;

    fn fresh_lock() -> RwLock<Option<ActiveFencing>> {
        RwLock::new(None)
    }

    #[test]
    fn apply_success_installs_active_fencing_with_provided_epoch() {
        let lock = fresh_lock();
        let scope = Uuid::new_v4();
        let resp = AcquireFencingLeaseResponse {
            outcome: Some(LeaseOutcome::Success(AcquireFencingLeaseSuccess {
                epoch: 42,
                ttl_expires_at: Some(Timestamp {
                    seconds: (Utc::now() + chrono::Duration::seconds(60)).timestamp(),
                    nanos: 0,
                }),
                action: "acquire".into(),
            })),
        };

        apply_lease_response(&lock, scope, "wl-a", 60, resp).expect("ok");

        let installed = lock.read().clone().expect("fencing installed");
        assert_eq!(installed.scope_id, scope);
        assert_eq!(installed.epoch, 42);
        assert!(installed.ttl_expires_at > Utc::now());
    }

    #[test]
    fn apply_success_falls_back_to_local_ttl_when_server_omits_timestamp() {
        // Defensive: SP always sets ttl_expires_at, but if a future server
        // version omits it we still install a usable lease (now + ttl_secs).
        let lock = fresh_lock();
        let scope = Uuid::new_v4();
        let resp = AcquireFencingLeaseResponse {
            outcome: Some(LeaseOutcome::Success(AcquireFencingLeaseSuccess {
                epoch: 7,
                ttl_expires_at: None,
                action: "renew".into(),
            })),
        };

        let before = Utc::now();
        apply_lease_response(&lock, scope, "wl-b", 30, resp).expect("ok");
        let installed = lock.read().clone().expect("fencing installed");

        let expected_min = before + chrono::Duration::seconds(29);
        let expected_max = Utc::now() + chrono::Duration::seconds(31);
        assert!(installed.ttl_expires_at >= expected_min);
        assert!(installed.ttl_expires_at <= expected_max);
    }

    #[test]
    fn apply_denied_returns_fencing_acquire_error_and_leaves_lock_untouched() {
        let lock = fresh_lock();
        let scope = Uuid::new_v4();
        let resp = AcquireFencingLeaseResponse {
            outcome: Some(LeaseOutcome::Denied(AcquireFencingLeaseDenied {
                current_holder_instance_id: "other-pod".into(),
                current_epoch: 5,
                current_ttl_expires_at: Some(Timestamp {
                    seconds: (Utc::now() + chrono::Duration::seconds(120)).timestamp(),
                    nanos: 0,
                }),
            })),
        };

        let err = apply_lease_response(&lock, scope, "wl-c", 60, resp).unwrap_err();
        match err {
            DomainError::FencingAcquire(msg) => {
                assert!(msg.contains("other-pod"), "msg should name holder: {msg}");
                assert!(msg.contains("epoch 5"), "msg should report epoch: {msg}");
            }
            other => panic!("expected FencingAcquire, got {other:?}"),
        }
        assert!(lock.read().is_none(), "denied must not install fencing");
    }

    #[test]
    fn apply_error_returns_fencing_acquire_error() {
        // The Error variant of the response oneof carries the canonical
        // spendguard.common.v1.Error (code is the typed enum, not a free
        // string). Sidecar formats both into the DomainError message so
        // operators see the SP's reason in logs.
        let lock = fresh_lock();
        let scope = Uuid::new_v4();
        let resp = AcquireFencingLeaseResponse {
            outcome: Some(LeaseOutcome::Error(ProtoError {
                code: crate::proto::common::v1::error::Code::TenantDisabled as i32,
                message: "tenant disabled — fencing acquire refused".into(),
                details: Default::default(),
            })),
        };
        let err = apply_lease_response(&lock, scope, "wl-d", 60, resp).unwrap_err();
        match err {
            DomainError::FencingAcquire(msg) => {
                assert!(msg.contains("tenant disabled"), "msg should carry server message: {msg}");
            }
            other => panic!("expected FencingAcquire, got {other:?}"),
        }
    }

    #[test]
    fn apply_empty_oneof_returns_fencing_acquire_error() {
        let lock = fresh_lock();
        let resp = AcquireFencingLeaseResponse { outcome: None };
        let err = apply_lease_response(&lock, Uuid::new_v4(), "wl-e", 60, resp).unwrap_err();
        match err {
            DomainError::FencingAcquire(msg) => assert!(msg.contains("empty oneof")),
            other => panic!("expected FencingAcquire, got {other:?}"),
        }
    }

    #[test]
    fn check_active_returns_acquire_error_when_no_lease_installed() {
        let lock = fresh_lock();
        let err = check_active_lock(&lock).unwrap_err();
        assert!(matches!(err, DomainError::FencingAcquire(_)));
    }

    #[test]
    fn check_active_passes_when_ttl_in_future() {
        let lock = RwLock::new(Some(ActiveFencing {
            scope_id: Uuid::new_v4(),
            epoch: 1,
            acquired_at: Utc::now(),
            renewed_at: Utc::now(),
            ttl_expires_at: Utc::now() + chrono::Duration::seconds(30),
        }));
        check_active_lock(&lock).expect("should pass within TTL");
    }

    #[test]
    fn check_active_returns_epoch_stale_when_ttl_in_past() {
        let lock = RwLock::new(Some(ActiveFencing {
            scope_id: Uuid::new_v4(),
            epoch: 1,
            acquired_at: Utc::now() - chrono::Duration::seconds(120),
            renewed_at: Utc::now() - chrono::Duration::seconds(120),
            ttl_expires_at: Utc::now() - chrono::Duration::seconds(1),
        }));
        let err = check_active_lock(&lock).unwrap_err();
        assert!(matches!(err, DomainError::FencingEpochStale(_)));
    }

    #[test]
    fn epoch_takeover_overwrites_previous_epoch_in_lock() {
        // Renewer/takeover semantics: the lock is rewritten on each
        // successful acquire, so a takeover (epoch+1 from the SP) is
        // visible to the next decision RPC's check_active call.
        let lock = RwLock::new(Some(ActiveFencing {
            scope_id: Uuid::new_v4(),
            epoch: 9,
            acquired_at: Utc::now(),
            renewed_at: Utc::now(),
            ttl_expires_at: Utc::now() + chrono::Duration::seconds(60),
        }));
        let scope = Uuid::new_v4();
        let resp = AcquireFencingLeaseResponse {
            outcome: Some(LeaseOutcome::Success(AcquireFencingLeaseSuccess {
                epoch: 10,
                ttl_expires_at: Some(Timestamp {
                    seconds: (Utc::now() + chrono::Duration::seconds(120)).timestamp(),
                    nanos: 0,
                }),
                action: "promote".into(),
            })),
        };
        apply_lease_response(&lock, scope, "wl-f", 120, resp).expect("ok");
        assert_eq!(lock.read().as_ref().unwrap().epoch, 10);
        assert_eq!(lock.read().as_ref().unwrap().scope_id, scope);
    }
}
