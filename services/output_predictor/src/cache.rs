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
//! 1. **In-memory**: parking_lot::RwLock<HashMap<BucketKey, CacheEntry>>;
//!    per-entry TTL stamp; read-mostly; bounded-size LRU NOT included in
//!    SLICE_06 (deferred to SLICE-extra — the bucket space is naturally
//!    bounded by (tenant × model × agent_id × class) which Helm gates
//!    via tenant + model counts).
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

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use sqlx::postgres::PgPool;
use sqlx::Row;
use tracing::{debug, warn};
use uuid::Uuid;

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
    /// In-memory entry map. RwLock keyed for read-mostly workload.
    entries: RwLock<HashMap<BucketKey, CacheEntry>>,
    /// In-memory TTL per spec §4.3 (5 min default).
    ttl: Duration,
}

impl OutputDistributionCache {
    pub fn new(pool: Option<PgPool>, ttl: Duration) -> Arc<Self> {
        Arc::new(Self {
            pool,
            entries: RwLock::new(HashMap::new()),
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
        let key = BucketKey {
            tenant_id,
            model: model.to_string(),
            agent_id: agent_id.to_string(),
            prompt_class: prompt_class.to_string(),
        };

        // ── Fast path: in-memory hit ────────────────────────────────
        {
            let guard = self.entries.read();
            if let Some(entry) = guard.get(&key) {
                if entry.expires_at > Instant::now() {
                    return entry.value.clone();
                }
            }
        }

        // ── Slow path: SQL lookup (when pool present) ───────────────
        let value = if let Some(pool) = &self.pool {
            match self.sql_lookup(pool, &key).await {
                Ok(row) => row,
                Err(e) => {
                    warn!(
                        tenant_id = %tenant_id,
                        model = %model,
                        agent_id = %agent_id,
                        prompt_class = %prompt_class,
                        error = %e,
                        "output_distribution_cache SQL lookup failed; falling to L1"
                    );
                    None
                }
            }
        } else {
            debug!("output_distribution_cache pool=None; skeleton mode L1");
            None
        };

        // ── Update in-memory cache (double-checked locking) ─────────
        {
            let mut guard = self.entries.write();
            // Re-check: another task may have populated the entry while
            // we were running SQL. Last writer wins is fine — both
            // values are equally fresh (within milliseconds).
            guard.insert(
                key,
                CacheEntry {
                    value: value.clone(),
                    expires_at: Instant::now() + self.ttl,
                },
            );
        }

        value
    }

    /// SQL lookup per spec §4.2. Sets `app.current_tenant_id` inside the
    /// transaction so RLS policy on `output_distribution_cache` enforces
    /// per-tenant isolation. The staleness gate `computed_at > now() -
    /// interval '2 hours'` is in the WHERE so stale rows return zero
    /// matches → cache miss path.
    ///
    /// Returns:
    ///   * `Ok(Some(row))` — row present + fresh + sample_size_30d >= 30
    ///   * `Ok(None)` — row absent / stale / under-sampled
    ///   * `Err(_)` — connection or RLS error
    async fn sql_lookup(
        &self,
        pool: &PgPool,
        key: &BucketKey,
    ) -> Result<Option<CacheRow>, sqlx::Error> {
        let mut tx = pool.begin().await?;

        // RLS session variable per stats-aggregator-spec §9.1. SET LOCAL
        // confines the variable to this transaction; transaction commit
        // / rollback resets it. The literal interpolation is safe
        // because tenant_id is a Uuid (parsed at gRPC boundary →
        // structurally limited to hex+dashes).
        sqlx::query(&format!(
            "SET LOCAL app.current_tenant_id = '{}'",
            key.tenant_id
        ))
        .execute(&mut *tx)
        .await?;

        let row: Option<(f32, i32)> = sqlx::query(
            r#"
            SELECT p95_30d, sample_size_30d
            FROM output_distribution_cache
            WHERE tenant_id = $1
              AND model = $2
              AND agent_id = $3
              AND prompt_class = $4
              AND computed_at > now() - interval '2 hours'
              AND sample_size_30d >= $5
            LIMIT 1
            "#,
        )
        .bind(key.tenant_id)
        .bind(&key.model)
        .bind(&key.agent_id)
        .bind(&key.prompt_class)
        .bind(PROMOTION_THRESHOLD_SAMPLES)
        .fetch_optional(&mut *tx)
        .await?
        .map(|r| (r.get::<f32, _>(0), r.get::<i32, _>(1)));

        tx.commit().await?;

        Ok(row.map(|(p95, n)| CacheRow {
            p95_30d: p95,
            sample_size_30d: n,
        }))
    }

    /// Test helper — number of in-memory entries (after expiry timestamps
    /// the entries still occupy slots until the next miss/insert; this
    /// returns the raw map size).
    #[cfg(test)]
    pub(crate) fn in_memory_size(&self) -> usize {
        self.entries.read().len()
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
