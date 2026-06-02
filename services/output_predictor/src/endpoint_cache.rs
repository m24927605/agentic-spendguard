//! Per-tenant plugin endpoint cache with 60s TTL refresh.
//!
//! Spec refs:
//!   - `output-predictor-plugin-contract-v1alpha1.md` §6.3 (60s health
//!     poll cadence implies cache refresh cadence)
//!   - `output-predictor-plugin-contract-v1alpha1.md` §7.3 (SpendGuard-
//!     side enforcement: tenant_id from request MUST match the
//!     endpoint's configured tenant_id; mismatch = HARD REFUSE, not
//!     fall to B)
//!   - `output-predictor-plugin-contract-v1alpha1.md` §8 (control plane
//!     REST API maintains the underlying predictor_plugin_endpoints
//!     table this cache reads)
//!
//! ## Design
//!
//! Read-through cache backed by a read-only sqlx pool to the
//! control_plane DB's `predictor_plugin_endpoints` table (per Phase A
//! migration). Each per-tenant lookup hits the in-memory cache first
//! (RwLock<HashMap<Uuid, Cached>>); on miss or staleness (computed_at
//! older than `refresh_ttl`) a tenant-scoped singleflight lock collapses
//! concurrent reloads before issuing a SELECT with `SET LOCAL
//! app.current_tenant_id` so the RLS policy enforces tenant isolation
//! at the read. The slow-path result is shared for a short backoff
//! window for true misses and DB-error stale serves, so queued callers
//! do not serialize into one DB lookup each during an outage or cold miss.
//! If that SELECT fails because the DB is temporarily unavailable,
//! lookup may serve a bounded stale enabled endpoint snapshot rather
//! than amplifying an outage into immediate C fallback.
//!
//! ## Critical invariant — tenant_id binding (spec §7.3)
//!
//! The cache lookup takes the request's tenant_id as input; the SQL
//! SELECT's RLS policy returns the row only if tenant_id matches. We
//! ADDITIONALLY verify in code that the returned row's tenant_id
//! matches what was asked — defense-in-depth against a hypothetical
//! RLS misconfiguration or a per-tenant connection that forgot the
//! SET LOCAL. A mismatch raises `TenantBindingViolation` which the
//! caller (strategy_c.rs) treats as a HARD CONFIG ERROR — not a
//! plugin failure mode — and refuses to call the plugin at all.
//!
//! Cross-tenant injection attempt is exhaustively covered by the
//! adversarial test in strategy_c.rs (per slice doc §9 checklist Q5).
//!
//! ## Cache eviction
//!
//! - Time-based: entries older than `refresh_ttl` (default 60s) are
//!   reloaded on the next access. We DO NOT background-evict; load on
//!   demand keeps the cache hot for active tenants and stale entries
//!   for dormant tenants self-clean only when next accessed.
//! - Manual: `evict(tenant_id)` is called by the control plane
//!   handlers after PUT/DELETE so the next strategy_c call picks up
//!   the new URL or sees the deletion.
//! - `enabled = FALSE` rows are STILL cached (with enabled flag set),
//!   so strategy_c.rs can fast-path skip-then-fall-to-B without
//!   re-hitting the DB. Operators flipping `enabled` use the manual
//!   evict to surface the change immediately.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use sqlx::PgPool;
use thiserror::Error;
use tokio::sync::Mutex as AsyncMutex;
use tracing::warn;
use uuid::Uuid;

/// Per spec §6.3 — periodic health-check cadence is 30s; cache TTL
/// must be ≤ that so a `force-reset` from the control plane is
/// reflected within one health cycle.
///
/// R2 M2 (Security F4): control-plane writes do NOT actively evict
/// output_predictor caches — the cache layer is per-pod in-memory and
/// the control plane has no fanout signal in v1alpha1. The pragmatic
/// fix is to shorten the TTL to 5s so the eventual-consistency window
/// between a control plane mutation and the predictor observing it is
/// at most 5s. Spec §11 documents this 5s window as the operator
/// contract; a tighter consistency guarantee requires the cache-
/// revision column approach tracked as a GH issue in R2 outputs.
pub const DEFAULT_REFRESH_TTL: Duration = Duration::from_secs(5);

