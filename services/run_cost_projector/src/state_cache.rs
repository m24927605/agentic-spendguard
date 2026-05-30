//! RunState cache with bounded LRU eviction + TTL expiry.
//!
//! Spec ref `run-cost-projector-spec-v1alpha1.md` §7.
//!
//! ## Concurrency model
//!
//! * Outer `RwLock<LruCache>`: synchronization for inserts/lookups/eviction.
//!   parking_lot::RwLock (poison-free, faster than std for read-mostly).
//! * Inner `Mutex<RunState>`: per-run serialization so concurrent Project
//!   calls for the same run_id don't corrupt step / cost state. Cheap
//!   sequential lock (only contended within a single run, which by
//!   definition is single-threaded user activity).
//!
//! The LRU + TTL combination is bounded:
//!   * `cap` entries hard cap → LRU evicts least-recently-used on insert.
//!   * `ttl` controls `last_activity_at` → on lookup we check if the entry
//!     expired; if so we evict + report a miss.
//!
//! Spec §0.2 endurance: 10K concurrent runs without memory leak. Each
//! RunState ~ 200 bytes + per_step_costs Vec — total ~ 200 KB-1 MB worst
//! case. Well under the 256 MiB Helm resource ceiling.
//!
//! ## Per-run mutex vs single global mutex
//!
//! A single global lock would serialize ALL Project RPCs across tenants,
//! defeating the 5ms p99 SLO under load. Per-run mutex preserves throughput
//! while preventing the "two concurrent Project for same run_id" corruption
//! risk (e.g. retry-during-flight from sidecar).

use std::sync::Arc;
use std::time::Instant;

use lru::LruCache;
use parking_lot::{Mutex, RwLock};
use uuid::Uuid;

/// Per-run in-memory state. All cost values are atomic-micros int64.
#[derive(Debug, Clone)]
pub struct RunState {
    pub tenant_id: Uuid,
    pub run_id: Uuid,
    pub agent_id: String,
    pub model: String,

    /// Decisions seen so far for this run (i64 to mirror the wire-level
    /// `run_steps_completed_so_far` column per audit-chain-extension §2.2
    /// round-4 fix M1 — int32 → int64).
    pub steps_completed: i64,

    /// Sum of all reserved per-step costs for prior calls in this run
    /// (i64 atomic-micros). Updated AFTER Project returns; the current
    /// call's reservation is added by the layering computation.
    pub cumulative_cost_atomic: i64,

    /// History of per-step reservations (ordered by step). Used by
    /// Signal 2 drift detection (compare ratio of last 3 step costs).
    /// Capped at 256 entries (drops oldest) to bound memory per run.
    pub per_step_costs: Vec<i64>,

    /// Last projection's predicted_remaining_cost for drift comparison.
    pub last_predicted_remaining_cost: Option<i64>,

    /// Consecutive-step drift counter (spec §4.2 threshold: 3 consecutive
    /// > 2σ ratio shifts).
    pub drift_consecutive_count: u32,

    /// Signal 3 hint (planned_steps_hint from with_run_plan decorator).
    /// Latched on first Project that carries a non-zero hint; subsequent
    /// hints are ignored to avoid client-side mutation racing (spec §5.2).
    pub signal3_hint_planned_steps: Option<i32>,

    /// Wall-clock instant of last Project / recovery write — used for TTL eviction.
    pub last_activity_at: Instant,

    /// Instant of first Project call (for diagnostics / lifetime metrics).
    pub started_at: Instant,
}

impl RunState {
    pub fn new(tenant_id: Uuid, run_id: Uuid, agent_id: String, model: String) -> Self {
        let now = Instant::now();
        Self {
            tenant_id,
            run_id,
            agent_id,
            model,
            steps_completed: 0,
            cumulative_cost_atomic: 0,
            per_step_costs: Vec::new(),
            last_predicted_remaining_cost: None,
            drift_consecutive_count: 0,
            signal3_hint_planned_steps: None,
            last_activity_at: now,
            started_at: now,
        }
    }

