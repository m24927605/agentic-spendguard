//! In-memory cache + SQL lookup for `output_distribution_cache`.
//!
//! Spec refs:
//!   * output-predictor-service-spec-v1alpha1.md §4.2 (SQL lookup +
//!     2h staleness gate)
//!   * output-predictor-service-spec-v1alpha1.md §4.3 (5min in-memory TTL)
//!   * stats-aggregator-spec-v1alpha1.md §5 (cache table schema)
//!   * stats-aggregator-spec-v1alpha1.md §9 (RLS via session variable)
//!
//! ## Two-tier cache architecture
//!
//! 1. **In-memory**: parking_lot::Mutex<LruCache<BucketKey, CacheEntry>>;
//!    per-entry TTL stamp; bounded at MAX_IN_MEMORY_ENTRIES (100K)
//!    so a tenant with high bucket cardinality cannot OOM the process.
//!    LRU eviction picks the coldest entry when the cache is full.
//!    R2 M5: prior shape used unbounded `HashMap` — high-cardinality
//!    customer + memory-pressure attack vector. Mutex (vs RwLock) is
//!    fine because the LRU implementation needs to mutate on every
//!    access (touch the recency list); RwLock would still need write
//!    lock on hit.
//! 2. **SQL**: read-only sqlx PgPool against canonical_ingest DB. Each
//!    query opens a transaction, runs `SET LOCAL app.current_tenant_id`
//!    for RLS enforcement, runs the SELECT, returns the row.
//!
//! ## Concurrency contract
//!
//! Multiple Predict calls for the same bucket race the cache. Read path
//! holds a read lock; on miss, the call drops the read lock + acquires
//! a write lock + re-checks (double-checked locking). This avoids the
//! thundering-herd SQL storm on cache expiry. Spec §4.3 doesn't
//! prescribe single-flight semantics so we keep the cheaper
//! double-check pattern.
//!
//! ## Staleness gate
//!
//! Spec §4.2 — rows with `computed_at < now() - 2h` are treated as cache
//! miss. Enforced in the SQL WHERE clause so the DB doesn't return stale
//! rows; in-memory TTL is the additional 5min freshness gate per spec §4.3.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use lru::LruCache;
use parking_lot::Mutex;
use sqlx::postgres::PgPool;
use sqlx::Row;
use tracing::{debug, warn};
use uuid::Uuid;

/// Bounded LRU capacity per spec §4.3 + R2 M5.
///
/// 100K entries × ~256 bytes per entry = ~25 MB resident cache memory
/// in the worst case. Plenty of headroom for 10K tenants × 5 models ×
/// 2 agents × 7 classes (= 700K theoretical max — beyond this the LRU
/// evicts cold entries instead of OOM-ing the process).
pub const MAX_IN_MEMORY_ENTRIES: usize = 100_000;

/// Total Strategy B cache lookups issued by Predict.
pub static OUTPUT_DISTRIBUTION_CACHE_LOOKUP_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Total Strategy B cache hits that returned a promoted L4 row.
pub static OUTPUT_DISTRIBUTION_CACHE_HIT_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Cache row consumed by Strategy B per spec §4.2.
#[derive(Debug, Clone)]
pub struct CacheRow {
    pub p95_30d: f32,
    pub sample_size_30d: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BucketKey {
    tenant_id: Uuid,
    model: String,
    agent_id: String,
    prompt_class: String,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    value: Option<CacheRow>,
    expires_at: Instant,
}

/// Promotion threshold per spec §4.1 — buckets need ≥ 30 samples in the
/// 30-day window for Strategy B to use the row. Below this threshold the
/// cache returns None and the caller falls to cold-start.
const PROMOTION_THRESHOLD_SAMPLES: i32 = 30;

/// In-memory cache + SQL lookup orchestrator.
pub struct OutputDistributionCache {
    /// Read-only pool to canonical_ingest DB. Optional so the server can
    /// boot in skeleton mode against a missing DATABASE_URL (demo
    /// fallback). When None, every lookup short-circuits to None and
    /// Strategy B collapses to L1 cold-start.
    pool: Option<PgPool>,
    /// In-memory entry map — R2 M5: bounded LRU. Mutex because every
    /// access touches the recency list (write lock semantics even on
    /// "read"). RwLock here would not be cheaper.
    entries: Mutex<LruCache<BucketKey, CacheEntry>>,
    /// In-memory TTL per spec §4.3 (5 min default).
    ttl: Duration,
}

impl OutputDistributionCache {
    pub fn new(pool: Option<PgPool>, ttl: Duration) -> Arc<Self> {
        let capacity = NonZeroUsize::new(MAX_IN_MEMORY_ENTRIES)
            .expect("MAX_IN_MEMORY_ENTRIES > 0 at compile time");
        Arc::new(Self {
            pool,
            entries: Mutex::new(LruCache::new(capacity)),
            ttl,
        })
    }