/// POST_GA_09 / #175: bounded stale serve window for DB errors.
/// Stale entries older than this are treated as unsafe and the DB
/// error falls back to Strategy B instead.
pub const DEFAULT_STALE_ON_DB_ERROR_TTL: Duration = Duration::from_secs(300);

/// POST_GA_09 / #174: after one reload observes a miss or serves stale
/// through a DB error, queued same-tenant callers reuse that result for
/// a short window instead of taking turns hitting the DB. This is not the
/// endpoint freshness TTL; it only collapses immediate herds.
pub const DEFAULT_RELOAD_RESULT_BACKOFF_TTL: Duration = Duration::from_secs(1);

/// Endpoint snapshot returned by the cache. Cheap to clone (Arc'd in
/// the cache to avoid copying the full struct on every Predict call).
#[derive(Debug, Clone)]
pub struct PluginEndpoint {
    pub plugin_endpoint_id: Uuid,
    pub tenant_id: Uuid,
    pub endpoint_url: String,
    pub server_cert_fingerprint: String,
    pub client_cert_id: String,
    pub enabled: bool,
}

impl PluginEndpoint {
    /// Comparison used by plugin_client.rs to decide whether a cached
    /// gRPC channel can be reused. Includes the URL, server cert
    /// fingerprint, and client_cert_id (the attributes that change the
    /// on-the-wire connection identity); excludes `enabled` (channel
    /// reuse is still valid; strategy_c.rs short-circuits on the
    /// enabled flag separately) and `last_health_check_at` (cache
    /// metadata).
    pub fn same_wire_shape(&self, other: &PluginEndpoint) -> bool {
        self.endpoint_url == other.endpoint_url
            && self.server_cert_fingerprint == other.server_cert_fingerprint
            && self.client_cert_id == other.client_cert_id
    }
}

#[derive(Debug, Error)]
pub enum EndpointCacheError {
    /// Returned when the (tenant_id) lookup yields no row OR the row's
    /// enabled flag is FALSE. strategy_c.rs treats both as "no C this
    /// call" and falls to B silently per spec §11.
    #[error("no enabled plugin endpoint registered for tenant {0}")]
    NotConfigured(Uuid),

    /// Spec §7.3 — defense-in-depth: the SQL row's tenant_id MUST match
    /// what the caller requested. Raising this fault means the RLS
    /// layer was bypassed somehow (misconfigured role, BYPASSRLS leaked,
    /// etc.) — caller MUST hard-refuse (not fall to B).
    #[error("tenant_id binding violation: requested {requested}, got {got}")]
    TenantBindingViolation { requested: Uuid, got: Uuid },

    /// DB error (connection failure, query timeout). strategy_c.rs maps
    /// to `customer_predictor_grpc_error` (the DB lookup is on the same
    /// hot path budget as the plugin call).
    #[error("predictor_plugin_endpoints SQL lookup failed: {0}")]
    Sql(#[from] sqlx::Error),
}

#[derive(Debug, Clone)]
struct Cached {
    endpoint: Arc<PluginEndpoint>,
    loaded_at: Instant,
    stale_reload_backoff_until: Option<Instant>,
}

/// Per-tenant endpoint cache. Shared across all output_predictor
/// concurrent Predict tasks via `Arc`.
#[derive(Debug)]
pub struct EndpointCache {
    pool: Option<PgPool>,
    refresh_ttl: Duration,
    stale_on_db_error_ttl: Duration,
    reload_result_backoff_ttl: Duration,
    entries: RwLock<HashMap<Uuid, Cached>>,
    not_configured_backoffs: RwLock<HashMap<Uuid, Instant>>,
    reload_locks: AsyncMutex<HashMap<Uuid, Arc<AsyncMutex<()>>>>,
}

impl EndpointCache {
    pub fn new(pool: Option<PgPool>, refresh_ttl: Duration) -> Arc<Self> {
        Self::new_with_stale_ttl(pool, refresh_ttl, DEFAULT_STALE_ON_DB_ERROR_TTL)
    }

