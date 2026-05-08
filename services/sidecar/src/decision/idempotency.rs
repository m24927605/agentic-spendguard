//! Adapter-side idempotency cache (Contract §6 idempotency_key).
//!
//! Adapter retries with the SAME `Idempotency.key` MUST produce the same
//! `decision_id` + audit chain. Without this, sidecar mints a fresh
//! `decision_id` per call and the ledger sees a duplicate logical request
//! as a brand-new transaction (audit chain breaks; reservations duplicate).
//!
//! POC: in-memory LRU bounded by `idempotency_cache_size` with per-entry
//! TTL `idempotency_cache_ttl_secs`. After process restart the cache is
//! empty; the ledger UNIQUE on `(tenant_id, operation_kind,
//! idempotency_key)` then catches duplicates server-side and returns a
//! Replay variant — sidecar maps that back to the cached response.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::Mutex;

use crate::proto::sidecar_adapter::v1::DecisionResponse;

#[derive(Clone)]
pub struct IdempotencyCache {
    inner: Arc<Mutex<Inner>>,
    capacity: usize,
    ttl_secs: i64,
}

struct Inner {
    /// key = Idempotency.key from the adapter.
    map: HashMap<String, Entry>,
    /// FIFO of insertion order for capacity bound.
    order: std::collections::VecDeque<String>,
}

#[derive(Clone)]
struct Entry {
    response: DecisionResponse,
    inserted_at: DateTime<Utc>,
}

impl IdempotencyCache {
    pub fn new(capacity: usize, ttl_secs: i64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                map: HashMap::with_capacity(capacity),
                order: std::collections::VecDeque::with_capacity(capacity),
            })),
            capacity: capacity.max(1),
            ttl_secs: ttl_secs.max(1),
        }
    }

    pub fn get(&self, key: &str) -> Option<DecisionResponse> {
        let mut g = self.inner.lock();
        let now = Utc::now();
        if let Some(entry) = g.map.get(key) {
            if (now - entry.inserted_at).num_seconds() <= self.ttl_secs {
                return Some(entry.response.clone());
            }
        }
        // Expired or missing.
        g.map.remove(key);
        None
    }

    pub fn put(&self, key: String, response: DecisionResponse) {
        let mut g = self.inner.lock();
        if g.map.contains_key(&key) {
            g.map.insert(key, Entry {
                response,
                inserted_at: Utc::now(),
            });
            return;
        }
        if g.order.len() >= self.capacity {
            if let Some(oldest) = g.order.pop_front() {
                g.map.remove(&oldest);
            }
        }
        g.order.push_back(key.clone());
        g.map.insert(
            key,
            Entry {
                response,
                inserted_at: Utc::now(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::sidecar_adapter::v1::decision_response::Decision;

    fn fake_response(decision_id: &str) -> DecisionResponse {
        DecisionResponse {
            decision_id: decision_id.to_string(),
            decision: Decision::Continue as i32,
            ..Default::default()
        }
    }

    #[test]
    fn round_trip_returns_same_response() {
        let c = IdempotencyCache::new(8, 600);
        c.put("key-1".into(), fake_response("decision-1"));
        let got = c.get("key-1").expect("cache hit");
        assert_eq!(got.decision_id, "decision-1");
    }

    #[test]
    fn evicts_oldest_on_capacity() {
        let c = IdempotencyCache::new(2, 600);
        c.put("a".into(), fake_response("1"));
        c.put("b".into(), fake_response("2"));
        c.put("c".into(), fake_response("3"));
        assert!(c.get("a").is_none());
        assert!(c.get("b").is_some());
        assert!(c.get("c").is_some());
    }
}
