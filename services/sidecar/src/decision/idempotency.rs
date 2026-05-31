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

#[derive(Debug, Clone, PartialEq)]
pub enum Lookup {
    Hit(DecisionResponse),
    Miss,
    Conflict { existing_fingerprint_hex: String },
}

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
    request_fingerprint_hex: String,
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

    pub fn get(&self, key: &str, request_fingerprint_hex: &str) -> Lookup {
        let mut g = self.inner.lock();
        let now = Utc::now();
        if let Some(entry) = g.map.get(key) {
            if (now - entry.inserted_at).num_seconds() <= self.ttl_secs {
                if entry.request_fingerprint_hex == request_fingerprint_hex {
                    return Lookup::Hit(entry.response.clone());
                }
                return Lookup::Conflict {
                    existing_fingerprint_hex: entry.request_fingerprint_hex.clone(),
                };
            }
        }
        // Expired or missing.
        g.map.remove(key);
        Lookup::Miss
    }

    pub fn put(&self, key: String, request_fingerprint_hex: String, response: DecisionResponse) {
        let mut g = self.inner.lock();
        if g.map.contains_key(&key) {
            g.map.insert(
                key,
                Entry {
                    request_fingerprint_hex,
                    response,
                    inserted_at: Utc::now(),
                },
            );
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
                request_fingerprint_hex,
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
        c.put("key-1".into(), "fp-1".into(), fake_response("decision-1"));
        let got = c.get("key-1", "fp-1");
        match got {
            Lookup::Hit(resp) => assert_eq!(resp.decision_id, "decision-1"),
            other => panic!("expected cache hit, got {other:?}"),
        }
    }

    #[test]
    fn same_key_different_fingerprint_conflicts() {
        let c = IdempotencyCache::new(8, 600);
        c.put("key-1".into(), "fp-1".into(), fake_response("decision-1"));
        let got = c.get("key-1", "fp-2");
        assert_eq!(
            got,
            Lookup::Conflict {
                existing_fingerprint_hex: "fp-1".into()
            }
        );
    }

    #[test]
    fn evicts_oldest_on_capacity() {
        let c = IdempotencyCache::new(2, 600);
        c.put("a".into(), "fp-a".into(), fake_response("1"));
        c.put("b".into(), "fp-b".into(), fake_response("2"));
        c.put("c".into(), "fp-c".into(), fake_response("3"));
        assert!(matches!(c.get("a", "fp-a"), Lookup::Miss));
        assert!(matches!(c.get("b", "fp-b"), Lookup::Hit(_)));
        assert!(matches!(c.get("c", "fp-c"), Lookup::Hit(_)));
    }
}