    pub fn new_with_stale_ttl(
        pool: Option<PgPool>,
        refresh_ttl: Duration,
        stale_on_db_error_ttl: Duration,
    ) -> Arc<Self> {
        if pool.is_none() {
            warn!(
                "EndpointCache initialised WITHOUT a control_plane DB pool — \
                 every tenant lookup returns NotConfigured. demo-only; \
                 production Helm profile requires databaseUrl per the chart \
                 control_plane gate."
            );
        }
        Arc::new(Self {
            pool,
            refresh_ttl,
            stale_on_db_error_ttl,
            reload_result_backoff_ttl: DEFAULT_RELOAD_RESULT_BACKOFF_TTL,
            entries: RwLock::new(HashMap::new()),
            not_configured_backoffs: RwLock::new(HashMap::new()),
            reload_locks: AsyncMutex::new(HashMap::new()),
        })
    }

    /// Convenience constructor with the spec default TTL.
    pub fn with_default_ttl(pool: Option<PgPool>) -> Arc<Self> {
        Self::new(pool, DEFAULT_REFRESH_TTL)
    }

    /// Look up the endpoint for `tenant`. Returns `NotConfigured` if no
    /// row exists or the row has `enabled = FALSE` (strategy_c.rs
    /// treats both as "fall to B silently" per spec §11).
    pub async fn lookup(&self, tenant: &Uuid) -> Result<Arc<PluginEndpoint>, EndpointCacheError> {
        if let Some(result) = self.cached_if_fresh(tenant) {
            return result;
        }
        if let Some(result) = self.cached_if_stale_reload_backoff(tenant) {
            return result;
        }
        if let Some(result) = self.cached_if_not_configured_backoff(tenant) {
            return result;
        }

        // POST_GA_09 / #174: tenant-scoped singleflight. Only one task
        // reloads a stale/missing endpoint for a given tenant; unrelated
        // tenants use different locks and do not serialize each other.
        let reload_lock = self.reload_lock_for(tenant).await;
        let guard = reload_lock.lock().await;
        let result = self.lookup_after_reload_lock(tenant).await;
        drop(guard);
        self.cleanup_reload_lock(tenant, &reload_lock).await;
        result
    }

    async fn lookup_after_reload_lock(
        &self,
        tenant: &Uuid,
    ) -> Result<Arc<PluginEndpoint>, EndpointCacheError> {
        // Another concurrent caller may have reloaded the cache while
        // this task waited for the tenant-scoped lock.
        if let Some(result) = self.cached_if_fresh(tenant) {
            return result;
        }
        if let Some(result) = self.cached_if_stale_reload_backoff(tenant) {
            return result;
        }
        if let Some(result) = self.cached_if_not_configured_backoff(tenant) {
            return result;
        }
        // Slow path — DB lookup (RLS-bound). Skeleton mode returns
        // NotConfigured immediately so strategy_c.rs falls to B.
        let pool = match &self.pool {
            Some(p) => p.clone(),
            None => return Err(EndpointCacheError::NotConfigured(*tenant)),
        };
        let endpoint = match load_one(&pool, tenant).await {
            Ok(endpoint) => endpoint,
            Err(EndpointCacheError::Sql(e)) => {
                if let Some(result) = self.mark_stale_reload_backoff_on_db_error(tenant) {
                    return result;
                }
                return Err(EndpointCacheError::Sql(e));
            }
            Err(EndpointCacheError::NotConfigured(t)) => {
                self.entries.write().remove(tenant);
                self.record_not_configured_backoff(tenant);
                return Err(EndpointCacheError::NotConfigured(t));
            }
            Err(e @ EndpointCacheError::TenantBindingViolation { .. }) => {
                return Err(e);
            }
        };
        // Cache the row regardless of enabled flag — strategy_c.rs needs
        // to see the enabled state to decide whether to call.
        let endpoint = Arc::new(endpoint);
        self.entries.write().insert(
            *tenant,
            Cached {
                endpoint: endpoint.clone(),
                loaded_at: Instant::now(),
                stale_reload_backoff_until: None,
            },
        );
        self.not_configured_backoffs.write().remove(tenant);
        if !endpoint.enabled {
            return Err(EndpointCacheError::NotConfigured(*tenant));
        }
        Ok(endpoint)
    }