    /// Append per-step cost; cap history at 256 to bound RunState size.
    /// Drift detection only inspects the last 3 entries so older history
    /// is never consulted; the cap is a memory-safety bound, not a
    /// semantic limit.
    pub fn record_step(&mut self, step_cost_atomic: i64) {
        const HISTORY_CAP: usize = 256;
        self.per_step_costs.push(step_cost_atomic);
        if self.per_step_costs.len() > HISTORY_CAP {
            let overflow = self.per_step_costs.len() - HISTORY_CAP;
            self.per_step_costs.drain(0..overflow);
        }
        self.cumulative_cost_atomic = self.cumulative_cost_atomic.saturating_add(step_cost_atomic);
        self.steps_completed += 1;
        self.last_activity_at = Instant::now();
    }
}

/// Bounded, TTL-aware LRU cache of `(tenant_id, run_id) -> Arc<Mutex<RunState>>`.
///
/// `RunStateKey` is the composite — including tenant_id prevents cross-tenant
/// reuse (defense-in-depth on top of the gRPC-boundary tenant assertion).
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct RunStateKey {
    pub tenant_id: Uuid,
    pub run_id: Uuid,
}

pub struct RunStateCache {
    inner: RwLock<LruCache<RunStateKey, Arc<Mutex<RunState>>>>,
    ttl: std::time::Duration,
}

impl RunStateCache {
    pub fn new(capacity: usize, ttl: std::time::Duration) -> Self {
        let cap = std::num::NonZeroUsize::new(capacity.max(1)).expect("capacity > 0");
        Self {
            inner: RwLock::new(LruCache::new(cap)),
            ttl,
        }
    }

    /// Look up the run state. Returns `None` when:
    ///   * The key is not in cache (cold miss), OR
    ///   * The entry's `last_activity_at + ttl <= now()` (TTL expired —
    ///     entry evicted by this call as a side effect).
    pub fn get(&self, key: &RunStateKey) -> Option<Arc<Mutex<RunState>>> {
        // Read path: lock the outer cache, fetch the Arc, then atomically
        // probe TTL inside the inner mutex. The LRU `get` updates recency.
        let mut cache = self.inner.write();
        let state_arc = cache.get(key).cloned()?;
        // Probe TTL with the inner mutex held briefly (read-only of
        // last_activity_at field).
        let expired = {
            let st = state_arc.lock();
            st.last_activity_at.elapsed() >= self.ttl
        };
        if expired {
            // Evict; treat as miss.
            cache.pop(key);
            None
        } else {
            Some(state_arc)
        }
    }

    /// Insert or replace an entry. LRU eviction is automatic when at cap.
    pub fn insert(&self, key: RunStateKey, state: RunState) -> Arc<Mutex<RunState>> {
        let arc = Arc::new(Mutex::new(state));
        self.inner.write().put(key, arc.clone());
        arc
    }

