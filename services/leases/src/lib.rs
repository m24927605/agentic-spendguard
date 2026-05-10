//! Lease primitive for singleton background workers (Phase 5 S1).
//!
//! Provides a `LeaseManager` trait with two production implementations:
//!   * [`PostgresLease`] — works against any Postgres instance via the
//!     `acquire_lease` / `release_lease` SPs (migration 0021). Used by
//!     compose / non-k8s deployments and by integration tests.
//!   * [`K8sLease`]      — k8s `coordination.k8s.io/Lease` API. Compiled
//!     in by default; only consumes `kube` types when the `k8s` feature
//!     would be activated. For Phase 5 S1 we keep this stub-only with
//!     a clear `unimplemented` failure so consumers explicitly opt out
//!     of k8s mode (set `mode = "postgres"` or `"disabled"`).
//!
//! Mode selection (caller's job — typically from env):
//!   * `Mode::Postgres { pool, ... }` — fully working
//!   * `Mode::K8s { ... }`            — returns a `LeaseError::ModeUnavailable`
//!     today; populated in S5 when chart RBAC + cluster wiring lands
//!   * `Mode::Disabled`               — no leader election; caller is
//!     ALWAYS leader. **Only safe with `replicas = 1`.** The Helm
//!     templates (S5) reject `replicas > 1` when this mode is active.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use thiserror::Error;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseState {
    /// We're the leader. Holds the fencing token + expiry.
    Leader {
        token: Uuid,
        expires_at: DateTime<Utc>,
        transition_count: i64,
    },
    /// Lease held by another instance. Carries observed holder for diagnostics.
    Standby {
        holder_workload_id: String,
        observed_expiry: DateTime<Utc>,
    },
    /// Initial state or transient post-error state.
    Unknown,
}

impl LeaseState {
    pub fn is_leader(&self) -> bool {
        matches!(self, LeaseState::Leader { .. })
    }

    /// Codex round-9 P2: expiry-aware leader check. Returns true only
    /// if we're Leader AND the lease has not yet expired. Use this
    /// before any leader-only side effect (sweep, forward, etc.) so a
    /// stalled renewal task doesn't keep the worker thinking it's
    /// still leader after another pod has taken over.
    pub fn is_leader_now(&self) -> bool {
        match self {
            LeaseState::Leader { expires_at, .. } => *expires_at > Utc::now(),
            _ => false,
        }
    }
}

#[derive(Debug, Error)]
pub enum LeaseError {
    #[error("lease mode unavailable: {0}")]
    ModeUnavailable(String),
    #[error("lease backend error: {0}")]
    Backend(#[from] sqlx::Error),
    #[error("lease lost (token mismatch or row gone)")]
    Lost,
    #[error("lease validation failed: {0}")]
    Invalid(String),
}

#[async_trait]
pub trait LeaseManager: Send + Sync {
    /// Try to acquire (or renew) the lease. Returns Granted variant on
    /// success or Denied on contention.
    async fn try_acquire(&self) -> Result<LeaseAttempt, LeaseError>;

    /// Release the lease the caller currently holds. Idempotent.
    async fn release(&self, token: Uuid) -> Result<(), LeaseError>;

    /// Lease name for diagnostics.
    fn lease_name(&self) -> &str;