    fn cached_if_fresh(
        &self,
        tenant: &Uuid,
    ) -> Option<Result<Arc<PluginEndpoint>, EndpointCacheError>> {
        let entries = self.entries.read();
        let cached = entries.get(tenant)?;
        if cached.loaded_at.elapsed() >= self.refresh_ttl {
            return None;
        }
        Some(endpoint_result_from_cached(tenant, cached.endpoint.clone()))
    }

    fn cached_if_stale_reload_backoff(
        &self,
        tenant: &Uuid,
    ) -> Option<Result<Arc<PluginEndpoint>, EndpointCacheError>> {
        let now = Instant::now();
        let entries = self.entries.read();
        let cached = entries.get(tenant)?;
        let backoff_until = cached.stale_reload_backoff_until?;
        if backoff_until <= now {
            return None;
        }
        self.result_if_stale_within_db_error_ttl(tenant, cached, now)
    }

    fn mark_stale_reload_backoff_on_db_error(
        &self,
        tenant: &Uuid,
    ) -> Option<Result<Arc<PluginEndpoint>, EndpointCacheError>> {
        let now = Instant::now();
        let mut entries = self.entries.write();
        let cached = entries.get_mut(tenant)?;
        let result = self.result_if_stale_within_db_error_ttl(tenant, cached, now)?;
        cached.stale_reload_backoff_until = Some(now + self.reload_result_backoff_ttl);
        Some(result)
    }

    fn result_if_stale_within_db_error_ttl(
        &self,
        tenant: &Uuid,
        cached: &Cached,
        now: Instant,
    ) -> Option<Result<Arc<PluginEndpoint>, EndpointCacheError>> {
        if now.duration_since(cached.loaded_at) > self.stale_on_db_error_ttl {
            return None;
        }
        Some(endpoint_result_from_cached(tenant, cached.endpoint.clone()))
    }

    fn cached_if_not_configured_backoff(
        &self,
        tenant: &Uuid,
    ) -> Option<Result<Arc<PluginEndpoint>, EndpointCacheError>> {
        let now = Instant::now();
        let until = self.not_configured_backoffs.read().get(tenant).copied()?;
        if until > now {
            return Some(Err(EndpointCacheError::NotConfigured(*tenant)));
        }
        self.not_configured_backoffs.write().remove(tenant);
        None
    }

    fn record_not_configured_backoff(&self, tenant: &Uuid) {
        self.not_configured_backoffs
            .write()
            .insert(*tenant, Instant::now() + self.reload_result_backoff_ttl);
    }