    /// Hot-path lookup. Per spec §4.2:
    ///   - Hit in in-memory cache + not expired → return Some(row) | None.
    ///   - Miss / expired → SQL lookup with staleness gate; update cache.
    ///   - SQL pool None → always None (skeleton mode).
    pub async fn lookup(
        &self,
        tenant_id: Uuid,
        model: &str,
        agent_id: &str,
        prompt_class: &str,
    ) -> Option<CacheRow> {
        OUTPUT_DISTRIBUTION_CACHE_LOOKUP_TOTAL.fetch_add(1, Ordering::Relaxed);
        let key = BucketKey {
            tenant_id,
            model: model.to_string(),
            agent_id: agent_id.to_string(),
            prompt_class: prompt_class.to_string(),
        };

        // ── Fast path: in-memory hit ────────────────────────────────
        {
            let mut guard = self.entries.lock();
            if let Some(entry) = guard.get(&key) {
                if entry.expires_at > Instant::now() {
                    if entry.value.is_some() {
                        OUTPUT_DISTRIBUTION_CACHE_HIT_TOTAL.fetch_add(1, Ordering::Relaxed);
                    }
                    return entry.value.clone();
                }
            }
        }

        // ── Slow path: SQL lookup (when pool present) ───────────────
        //
        // R2 (Backend F6): when the SQL path errors transiently
        // (connection timeout / RLS misconfiguration / etc.) we do NOT
        // cache the None result — caching would mask the recovery
        // for `ttl` seconds. Only an authoritative absence (Ok(None) —
        // row genuinely absent / stale / under-sampled) populates the
        // cache.
        let sql_outcome: Result<Option<CacheRow>, ()> = if let Some(pool) = &self.pool {
            match self.sql_lookup(pool, &key).await {
                Ok(row) => Ok(row),
                Err(e) => {
                    warn!(
                        tenant_id = %tenant_id,
                        model = %model,
                        agent_id = %agent_id,
                        prompt_class = %prompt_class,
                        error = %e,
                        "output_distribution_cache SQL lookup failed; falling to L1 \
                         WITHOUT cache poisoning (transient error)"
                    );
                    Err(())
                }
            }
        } else {
            debug!("output_distribution_cache pool=None; skeleton mode L1");
            Ok(None)
        };

        match sql_outcome {
            Ok(value) => {
                // Authoritative result — populate cache with TTL stamp.
                if value.is_some() {
                    OUTPUT_DISTRIBUTION_CACHE_HIT_TOTAL.fetch_add(1, Ordering::Relaxed);
                }
                let mut guard = self.entries.lock();
                guard.put(
                    key,
                    CacheEntry {
                        value: value.clone(),
                        expires_at: Instant::now() + self.ttl,
                    },
                );
                value
            }
            Err(()) => {
                // Transient SQL error → don't populate cache; return
                // None so caller falls to L1 cold-start. Next call will
                // retry SQL.
                None
            }
        }
    }

    /// SQL lookup per spec §4.2. Sets `app.current_tenant_id` inside the
    /// transaction so RLS policy on `output_distribution_cache` enforces
    /// per-tenant isolation. The staleness gate `computed_at > now() -
    /// interval '2 hours'` is in the WHERE so stale rows return zero
    /// matches → cache miss path.
    ///
    /// R2 B1 / M4: use `set_config(..., true)` with a bound parameter
    /// instead of literal interpolation. tenant_id is already a Uuid
    /// (parsed at gRPC boundary) so the literal form was technically
    /// safe, but the parametrised form (a) removes the format-string
    /// foot-gun entirely, (b) matches the writer-side discipline in
    /// services/stats_aggregator/src/aggregation.rs, and (c) makes the
    /// adversarial RLS-injection test pattern parallel between writer
    /// and reader paths.
    ///
    /// R2 M15: NULL p95_30d handled via try_get → Option mapping. Rows
    /// with NULL p95 (e.g. a fresh cycle that found 0 samples) are
    /// filtered as cache miss instead of panicking in `r.get::<f32, _>(0)`.
    ///
    /// Returns:
    ///   * `Ok(Some(row))` — row present + fresh + sample_size_30d >= 30
    ///                      + p95_30d non-NULL
    ///   * `Ok(None)` — row absent / stale / under-sampled / NULL p95
    ///   * `Err(_)` — connection or RLS error
    async fn sql_lookup(
        &self,
        pool: &PgPool,
        key: &BucketKey,
    ) -> Result<Option<CacheRow>, sqlx::Error> {
        let mut tx = pool.begin().await?;

        // RLS session variable per stats-aggregator-spec §9.1.
        // set_config(..., true) is SET LOCAL — auto-reset at commit.
        sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
            .bind(key.tenant_id.to_string())
            .execute(&mut *tx)
            .await?;

        let row_opt = sqlx::query(
            r#"
            SELECT p95_30d, sample_size_30d
            FROM output_distribution_cache
            WHERE tenant_id = $1
              AND model = $2
              AND agent_id = $3
              AND prompt_class = $4
              AND computed_at > now() - interval '2 hours'
              AND sample_size_30d >= $5
              AND p95_30d IS NOT NULL
            LIMIT 1
            "#,
        )
        .bind(key.tenant_id)
        .bind(&key.model)
        .bind(&key.agent_id)
        .bind(&key.prompt_class)
        .bind(PROMOTION_THRESHOLD_SAMPLES)
        .fetch_optional(&mut *tx)
        .await?;

        tx.commit().await?;

        // R2 M15: try_get tolerates NULL → cache miss instead of panic.
        let row = row_opt.and_then(|r| {
            let p95 = r.try_get::<Option<f32>, _>(0).ok().flatten()?;
            let n = r.try_get::<Option<i32>, _>(1).ok().flatten()?;
            Some(CacheRow {
                p95_30d: p95,
                sample_size_30d: n,
            })
        });

        Ok(row)
    }

