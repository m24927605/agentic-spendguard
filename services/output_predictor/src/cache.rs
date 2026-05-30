//! Phase B placeholder — full in-memory cache + SQL lookup arrives in
//! Phase D. The skeleton is constructible (so server.rs compiles) and
//! exposes the lookup signature compute_b will call.
//!
//! Spec ref output-predictor-service-spec-v1alpha1.md §4.2 (cache lookup)
//! + §4.3 (5min in-memory TTL).

use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use sqlx::postgres::PgPool;

/// In-memory cache + SQL lookup shim. Phase D wires the real cache +
/// SQL query. SLICE_06's skeleton holds the SqlPool when present so the
/// server boot path can be exercised end-to-end without a DB.
pub struct OutputDistributionCache {
    /// Read-only pool to canonical_ingest DB. Optional so the server can
    /// boot in "skeleton mode" against a missing DATABASE_URL (demo
    /// fallback). When None, every lookup returns None and Strategy B
    /// collapses to L1 cold-start. Phase D wires the SQL queries that
    /// consume this; SLICE_06 Phase B reserves the field.
    #[allow(dead_code)]
    pub(crate) pool: Option<PgPool>,
    /// In-memory cache TTL (spec §4.3 default 5min). Phase D uses this.
    #[allow(dead_code)]
    pub(crate) ttl: Duration,
    /// Phase D: per-bucket cache map. Placeholder for the skeleton phase.
    #[allow(dead_code)]
    pub(crate) entries: RwLock<()>,
}

impl OutputDistributionCache {
    /// Construct with an optional DB pool + the configured TTL. None
    /// pool path is demo-only; production Helm gate (Phase F) enforces
    /// DATABASE_URL non-empty.
    pub fn new(pool: Option<PgPool>, ttl: Duration) -> Arc<Self> {
        Arc::new(Self {
            pool,
            ttl,
            entries: RwLock::new(()),
        })
    }
}