    /// Workload id for diagnostics.
    fn workload_id(&self) -> &str;
}

#[derive(Debug, Clone)]
pub struct LeaseAttempt {
    pub state: LeaseState,
    pub event_type: String,
}

#[derive(Debug, Clone)]
pub struct LeaseConfig {
    pub lease_name: String,
    pub workload_id: String,
    pub region: String,
    /// How long each acquire/renew extends the lease.
    pub ttl: Duration,
    /// How often to renew while leader. Must be < ttl.
    pub renew_interval: Duration,
    /// How often to retry acquire while standby.
    pub retry_interval: Duration,
}

impl LeaseConfig {
    pub fn validate(&self) -> Result<(), LeaseError> {
        if self.lease_name.is_empty() {
            return Err(LeaseError::Invalid("lease_name empty".into()));
        }
        if self.workload_id.is_empty() {
            return Err(LeaseError::Invalid("workload_id empty".into()));
        }
        if self.region.is_empty() {
            return Err(LeaseError::Invalid("region empty".into()));
        }
        if self.ttl.is_zero() || self.renew_interval.is_zero()
            || self.retry_interval.is_zero()
        {
            return Err(LeaseError::Invalid("intervals must be > 0".into()));
        }
        if self.renew_interval >= self.ttl {
            return Err(LeaseError::Invalid(format!(
                "renew_interval ({:?}) MUST be < ttl ({:?}) — otherwise the lease expires before we renew",
                self.renew_interval, self.ttl
            )));
        }
        Ok(())
    }
}

// =============================================================================
// Postgres backend
// =============================================================================

pub struct PostgresLease {
    pool: PgPool,
    cfg: LeaseConfig,
}

impl PostgresLease {
    pub fn new(pool: PgPool, cfg: LeaseConfig) -> Result<Self, LeaseError> {
        cfg.validate()?;
        Ok(Self { pool, cfg })
    }
}

#[async_trait]
impl LeaseManager for PostgresLease {
    async fn try_acquire(&self) -> Result<LeaseAttempt, LeaseError> {
        let ttl_secs: i32 = self
            .cfg
            .ttl
            .as_secs()
            .try_into()
            .map_err(|_| LeaseError::Invalid("ttl too large for i32".into()))?;

        let row: (
            bool,
            Option<Uuid>,
            Option<String>,
            Option<DateTime<Utc>>,
            i64,
            String,
        ) = sqlx::query_as(
            "SELECT granted, holder_token, holder_workload_id, expires_at, \
                    transition_count, event_type \
               FROM acquire_lease($1, $2, $3, $4)",
        )
        .bind(&self.cfg.lease_name)
        .bind(&self.cfg.workload_id)
        .bind(&self.cfg.region)
        .bind(ttl_secs)
        .fetch_one(&self.pool)
        .await?;

        let (granted, holder_token, holder_workload_id, expires_at, transition_count, event_type) =
            row;

        if granted {
            let token = holder_token.ok_or_else(|| {
                LeaseError::Invalid("acquire_lease granted without holder_token".into())
            })?;
            let expires = expires_at.ok_or_else(|| {
                LeaseError::Invalid("acquire_lease granted without expires_at".into())
            })?;
            Ok(LeaseAttempt {
                state: LeaseState::Leader {
                    token,
                    expires_at: expires,
                    transition_count,
                },
                event_type,
            })
        } else {
            // Denied path: holder_workload_id should be present.
            let holder = holder_workload_id.unwrap_or_else(|| "<unknown>".into());
            let observed = expires_at.unwrap_or_else(Utc::now);
            Ok(LeaseAttempt {
                state: LeaseState::Standby {
                    holder_workload_id: holder,
                    observed_expiry: observed,
                },
                event_type,
            })
        }
    }

    async fn release(&self, token: Uuid) -> Result<(), LeaseError> {
        let released: (bool,) = sqlx::query_as(
            "SELECT release_lease($1, $2, $3)",
        )
        .bind(&self.cfg.lease_name)
        .bind(&self.cfg.workload_id)
        .bind(token)
        .fetch_one(&self.pool)
        .await?;
        if !released.0 {
            // Caller didn't hold — log but don't error (idempotent).
            debug!(
                lease = %self.cfg.lease_name,
                workload = %self.cfg.workload_id,
                "release_lease was a no-op (not the current holder)"
            );
        }
        Ok(())
    }

    fn lease_name(&self) -> &str {
        &self.cfg.lease_name
    }

    fn workload_id(&self) -> &str {
        &self.cfg.workload_id
    }
}

// =============================================================================
// k8s backend (followup #5)
// =============================================================================
//
// Real `coordination.k8s.io/Lease`-backed leader election. Mirrors the
// PostgresLease semantics: try_acquire returns Granted::Leader when we
// own (or just took) the lease, Standby when another holder is active,
// Unknown on transient errors. release best-effort clears the holder.
//
// Operator must grant the workload's ServiceAccount these verbs in the
// configured namespace:
//   apiGroups: ["coordination.k8s.io"]
//   resources: ["leases"]
//   verbs:     ["get", "create", "patch", "update", "delete"]
//
// The PostgresLease backend stays the default for sites without
// kube-crate connectivity.

pub struct K8sLease {
    pub namespace: String,
    pub lease_name: String,
    pub workload_id: String,
    /// Lease duration in seconds. `is_leader_now()` will reject stale
    /// Leader states past this window even if the watch channel is
    /// still cached.
    pub lease_duration_seconds: i32,
    /// Pre-built kube `Api<Lease>` handle. Constructed once at startup
    /// via `K8sLease::new` (which calls `Client::try_default` against
    /// the in-cluster ServiceAccount); injectable in tests via
    /// `K8sLease::with_api`.
    api: kube::Api<k8s_openapi::api::coordination::v1::Lease>,
}

impl K8sLease {
    /// Construct from in-cluster config (ServiceAccount + namespace).
    pub async fn new(
        namespace: String,
        lease_name: String,
        workload_id: String,
        lease_duration_seconds: i32,
    ) -> Result<Self, LeaseError> {
        let client = kube::Client::try_default().await.map_err(|e| {
            LeaseError::ModeUnavailable(format!(
                "kube client init failed: {e} (no in-cluster ServiceAccount?)"
            ))
        })?;
        let api: kube::Api<k8s_openapi::api::coordination::v1::Lease> =
            kube::Api::namespaced(client, &namespace);
        Ok(Self {
            namespace,
            lease_name,
            workload_id,
            lease_duration_seconds,
            api,
        })
    }