    /// Remove an entry if present. Returns true if the entry existed.
    /// Idempotent: calling on a non-existent key is a no-op.
    pub fn remove(&self, key: &RunStateKey) -> bool {
        self.inner.write().pop(key).is_some()
    }

    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().is_empty()
    }

    /// Test-only helper: peek without recency update.
    #[cfg(test)]
    pub fn peek(&self, key: &RunStateKey) -> Option<Arc<Mutex<RunState>>> {
        self.inner.read().peek(key).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    fn make_key(tag: u8) -> RunStateKey {
        // Build deterministic UUIDs from a single byte for test legibility.
        let mut bytes = [0u8; 16];
        bytes[0] = tag;
        let id = Uuid::from_bytes(bytes);
        RunStateKey {
            tenant_id: id,
            run_id: id,
        }
    }

    fn make_state(tag: u8) -> RunState {
        let key = make_key(tag);
        RunState::new(key.tenant_id, key.run_id, format!("agent-{tag}"), "m".into())
    }

    #[test]
    fn insert_and_get_round_trip() {
        let cache = RunStateCache::new(4, Duration::from_secs(60));
        let key = make_key(1);
        cache.insert(key.clone(), make_state(1));
        let arc = cache.get(&key).expect("hit");
        assert_eq!(arc.lock().agent_id, "agent-1");
    }

    #[test]
    fn lru_evicts_at_capacity() {
        let cache = RunStateCache::new(2, Duration::from_secs(60));
        for tag in 1u8..=3 {
            cache.insert(make_key(tag), make_state(tag));
        }
        // Three inserts into a cap-2 cache → first one (tag=1) evicted.
        assert_eq!(cache.len(), 2);
        assert!(cache.get(&make_key(1)).is_none(), "tag=1 should be evicted");
        assert!(cache.get(&make_key(2)).is_some());
        assert!(cache.get(&make_key(3)).is_some());
    }

    #[test]
    fn ttl_expires_entry_on_lookup() {
        let cache = RunStateCache::new(4, Duration::from_millis(50));
        let key = make_key(1);
        cache.insert(key.clone(), make_state(1));
        assert!(cache.get(&key).is_some(), "fresh entry hits");
        // Sleep past TTL — get must evict + miss.
        thread::sleep(Duration::from_millis(75));
        assert!(cache.get(&key).is_none(), "expired entry should miss");
        // Side-effect eviction confirmed by len.
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn remove_is_idempotent() {
        let cache = RunStateCache::new(4, Duration::from_secs(60));
        let key = make_key(1);
        assert!(!cache.remove(&key), "missing key returns false");
        cache.insert(key.clone(), make_state(1));
        assert!(cache.remove(&key), "existing key returns true");
        assert!(!cache.remove(&key), "second remove returns false (idempotent)");
    }

    #[test]
    fn record_step_history_caps_at_256() {
        let mut st = make_state(1);
        for i in 0..300 {
            st.record_step(i as i64);
        }
        assert_eq!(st.per_step_costs.len(), 256);
        // Verify the oldest entries dropped (0..44 should be gone).
        assert_eq!(st.per_step_costs[0], 44);
        assert_eq!(st.per_step_costs[255], 299);
        // Cumulative sum unaffected by capping — record_step always adds.
        let expected_sum: i64 = (0..300).sum();
        assert_eq!(st.cumulative_cost_atomic, expected_sum);
        assert_eq!(st.steps_completed, 300);
    }

    #[test]
    fn per_run_mutex_serializes_concurrent_updates() {
        // Spec invariant: two concurrent Project calls for the same run_id
        // must serialize so cumulative_cost_atomic is correct.
        use std::sync::Arc as StdArc;

        let cache = StdArc::new(RunStateCache::new(4, Duration::from_secs(60)));
        let key = make_key(1);
        cache.insert(key.clone(), make_state(1));

        let mut handles = Vec::new();
        for _ in 0..16 {
            let cache_clone = cache.clone();
            let key_clone = key.clone();
            handles.push(thread::spawn(move || {
                let arc = cache_clone.get(&key_clone).expect("hit");
                let mut st = arc.lock();
                st.record_step(100);
            }));
        }
        for h in handles {
            h.join().expect("thread");
        }
        let final_state = cache.get(&key).expect("still present");
        let st = final_state.lock();
        // 16 threads × 100 micros each = 1600 cumulative. If the per-run
        // mutex were broken we'd see torn writes < 1600.
        assert_eq!(st.cumulative_cost_atomic, 1600);
        assert_eq!(st.steps_completed, 16);
    }

    #[test]
    fn cache_key_includes_tenant_id() {
        // Two different tenants with the same run_id MUST map to different
        // cache slots (cross-tenant defense-in-depth per spec §7.4 +
        // SLICE_05 R2 B5 tenant_id parsing convention).
        let cache = RunStateCache::new(4, Duration::from_secs(60));
        let mut bytes_a = [0u8; 16];
        bytes_a[0] = 0xAA;
        let mut bytes_b = [0u8; 16];
        bytes_b[0] = 0xBB;
        let mut bytes_r = [0u8; 16];
        bytes_r[15] = 0xFF;

        let key_a = RunStateKey {
            tenant_id: Uuid::from_bytes(bytes_a),
            run_id: Uuid::from_bytes(bytes_r),
        };
        let key_b = RunStateKey {
            tenant_id: Uuid::from_bytes(bytes_b),
            run_id: Uuid::from_bytes(bytes_r),
        };
        let mut state_a = RunState::new(key_a.tenant_id, key_a.run_id, "a".into(), "m".into());
        state_a.cumulative_cost_atomic = 111;
        let mut state_b = RunState::new(key_b.tenant_id, key_b.run_id, "b".into(), "m".into());
        state_b.cumulative_cost_atomic = 222;

        cache.insert(key_a.clone(), state_a);
        cache.insert(key_b.clone(), state_b);

        assert_eq!(cache.get(&key_a).expect("a").lock().cumulative_cost_atomic, 111);
        assert_eq!(cache.get(&key_b).expect("b").lock().cumulative_cost_atomic, 222);
    }
}