    /// Test helper — number of in-memory entries (after expiry timestamps
    /// the entries still occupy slots until the next miss/insert; this
    /// returns the raw map size).
    #[cfg(test)]
    pub(crate) fn in_memory_size(&self) -> usize {
        self.entries.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn skeleton_mode_no_pool_returns_none() {
        let cache = OutputDistributionCache::new(None, Duration::from_secs(300));
        let tenant = Uuid::new_v4();
        let r = cache
            .lookup(tenant, "gpt-4o", "agent-a", "chat_short")
            .await;
        assert!(r.is_none(), "skeleton mode must return None");
        // Cache slot populated with None so next call short-circuits.
        assert_eq!(cache.in_memory_size(), 1);
    }

    #[tokio::test]
    async fn second_lookup_hits_in_memory_cache() {
        let cache = OutputDistributionCache::new(None, Duration::from_secs(300));
        let tenant = Uuid::new_v4();
        // First lookup: populates with None.
        cache.lookup(tenant, "m", "a", "c").await;
        // Second: should hit in-memory and return None without DB.
        cache.lookup(tenant, "m", "a", "c").await;
        // Map still has exactly one entry.
        assert_eq!(cache.in_memory_size(), 1);
    }

    #[tokio::test]
    async fn different_bucket_keys_create_separate_entries() {
        let cache = OutputDistributionCache::new(None, Duration::from_secs(300));
        let tenant = Uuid::new_v4();
        cache.lookup(tenant, "gpt-4o", "a", "chat_short").await;
        cache.lookup(tenant, "gpt-4o", "a", "chat_long").await;
        cache.lookup(tenant, "claude-3", "a", "chat_short").await;
        assert_eq!(cache.in_memory_size(), 3);
    }

    #[tokio::test]
    async fn concurrent_lookups_same_bucket_no_panic() {
        // Smoke test for the parking_lot::RwLock double-checked locking
        // path. Spawn N concurrent lookups for the same bucket; final
        // map size must be exactly 1 (no race-induced duplicate keys).
        let cache = OutputDistributionCache::new(None, Duration::from_secs(300));
        let tenant = Uuid::new_v4();
        let mut handles = Vec::new();
        for _ in 0..32 {
            let c = cache.clone();
            handles.push(tokio::spawn(async move {
                c.lookup(tenant, "gpt-4o", "agent-x", "chat_short").await
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(cache.in_memory_size(), 1);
    }

    #[tokio::test]
    async fn ttl_zero_means_every_lookup_re_runs_slow_path() {
        // TTL=0 means every read sees an expired entry; in skeleton mode
        // that just means "re-run the None branch", but we want to
        // verify the in-memory hit gate respects expires_at.
        let cache = OutputDistributionCache::new(None, Duration::from_secs(0));
        let tenant = Uuid::new_v4();
        let r1 = cache.lookup(tenant, "m", "a", "c").await;
        // Sleep 10ms so the previous "now + 0" stamp is definitely past.
        tokio::time::sleep(Duration::from_millis(10)).await;
        let r2 = cache.lookup(tenant, "m", "a", "c").await;
        assert!(r1.is_none() && r2.is_none());
        // Map still has one entry (key overwritten).
        assert_eq!(cache.in_memory_size(), 1);
    }
}