    /// Test/operator hook: build with a pre-configured Api.
    pub fn with_api(
        namespace: String,
        lease_name: String,
        workload_id: String,
        lease_duration_seconds: i32,
        api: kube::Api<k8s_openapi::api::coordination::v1::Lease>,
    ) -> Self {
        Self {
            namespace,
            lease_name,
            workload_id,
            lease_duration_seconds,
            api,
        }
    }
}

#[async_trait]
impl LeaseManager for K8sLease {
    async fn try_acquire(&self) -> Result<LeaseAttempt, LeaseError> {
        use k8s_openapi::api::coordination::v1::{Lease, LeaseSpec};
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::MicroTime;
        use kube::api::{ObjectMeta, PatchParams, PostParams};

        let now = Utc::now();

        // 1) GET — does the Lease exist?
        let existing = self.api.get_opt(&self.lease_name).await.map_err(|e| {
            LeaseError::Invalid(format!("k8s GET lease {}: {e}", self.lease_name))
        })?;

        match existing {
            None => {
                // 2) Absent → CREATE with us as the holder.
                let lease = Lease {
                    metadata: ObjectMeta {
                        name: Some(self.lease_name.clone()),
                        namespace: Some(self.namespace.clone()),
                        ..Default::default()
                    },
                    spec: Some(LeaseSpec {
                        holder_identity: Some(self.workload_id.clone()),
                        lease_duration_seconds: Some(self.lease_duration_seconds),
                        acquire_time: Some(MicroTime(now)),
                        renew_time: Some(MicroTime(now)),
                        lease_transitions: Some(1),
                        ..Default::default()
                    }),
                };
                let created = self
                    .api
                    .create(&PostParams::default(), &lease)
                    .await
                    .map_err(|e| LeaseError::Invalid(format!("k8s CREATE lease: {e}")))?;
                let token = derive_k8s_token(&created);
                let expires = now
                    + chrono::Duration::seconds(self.lease_duration_seconds as i64);
                Ok(LeaseAttempt {
                    state: LeaseState::Leader {
                        token,
                        expires_at: expires,
                        transition_count: 1,
                    },
                    event_type: "acquired".into(),
                })
            }
            Some(lease) => {
                let spec = lease.spec.clone().unwrap_or_default();
                let holder = spec.holder_identity.clone();
                let renew_time = spec.renew_time.clone().map(|MicroTime(t)| t);
                let duration = spec
                    .lease_duration_seconds
                    .unwrap_or(self.lease_duration_seconds);
                let observed_expiry = renew_time
                    .map(|t| t + chrono::Duration::seconds(duration as i64))
                    .unwrap_or(now);
                let prior_transitions = spec.lease_transitions.unwrap_or(0);

                if holder.as_deref() == Some(self.workload_id.as_str()) {
                    // 3) Held by us — PATCH renewTime.
                    let patch = serde_json::json!({
                        "spec": {
                            "renewTime": MicroTime(now),
                        }
                    });
                    let _ = self
                        .api
                        .patch(
                            &self.lease_name,
                            &PatchParams::default(),
                            &kube::api::Patch::Merge(&patch),
                        )
                        .await
                        .map_err(|e| {
                            LeaseError::Invalid(format!("k8s PATCH renewTime: {e}"))
                        })?;
                    let token = derive_k8s_token(&lease);
                    let expires = now
                        + chrono::Duration::seconds(self.lease_duration_seconds as i64);
                    Ok(LeaseAttempt {
                        state: LeaseState::Leader {
                            token,
                            expires_at: expires,
                            transition_count: prior_transitions as i64,
                        },
                        event_type: "renewed".into(),
                    })
                } else if observed_expiry < now {
                    // 4) Held by someone else but expired → take over.
                    let new_transitions = prior_transitions + 1;
                    let patch = serde_json::json!({
                        "spec": {
                            "holderIdentity":   self.workload_id,
                            "acquireTime":      MicroTime(now),
                            "renewTime":        MicroTime(now),
                            "leaseTransitions": new_transitions,
                        }
                    });
                    let patched = self
                        .api
                        .patch(
                            &self.lease_name,
                            &PatchParams::default(),
                            &kube::api::Patch::Merge(&patch),
                        )
                        .await
                        .map_err(|e| {
                            LeaseError::Invalid(format!("k8s PATCH takeover: {e}"))
                        })?;
                    let token = derive_k8s_token(&patched);
                    let expires = now
                        + chrono::Duration::seconds(self.lease_duration_seconds as i64);
                    Ok(LeaseAttempt {
                        state: LeaseState::Leader {
                            token,
                            expires_at: expires,
                            transition_count: new_transitions as i64,
                        },
                        event_type: "transitioned".into(),
                    })
                } else {
                    // 5) Held by another fresh holder → standby.
                    Ok(LeaseAttempt {
                        state: LeaseState::Standby {
                            holder_workload_id: holder.unwrap_or_default(),
                            observed_expiry,
                        },
                        event_type: "denied".into(),
                    })
                }
            }
        }
    }