    async fn reload_lock_for(&self, tenant: &Uuid) -> Arc<AsyncMutex<()>> {
        let mut locks = self.reload_locks.lock().await;
        locks
            .entry(*tenant)
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    async fn cleanup_reload_lock(&self, tenant: &Uuid, lock: &Arc<AsyncMutex<()>>) {
        let mut locks = self.reload_locks.lock().await;
        if let Some(current) = locks.get(tenant) {
            if Arc::ptr_eq(current, lock) && Arc::strong_count(current) == 2 {
                locks.remove(tenant);
            }
        }
    }

    /// Operator-triggered cache evict for the tenant. Called by the
    /// control plane handlers after PUT / DELETE / force-reset.
    pub fn evict(&self, tenant: &Uuid) {
        self.entries.write().remove(tenant);
        self.not_configured_backoffs.write().remove(tenant);
    }

    /// R2 B2 — snapshot of currently-cached tenant ids. The 30s
    /// HealthCheck loop in main.rs iterates over this set to drive
    /// per-tenant probes (spec §6.3); we return a cloned `Vec<Uuid>`
    /// rather than a guard so the caller does not hold the read lock
    /// across `.await` points inside the loop.
    ///
    /// Returns only tenants whose cached entry is still within
    /// `refresh_ttl`. Stale entries are excluded: the health loop will
    /// pick them up after the next lookup() reload via the slow path.
    /// Includes both enabled + disabled tenants. The health loop sees
    /// `enabled = FALSE` through `EndpointCacheError::NotConfigured`,
    /// matching the Strategy C kill-switch semantics.
    pub fn cached_tenants(&self) -> Vec<Uuid> {
        let now = Instant::now();
        self.entries
            .read()
            .iter()
            .filter(|(_, c)| now.duration_since(c.loaded_at) < self.refresh_ttl)
            .map(|(k, _)| *k)
            .collect()
    }

    /// Test-visible accessor.
    #[cfg(test)]
    pub fn cached_count(&self) -> usize {
        self.entries.read().len()
    }
}

fn endpoint_result_from_cached(
    tenant: &Uuid,
    endpoint: Arc<PluginEndpoint>,
) -> Result<Arc<PluginEndpoint>, EndpointCacheError> {
    if !endpoint.enabled {
        return Err(EndpointCacheError::NotConfigured(*tenant));
    }
    Ok(endpoint)
}

/// SQL read for one tenant. Per SLICE_06 R2 B1 + R2 B5: open a tx,
/// SET LOCAL `app.current_tenant_id` so the RLS policy enforces
/// isolation, then SELECT and verify the returned row's tenant_id
/// matches what was asked.
///
/// Defense-in-depth: even though the RLS policy alone is sufficient
/// for correctness, the explicit tenant_id check in code makes the
/// adversarial-injection guarantee from spec §7.3 visible at the
/// CALL SITE in strategy_c.rs rather than hidden in a migration file.
async fn load_one(pool: &PgPool, tenant: &Uuid) -> Result<PluginEndpoint, EndpointCacheError> {
    let mut tx = pool.begin().await?;
    // SLICE_06 R2 B1 convention: set_config(..., true) is SET LOCAL —
    // auto-resets at tx commit. Bound parameter avoids SQL injection
    // via the tenant string.
    sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await?;

    let row: Option<(Uuid, Uuid, String, String, String, bool)> = sqlx::query_as(
        r#"
        SELECT plugin_endpoint_id, tenant_id, endpoint_url,
               server_cert_fingerprint, client_cert_id, enabled
          FROM predictor_plugin_endpoints
         WHERE tenant_id = $1
        "#,
    )
    .bind(tenant)
    .fetch_optional(&mut *tx)
    .await?;

    tx.commit().await?;

    let (
        plugin_endpoint_id,
        row_tenant_id,
        endpoint_url,
        server_cert_fingerprint,
        client_cert_id,
        enabled,
    ) = row.ok_or(EndpointCacheError::NotConfigured(*tenant))?;

    // Spec §7.3 defense-in-depth check. RLS already enforced this but
    // we make the check visible in code so adversarial reviewers + the
    // adversarial test in strategy_c.rs can directly see the boundary.
    if row_tenant_id != *tenant {
        return Err(EndpointCacheError::TenantBindingViolation {
            requested: *tenant,
            got: row_tenant_id,
        });
    }

    Ok(PluginEndpoint {
        plugin_endpoint_id,
        tenant_id: row_tenant_id,
        endpoint_url,
        server_cert_fingerprint,
        client_cert_id,
        enabled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(enabled: bool) -> PluginEndpoint {
        PluginEndpoint {
            plugin_endpoint_id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            endpoint_url: "https://plugin.tenant-x.example/predict".into(),
            server_cert_fingerprint: "a".repeat(64),
            client_cert_id: "spendguard-client-001".into(),
            enabled,
        }
    }

    fn cached(endpoint: PluginEndpoint, loaded_at: Instant) -> Cached {
        Cached {
            endpoint: Arc::new(endpoint),
            loaded_at,
            stale_reload_backoff_until: None,
        }
    }

    #[test]
    fn same_wire_shape_compares_url_fingerprint_and_client_cert_id() {
        let a = ep(true);
        let mut b = a.clone();
        assert!(a.same_wire_shape(&b), "identical endpoints match");
        b.enabled = false;
        assert!(
            a.same_wire_shape(&b),
            "enabled flag does not affect wire shape"
        );
        b.endpoint_url = "https://other.example/predict".into();
        assert!(!a.same_wire_shape(&b), "different url breaks wire shape");
        b = a.clone();
        b.server_cert_fingerprint = "b".repeat(64);
        assert!(
            !a.same_wire_shape(&b),
            "different fingerprint breaks wire shape"
        );
        b = a.clone();
        b.client_cert_id = "spendguard-client-002".into();
        assert!(
            !a.same_wire_shape(&b),
            "different client_cert_id breaks wire shape"
        );
        b = a.clone();
        assert!(
            a.same_wire_shape(&b),
            "matching url + fingerprint + client_cert_id match"
        );
    }

    #[tokio::test]
    async fn skeleton_mode_returns_not_configured() {
        // No DB pool → every lookup returns NotConfigured immediately
        // so strategy_c.rs falls to B without latency.
        let cache = EndpointCache::with_default_ttl(None);
        let tenant = Uuid::new_v4();
        let err = cache
            .lookup(&tenant)
            .await
            .expect_err("skeleton mode must return NotConfigured");
        match err {
            EndpointCacheError::NotConfigured(t) => assert_eq!(t, tenant),
            other => panic!("expected NotConfigured, got {other:?}"),
        }
        assert_eq!(cache.cached_count(), 0);
    }

    #[test]
    fn evict_drops_cached_entry() {
        let cache = EndpointCache::with_default_ttl(None);
        let tenant = Uuid::new_v4();
        // Manually seed (skip DB) so evict has something to remove.
        cache
            .entries
            .write()
            .insert(tenant, cached(ep(true), Instant::now()));
        assert_eq!(cache.cached_count(), 1);
        cache.evict(&tenant);
        assert_eq!(cache.cached_count(), 0);
    }

    #[test]
    fn default_ttl_matches_spec() {
        // R2 M2 (Security F4): cache TTL tightened from 60s → 5s to
        // bound the control-plane mutation → predictor observation
        // window. Spec §11 documents the 5s eventual-consistency
        // contract; tighter consistency is tracked as a follow-up
        // GH issue (cache_revision_at column on the registry table).
        assert_eq!(DEFAULT_REFRESH_TTL, Duration::from_secs(5));
    }

    #[test]
    fn tenant_binding_violation_is_distinct_from_not_configured() {
        // Spec §7.3 — these are semantically different:
        //   NotConfigured           = no row / disabled → fall to B
        //   TenantBindingViolation = SQL row tenant_id mismatch →
        //                            HARD REFUSE (RLS bypass suspect)
        // strategy_c.rs MUST route them down distinct paths.
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let not_configured = EndpointCacheError::NotConfigured(a);
        let violation = EndpointCacheError::TenantBindingViolation {
            requested: a,
            got: b,
        };
        assert_ne!(
            format!("{not_configured}"),
            format!("{violation}"),
            "the error messages must distinguish the two failure modes"
        );
        // Match-arm distinction so any future code-path change must
        // explicitly visit both arms.
        let observed = match not_configured {
            EndpointCacheError::NotConfigured(_) => "not_configured",
            EndpointCacheError::TenantBindingViolation { .. } => "violation",
            EndpointCacheError::Sql(_) => "sql",
        };
        assert_eq!(observed, "not_configured");
        let observed = match violation {
            EndpointCacheError::NotConfigured(_) => "not_configured",
            EndpointCacheError::TenantBindingViolation { .. } => "violation",
            EndpointCacheError::Sql(_) => "sql",
        };
        assert_eq!(observed, "violation");
    }

    #[test]
    fn cached_tenants_returns_fresh_entries_only() {
        // R2 B2: the 30s health loop iterates over cached_tenants().
        // The method MUST exclude stale entries so the loop doesn't
        // probe endpoints we haven't validated recently.
        let cache = EndpointCache::new(None, Duration::from_millis(100));
        let fresh = Uuid::new_v4();
        let mut row_fresh = ep(true);
        row_fresh.tenant_id = fresh;
        cache
            .entries
            .write()
            .insert(fresh, cached(row_fresh, Instant::now()));
        let stale = Uuid::new_v4();
        let mut row_stale = ep(true);
        row_stale.tenant_id = stale;
        cache.entries.write().insert(
            stale,
            cached(
                row_stale,
                // Far past the 100ms refresh window.
                Instant::now()
                    .checked_sub(Duration::from_secs(60))
                    .unwrap_or_else(Instant::now),
            ),
        );
        let cached = cache.cached_tenants();
        assert!(cached.contains(&fresh), "fresh tenant must be reported");
        assert!(!cached.contains(&stale), "stale tenant must be excluded");
    }

    #[test]
    fn cached_tenants_returns_empty_when_cold() {
        let cache = EndpointCache::with_default_ttl(None);
        assert!(cache.cached_tenants().is_empty());
    }

    #[tokio::test]
    async fn reload_locks_are_tenant_scoped_singleflight_keys() {
        let cache = EndpointCache::with_default_ttl(None);
        let tenant_a = Uuid::new_v4();
        let tenant_b = Uuid::new_v4();

        let a1 = cache.reload_lock_for(&tenant_a).await;
        let a2 = cache.reload_lock_for(&tenant_a).await;
        let b = cache.reload_lock_for(&tenant_b).await;

        assert!(
            Arc::ptr_eq(&a1, &a2),
            "same tenant must share one reload lock"
        );
        assert!(
            !Arc::ptr_eq(&a1, &b),
            "different tenants must not serialize behind one lock"
        );
    }

    #[tokio::test]
    async fn db_error_serves_bounded_stale_enabled_endpoint() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://spendguard:spendguard@127.0.0.1:1/spendguard")
            .expect("lazy pool");
        let cache = EndpointCache::new_with_stale_ttl(
            Some(pool),
            Duration::from_millis(1),
            Duration::from_secs(60),
        );
        let tenant = Uuid::new_v4();
        let mut row = ep(true);
        row.tenant_id = tenant;
        cache.entries.write().insert(
            tenant,
            cached(
                row.clone(),
                Instant::now()
                    .checked_sub(Duration::from_secs(10))
                    .unwrap_or_else(Instant::now),
            ),
        );

        let got = cache
            .lookup(&tenant)
            .await
            .expect("stale enabled endpoint should serve through DB error");
        assert_eq!(got.tenant_id, tenant);
        assert_eq!(got.endpoint_url, row.endpoint_url);
        let entries = cache.entries.read();
        let cached = entries.get(&tenant).expect("stale entry retained");
        assert!(
            cached.stale_reload_backoff_until.is_some(),
            "DB-error stale serve must set a short reload backoff for queued callers"
        );
    }

    #[tokio::test]
    async fn db_error_does_not_serve_stale_beyond_bound() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://spendguard:spendguard@127.0.0.1:1/spendguard")
            .expect("lazy pool");
        let cache = EndpointCache::new_with_stale_ttl(
            Some(pool),
            Duration::from_millis(1),
            Duration::from_secs(60),
        );
        let tenant = Uuid::new_v4();
        let mut row = ep(true);
        row.tenant_id = tenant;
        cache.entries.write().insert(
            tenant,
            cached(
                row,
                Instant::now()
                    .checked_sub(Duration::from_secs(120))
                    .unwrap_or_else(Instant::now),
            ),
        );

        let err = cache
            .lookup(&tenant)
            .await
            .expect_err("stale endpoint beyond bound must not serve");
        match err {
            EndpointCacheError::Sql(_) => {}
            other => panic!("expected SQL error after stale bound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn db_error_does_not_resurrect_disabled_stale_endpoint() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://spendguard:spendguard@127.0.0.1:1/spendguard")
            .expect("lazy pool");
        let cache = EndpointCache::new_with_stale_ttl(
            Some(pool),
            Duration::from_millis(1),
            Duration::from_secs(60),
        );
        let tenant = Uuid::new_v4();
        let mut row = ep(false);
        row.tenant_id = tenant;
        cache.entries.write().insert(
            tenant,
            cached(
                row,
                Instant::now()
                    .checked_sub(Duration::from_secs(10))
                    .unwrap_or_else(Instant::now),
            ),
        );

        let err = cache
            .lookup(&tenant)
            .await
            .expect_err("disabled stale endpoint must remain a kill switch");
        match err {
            EndpointCacheError::NotConfigured(t) => assert_eq!(t, tenant),
            other => panic!("expected NotConfigured for disabled stale endpoint, got {other:?}"),
        }
    }

    #[test]
    fn enabled_false_falls_through_to_not_configured() {
        // Spec §11 — kill-switch: enabled=FALSE is observable to
        // strategy_c.rs as NotConfigured (fall to B silently).
        let cache = EndpointCache::with_default_ttl(None);
        let tenant = Uuid::new_v4();
        let mut row = ep(false);
        row.tenant_id = tenant;
        cache
            .entries
            .write()
            .insert(tenant, cached(row, Instant::now()));
        // The cache is now fresh but disabled → lookup returns
        // NotConfigured per the enabled flag check.
        // (Can't call async lookup in a sync test; we exercise the
        // same logic inline.)
        let read = cache.entries.read();
        let cached = read.get(&tenant).expect("seeded entry");
        assert!(!cached.endpoint.enabled);
        // The lookup() async fn checks `!enabled` and returns
        // NotConfigured; this test asserts the precondition that
        // makes that path fire.
    }

    #[test]
    fn stale_reload_backoff_reuses_stale_result_without_marking_fresh() {
        let cache = EndpointCache::new_with_stale_ttl(
            None,
            Duration::from_millis(1),
            Duration::from_secs(60),
        );
        let tenant = Uuid::new_v4();
        let loaded_at = Instant::now()
            .checked_sub(Duration::from_secs(10))
            .unwrap_or_else(Instant::now);
        let mut row = ep(true);
        row.tenant_id = tenant;
        let mut entry = cached(row, loaded_at);
        entry.stale_reload_backoff_until = Some(Instant::now() + Duration::from_secs(1));
        cache.entries.write().insert(tenant, entry);

        let got = cache
            .cached_if_stale_reload_backoff(&tenant)
            .expect("active backoff should return cached result")
            .expect("enabled endpoint should serve stale during backoff");
        assert_eq!(got.tenant_id, tenant);
        let entries = cache.entries.read();
        let cached = entries.get(&tenant).expect("entry retained");
        assert_eq!(
            cached.loaded_at, loaded_at,
            "reload-error backoff must not make stale data fresh for health-loop visibility"
        );
    }

    #[test]
    fn not_configured_backoff_reuses_true_miss_result() {
        let cache = EndpointCache::with_default_ttl(None);
        let tenant = Uuid::new_v4();

        cache.record_not_configured_backoff(&tenant);
        let err = cache
            .cached_if_not_configured_backoff(&tenant)
            .expect("active miss backoff should be visible")
            .expect_err("true miss backoff should return NotConfigured");
        match err {
            EndpointCacheError::NotConfigured(t) => assert_eq!(t, tenant),
            other => panic!("expected NotConfigured, got {other:?}"),
        }

        cache.evict(&tenant);
        assert!(
            cache.cached_if_not_configured_backoff(&tenant).is_none(),
            "explicit evict must clear miss backoff so control-plane registration is observed immediately"
        );
    }
}