    async fn release(&self, _token: Uuid) -> Result<(), LeaseError> {
        use kube::api::PatchParams;
        // Best-effort: clear holderIdentity. Other pods will take over
        // via expiry anyway, so failure here is not fatal.
        let patch = serde_json::json!({
            "spec": {
                "holderIdentity": null,
                "renewTime":      null,
            }
        });
        match self
            .api
            .patch(
                &self.lease_name,
                &PatchParams::default(),
                &kube::api::Patch::Merge(&patch),
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(e) => {
                tracing::warn!(
                    err = %e,
                    lease = %self.lease_name,
                    "k8s release best-effort patch failed; relying on TTL takeover"
                );
                Ok(())
            }
        }
    }

    fn lease_name(&self) -> &str {
        &self.lease_name
    }

    fn workload_id(&self) -> &str {
        &self.workload_id
    }
}

/// Derive a stable token for a K8s Lease epoch. PostgresLease uses a
/// random UUID per acquire; for k8s, derive from the resource UID +
/// transition count so the token is unique per leader epoch and any
/// caller storing it (or comparing it) gets the canonical contract.
fn derive_k8s_token(lease: &k8s_openapi::api::coordination::v1::Lease) -> Uuid {
    let uid = lease
        .metadata
        .uid
        .as_deref()
        .unwrap_or("00000000-0000-0000-0000-000000000000");
    let transitions = lease
        .spec
        .as_ref()
        .and_then(|s| s.lease_transitions)
        .unwrap_or(0);
    // Combine uid bytes + transition count via a small fold; we want
    // determinism per (uid, transition), not cryptographic strength.
    let mut bytes = [0u8; 16];
    let uid_bytes = uid.as_bytes();
    for (i, b) in uid_bytes.iter().take(16).enumerate() {
        bytes[i] = *b;
    }
    let t_bytes = (transitions as u32).to_le_bytes();
    bytes[12] ^= t_bytes[0];
    bytes[13] ^= t_bytes[1];
    bytes[14] ^= t_bytes[2];
    bytes[15] ^= t_bytes[3];
    Uuid::from_bytes(bytes)
}

// =============================================================================
// Disabled backend (single-pod escape hatch)
// =============================================================================

/// "No-leader-election" mode: caller is always leader. **Helm chart
/// rejects this mode when `replicas > 1`** (see S5).
pub struct DisabledLease {
    pub lease_name: String,
    pub workload_id: String,
}

#[async_trait]
impl LeaseManager for DisabledLease {
    async fn try_acquire(&self) -> Result<LeaseAttempt, LeaseError> {
        let token = Uuid::nil(); // Sentinel: never matches a real Postgres token.
        let expires_at = Utc::now() + chrono::Duration::seconds(3600);
        Ok(LeaseAttempt {
            state: LeaseState::Leader {
                token,
                expires_at,
                transition_count: 0,
            },
            event_type: "disabled-mode-always-leader".into(),
        })
    }

    async fn release(&self, _token: Uuid) -> Result<(), LeaseError> {
        Ok(())
    }

    fn lease_name(&self) -> &str {
        &self.lease_name
    }

    fn workload_id(&self) -> &str {
        &self.workload_id
    }
}

// =============================================================================
// Lease loop helper
// =============================================================================

/// Spawn a renewal task that:
///   * acquires the lease at startup
///   * publishes state via a `watch::Receiver<LeaseState>`
///   * renews every `cfg.renew_interval` while leader
///   * retries every `cfg.retry_interval` while standby
///   * exits cleanly on `shutdown` signal
///
/// Workers consume the `watch::Receiver` to decide whether to process
/// a batch. The LeaseGuard returned from this function holds the join
/// handle and a channel sender to request shutdown.
pub fn spawn_lease_loop(
    manager: std::sync::Arc<dyn LeaseManager>,
    cfg: LeaseConfig,
) -> LeaseGuard {
    let (state_tx, state_rx) = watch::channel(LeaseState::Unknown);
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

    let lease_name = cfg.lease_name.clone();
    let workload_id = cfg.workload_id.clone();

    let handle: JoinHandle<()> = tokio::spawn(async move {
        loop {
            if *shutdown_rx.borrow() {
                break;
            }
            let attempt = match manager.try_acquire().await {
                Ok(a) => a,
                Err(e) => {
                    warn!(lease = %lease_name, workload = %workload_id, error = ?e, "lease acquire failed");
                    let _ = state_tx.send(LeaseState::Unknown);
                    tokio::select! {
                        _ = tokio::time::sleep(cfg.retry_interval) => continue,
                        changed = shutdown_rx.changed() => {
                            if changed.is_ok() && *shutdown_rx.borrow() { break; }
                        }
                    }
                    continue;
                }
            };

            let next_wait = match &attempt.state {
                LeaseState::Leader { .. } => {
                    info!(
                        lease = %lease_name,
                        workload = %workload_id,
                        event = %attempt.event_type,
                        "lease state = LEADER"
                    );
                    cfg.renew_interval
                }
                LeaseState::Standby {
                    holder_workload_id,
                    ..
                } => {
                    debug!(
                        lease = %lease_name,
                        workload = %workload_id,
                        held_by = %holder_workload_id,
                        "lease state = STANDBY"
                    );
                    cfg.retry_interval
                }
                LeaseState::Unknown => cfg.retry_interval,
            };

            let _ = state_tx.send(attempt.state);

            tokio::select! {
                _ = tokio::time::sleep(next_wait) => {}
                changed = shutdown_rx.changed() => {
                    if changed.is_ok() && *shutdown_rx.borrow() { break; }
                }
            }
        }

        // On shutdown, attempt a graceful release if we hold the lease.
        let last = state_tx.borrow().clone();
        if let LeaseState::Leader { token, .. } = last {
            if let Err(e) = manager.release(token).await {
                warn!(error = ?e, "graceful lease release failed");
            } else {
                info!(lease = %lease_name, workload = %workload_id, "lease released on shutdown");
            }
        }
    });

    LeaseGuard {
        state_rx,
        shutdown_tx,
        handle,
    }
}

pub struct LeaseGuard {
    pub state_rx: watch::Receiver<LeaseState>,
    shutdown_tx: watch::Sender<bool>,
    handle: JoinHandle<()>,
}

impl LeaseGuard {
    /// Convenience: returns `true` if the most recently published state
    /// is `Leader` AND not yet expired (defensive — workers must still
    /// honour `try_acquire` failures to detect mid-operation loss).
    pub fn is_leader(&self) -> bool {
        match &*self.state_rx.borrow() {
            LeaseState::Leader { expires_at, .. } => *expires_at > Utc::now(),
            _ => false,
        }
    }

    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(true);
        if let Err(e) = self.handle.await {
            error!(error = ?e, "lease loop join failed");
        }
    }
}

// =============================================================================
// Unit tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_state_is_leader_only_for_leader() {
        let leader = LeaseState::Leader {
            token: Uuid::nil(),
            expires_at: Utc::now() + chrono::Duration::seconds(60),
            transition_count: 1,
        };
        assert!(leader.is_leader());
        assert!(!LeaseState::Standby {
            holder_workload_id: "other".into(),
            observed_expiry: Utc::now(),
        }
        .is_leader());
        assert!(!LeaseState::Unknown.is_leader());
    }

    /// Codex round-9 P2: is_leader_now() must reject expired Leader.
    #[test]
    fn is_leader_now_rejects_expired_leader() {
        let fresh = LeaseState::Leader {
            token: Uuid::nil(),
            expires_at: Utc::now() + chrono::Duration::seconds(60),
            transition_count: 1,
        };
        let expired = LeaseState::Leader {
            token: Uuid::nil(),
            expires_at: Utc::now() - chrono::Duration::seconds(1),
            transition_count: 1,
        };
        assert!(fresh.is_leader_now(), "fresh leader should be leader-now");
        assert!(!expired.is_leader_now(), "expired leader must NOT be leader-now");
        // Plain is_leader is variant-only and does not check expiry —
        // verifies the new method is genuinely stricter.
        assert!(expired.is_leader());
        assert!(!expired.is_leader_now());
    }

    #[test]
    fn is_leader_now_false_for_standby_and_unknown() {
        assert!(!LeaseState::Standby {
            holder_workload_id: "x".into(),
            observed_expiry: Utc::now() + chrono::Duration::seconds(60),
        }
        .is_leader_now());
        assert!(!LeaseState::Unknown.is_leader_now());
    }

    #[test]
    fn lease_config_validates_renew_lt_ttl() {
        let mut cfg = LeaseConfig {
            lease_name: "x".into(),
            workload_id: "w".into(),
            region: "demo".into(),
            ttl: Duration::from_secs(10),
            renew_interval: Duration::from_secs(3),
            retry_interval: Duration::from_secs(1),
        };
        assert!(cfg.validate().is_ok());
        cfg.renew_interval = Duration::from_secs(11);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn lease_config_rejects_empty_fields() {
        let cfg = LeaseConfig {
            lease_name: "".into(),
            workload_id: "w".into(),
            region: "demo".into(),
            ttl: Duration::from_secs(10),
            renew_interval: Duration::from_secs(3),
            retry_interval: Duration::from_secs(1),
        };
        assert!(cfg.validate().is_err());
    }

    #[tokio::test]
    async fn disabled_lease_always_grants() {
        let m = DisabledLease {
            lease_name: "test".into(),
            workload_id: "w0".into(),
        };
        let attempt = m.try_acquire().await.expect("acquire");
        assert!(matches!(attempt.state, LeaseState::Leader { .. }));
        m.release(Uuid::nil()).await.expect("release");
    }

    /// Followup #5: K8sLease is now a real kube-rs integration. Build
    /// + struct shape compile-checked here. End-to-end leader-election
    /// behaviour requires a kind cluster (verified by operator before
    /// flipping `leaderElection.mode=k8s` in Helm).
    #[test]
    fn k8s_lease_struct_constructs() {
        // We can't easily mock `kube::Api` without a live cluster.
        // This test asserts the struct + helper compile + the
        // derive_k8s_token fold is deterministic.
        use k8s_openapi::api::coordination::v1::{Lease, LeaseSpec};
        use kube::api::ObjectMeta;
        let lease = Lease {
            metadata: ObjectMeta {
                uid: Some("11111111-1111-1111-1111-111111111111".into()),
                ..Default::default()
            },
            spec: Some(LeaseSpec {
                lease_transitions: Some(7),
                ..Default::default()
            }),
        };
        let t1 = derive_k8s_token(&lease);
        let t2 = derive_k8s_token(&lease);
        assert_eq!(t1, t2, "derive_k8s_token must be deterministic per (uid, transition)");
    }

    #[test]
    fn derive_k8s_token_changes_with_transition() {
        use k8s_openapi::api::coordination::v1::{Lease, LeaseSpec};
        use kube::api::ObjectMeta;
        let mk = |t: i32| Lease {
            metadata: ObjectMeta {
                uid: Some("aaaaaaaa-1111-1111-1111-111111111111".into()),
                ..Default::default()
            },
            spec: Some(LeaseSpec {
                lease_transitions: Some(t),
                ..Default::default()
            }),
        };
        let t1 = derive_k8s_token(&mk(1));
        let t2 = derive_k8s_token(&mk(2));
        assert_ne!(t1, t2, "different transitions must yield different tokens");
    }
}
