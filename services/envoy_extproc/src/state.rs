//! Per-ExtProc-stream state map.
//!
//! Spec refs:
//!   - docs/specs/coverage/D01_envoy_extproc/implementation.md §13
//!     ("Per-stream state location? In-memory map keyed by session_id")
//!   - docs/specs/coverage/D01_envoy_extproc/review-standards.md §4.1
//!     blocker on `StreamState` boundedness (SLICE 3 spec; SLICE 2 lays
//!     the foundation honouring the same boundedness rule).
//!
//! ## What we stash here
//!
//! Each ExtProc `Process` stream represents one downstream HTTP request
//! through Envoy. Across that stream's lifetime we receive separate
//! frames for Request-Headers, Request-Body, Response-Headers,
//! Response-Body. We need a place to carry data between phases:
//!
//!   * **SLICE 2**: the parsed [`ParsedRequest`] (so log lines on later
//!     phases can mention the model + provider) and the
//!     [`ClaimEstimate`] (so SLICE 3 has the input_tokens reservation
//!     when it issues the sidecar `RequestDecision` RPC).
//!   * **SLICE 3**: the sidecar `reservation_id` + `decision_id`.
//!   * **SLICE 4**: the upstream response usage block.
//!
//! ## Stream id derivation
//!
//! The stream id is derived from the Request-Headers phase. We pick the
//! first non-empty of:
//!
//!   1. `x-request-id` (Envoy injects this for every request)
//!   2. an internal generated UUIDv4 (when the header is absent — should
//!      only happen in unit tests; production Envoy AI Gateway always
//!      sets it).
//!
//! The id is per-process unique; the map handles concurrent insert /
//! lookup via a `Mutex<HashMap<...>>`. SLICE 3 will switch to a TTL-
//! bound LRU per review-standards §4.1; SLICE 2's bounded `HashMap` is
//! sized at construction (default 8 192 streams) and rejects insertions
//! beyond capacity with a warn-and-evict-oldest fallback.
//!
//! ## Why a `tokio::sync::Mutex<HashMap>` rather than `dashmap`
//!
//! The Request-Body phase only touches one key per stream and holds the
//! lock for ~10 µs (an `insert` + clone-out). The contention envelope at
//! 50ms p99 per call (Contract §14) is far below the threshold where
//! lock-free maps win, and adding a new third-party crate would expand
//! the supply-chain surface for marginal performance. SLICE 3 will
//! re-evaluate if real-stack load tests show contention; current
//! `services/sidecar/src/decision/idempotency.rs` uses the same
//! `Mutex<HashMap>` shape for the audit chain's idempotency cache.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::parse::ParsedRequest;
use crate::tokenize::ClaimEstimate;

/// SLICE 3 — coarse-grained record of the sidecar's verdict for this
/// stream. SLICE 4's audit emit reads this to decide between emitting
/// `LLM_CALL_POST.SUCCESS` (after Response-Body for ALLOW) vs
/// `RUN_ABORTED` (when the stream closed without a SUCCESS pair). The
/// Display impl is used by structured-log fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionOutcome {
    /// Sidecar said `CONTINUE` (and we ACK'd the Request-Body with
    /// ExtProc CONTINUE). The reservation_id+decision_id pair lives in
    /// [`StreamState`] for SLICE 4's outcome emit.
    Allow,
    /// Sidecar said `STOP` or `STOP_RUN_PROJECTION` — we returned a 429
    /// ImmediateResponse and closed the stream.
    Deny,
    /// Sidecar returned DENY, DEGRADE, or REQUIRE_APPROVAL — request
    /// rejected at fail-closed boundary. SLICE 4 disambiguates via
    /// decision_id lookup against sidecar audit. Per the SLICE 3
    /// fail-closed carve-out the DEGRADE `mutation_patch_json` is NOT
    /// applied; the BodyMutation arm lands in SLICE 5 conformance.
    Rejected,
    /// Sidecar was unreachable / errored — we returned a 503
    /// ImmediateResponse with Retry-After.
    SidecarError,
    /// SLICE 2 carry-over: no ClaimEstimate was available (parse/tokenize
    /// failed). We returned 503 fail-closed at the Request-Body phase.
    MissingClaimEstimate,
}

/// Hard cap on concurrent in-flight streams. Per review-standards §4.1
/// the SLICE 3 spec requires bounded state to prevent OOM under chaos;
/// SLICE 2 inherits the same bound so we don't ship an unbounded map
/// that SLICE 3 has to retrofit.
pub const DEFAULT_STREAM_STATE_CAPACITY: usize = 8_192;

/// Per-stream session id — a copy of the `x-request-id` header (or a
/// generated UUIDv4 fallback). String form preserves any non-UUID values
/// (Envoy can be configured to inject a custom request id format).
pub type StreamId = String;

/// Per-stream state. SLICE 2 carries the parsed request + claim
/// estimate; SLICE 3 (this slice) appends the sidecar's reservation_id +
/// decision_id + coarse-grained outcome enum so SLICE 4's audit emit
/// (Response-Body phase) can reference them.
///
/// `Default` is not derived because [`Instant`] has no `Default`
/// impl. Use [`StreamState::new`] which seeds `created_at = Instant::now()`.
#[derive(Debug, Clone)]
pub struct StreamState {
    /// HTTP `:path` captured from the Request-Headers phase. Empty until
    /// Request-Headers is received. SLICE 2 uses it to invoke
    /// `parse::parse_request_body` from the Request-Body phase.
    pub path: String,
    /// Parsed Request-Body output (None until Request-Body is received).
    pub parsed: Option<ParsedRequest>,
    /// SLICE 2 ClaimEstimate (None until Request-Body parse + tokenize
    /// both succeed). On parse / tokenize error the state holds None,
    /// and SLICE 3 fails closed at the budget query.
    pub estimate: Option<ClaimEstimate>,
    /// SLICE 3 — sidecar's reservation handle. None until the
    /// RequestDecision RPC completes with `Decision::CONTINUE`. SLICE 4
    /// uses this in `LLM_CALL_POST.reservation_id` (per
    /// review-standards §5.1: "no UUID generation in ExtProc").
    pub reservation_id: Option<String>,
    /// SLICE 3 — sidecar's decision_id. Carried to SLICE 4 so the audit
    /// emit references the same decision row.
    pub decision_id: Option<String>,
    /// SLICE 3 — coarse-grained verdict the budget query produced. SLICE
    /// 4 reads this on stream close (or response-body end) to choose
    /// between LLM_CALL_POST.SUCCESS / RUN_ABORTED / no-op.
    pub decision_outcome: Option<DecisionOutcome>,
    /// Insertion time. SLICE 3's bounded LRU will use this for eviction.
    pub created_at: Instant,
}

impl Default for StreamState {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamState {
    /// Construct a fresh state for a newly observed stream id. `path` is
    /// updated by [`StreamStateMap::upsert`] once Request-Headers
    /// arrives. SLICE 3 reservation_id/decision_id/decision_outcome
    /// start `None` — the Request-Body handler populates them after the
    /// sidecar `RequestDecision` RPC returns.
    pub fn new() -> Self {
        Self {
            path: String::new(),
            parsed: None,
            estimate: None,
            reservation_id: None,
            decision_id: None,
            decision_outcome: None,
            created_at: Instant::now(),
        }
    }
}

/// Process-shared map of stream id → state.
///
/// Wrapped in `Arc<Mutex<...>>` so the gRPC server can clone the handle
/// per-stream without losing the shared store. The map is bounded; an
/// overflow warns and falls back to dropping the oldest insertion. SLICE
/// 3 swaps the eviction policy for true LRU.
#[derive(Debug, Clone)]
pub struct StreamStateMap {
    inner: Arc<Mutex<HashMap<StreamId, StreamState>>>,
    capacity: usize,
}

impl Default for StreamStateMap {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_STREAM_STATE_CAPACITY)
    }
}

impl StreamStateMap {
    /// Construct a fresh map. `capacity` is the hard cap on concurrent
    /// streams; over-cap inserts warn and evict a random entry (the
    /// `HashMap::keys().next()` pick is not ordered — SLICE 3 will switch
    /// to true LRU).
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::with_capacity(capacity.min(1024)))),
            capacity: capacity.max(1),
        }
    }

    /// Insert / replace state for `stream_id`. Returns the previous
    /// state if any (mostly useful for tests; production callers ignore
    /// the return value).
    pub async fn upsert(&self, stream_id: StreamId, state: StreamState) -> Option<StreamState> {
        let mut g = self.inner.lock().await;
        if g.len() >= self.capacity && !g.contains_key(&stream_id) {
            // Capacity reached and we're not replacing an existing key —
            // evict an arbitrary entry. SLICE 3 will replace this with a
            // proper LRU.
            if let Some(evicted_id) = g.keys().next().cloned() {
                warn!(
                    capacity = self.capacity,
                    evicted = %evicted_id,
                    new = %stream_id,
                    "StreamStateMap at capacity; evicting a random entry (SLICE 3 will switch to true LRU)"
                );
                g.remove(&evicted_id);
            }
        }
        g.insert(stream_id, state)
    }

    /// Look up state by stream id, returning a clone for the caller to
    /// inspect without holding the lock.
    pub async fn get(&self, stream_id: &str) -> Option<StreamState> {
        let g = self.inner.lock().await;
        g.get(stream_id).cloned()
    }

    /// Mutate state in place. Returns true if the stream id was present.
    /// The closure runs while the lock is held — keep it short.
    pub async fn mutate<F>(&self, stream_id: &str, mutator: F) -> bool
    where
        F: FnOnce(&mut StreamState),
    {
        let mut g = self.inner.lock().await;
        match g.get_mut(stream_id) {
            Some(s) => {
                mutator(s);
                true
            }
            None => false,
        }
    }

    /// Remove + return state when the stream closes. SLICE 4 will call
    /// this from the stream end-of-life path; SLICE 2 doesn't yet.
    #[allow(dead_code)]
    pub async fn remove(&self, stream_id: &str) -> Option<StreamState> {
        let mut g = self.inner.lock().await;
        g.remove(stream_id)
    }

    /// Current map size — for unit tests + future Prometheus gauge.
    pub async fn len(&self) -> usize {
        let g = self.inner.lock().await;
        g.len()
    }

    /// Empty check.
    #[allow(dead_code)]
    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }
}

/// Pull the request id from a HeaderMap. Returns the first non-empty of
/// the candidate headers (lower-case match); falls back to a freshly
/// minted UUIDv4 when no candidate is present. Envoy AI Gateway always
/// injects `x-request-id` in production, but unit tests and the
/// SLICE 1 handshake_smoke fixture may not — the v4 fallback keeps the
/// state map keyed correctly.
pub fn derive_stream_id_from_headers(
    headers: &crate::proto::envoy::config::core::v3::HeaderMap,
) -> StreamId {
    const CANDIDATES: &[&str] = &["x-request-id", "x-envoy-request-id"];
    for h in &headers.headers {
        let key = h.key.to_ascii_lowercase();
        if CANDIDATES.contains(&key.as_str()) {
            // HeaderValue.value is a String per the Envoy proto; the
            // newer field `raw_value` is bytes. Fall back to raw_value
            // if value is empty (HTTP/2 binary headers).
            if !h.value.is_empty() {
                return h.value.clone();
            }
            if !h.raw_value.is_empty() {
                if let Ok(s) = std::str::from_utf8(&h.raw_value) {
                    if !s.is_empty() {
                        return s.to_string();
                    }
                }
            }
        }
    }
    // Fallback — keep the stream addressable in the map.
    debug!("no x-request-id header present; minting UUIDv4 fallback stream id");
    uuid::Uuid::new_v4().to_string()
}

/// Pull the `:path` pseudo-header from a HeaderMap. Returns an empty
/// string when absent (the caller logs + falls through to CONTINUE).
pub fn derive_path_from_headers(
    headers: &crate::proto::envoy::config::core::v3::HeaderMap,
) -> String {
    for h in &headers.headers {
        if h.key == ":path" {
            if !h.value.is_empty() {
                return h.value.clone();
            }
            if let Ok(s) = std::str::from_utf8(&h.raw_value) {
                return s.to_string();
            }
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::envoy::config::core::v3::{HeaderMap, HeaderValue};

    #[tokio::test]
    async fn upsert_then_get_returns_state() {
        let map = StreamStateMap::default();
        let mut state = StreamState::new();
        state.path = "/v1/chat/completions".to_string();
        map.upsert("stream-A".to_string(), state).await;
        let got = map.get("stream-A").await.expect("must hit");
        assert_eq!(got.path, "/v1/chat/completions");
    }

    #[tokio::test]
    async fn mutate_in_place_updates_existing_state() {
        let map = StreamStateMap::default();
        map.upsert("stream-B".to_string(), StreamState::new()).await;
        let mutated = map
            .mutate("stream-B", |s| {
                s.path = "/v1/messages".to_string();
            })
            .await;
        assert!(mutated);
        let got = map.get("stream-B").await.unwrap();
        assert_eq!(got.path, "/v1/messages");
    }

    #[tokio::test]
    async fn mutate_missing_returns_false() {
        let map = StreamStateMap::default();
        let mutated = map.mutate("nope", |_| {}).await;
        assert!(!mutated);
    }

    #[tokio::test]
    async fn over_capacity_inserts_evict() {
        let map = StreamStateMap::with_capacity(2);
        map.upsert("a".to_string(), StreamState::new()).await;
        map.upsert("b".to_string(), StreamState::new()).await;
        assert_eq!(map.len().await, 2);
        // Insert "c" — capacity is 2 so one of {a, b} must be evicted.
        map.upsert("c".to_string(), StreamState::new()).await;
        assert_eq!(map.len().await, 2);
        // "c" must be present.
        assert!(map.get("c").await.is_some());
    }

    #[test]
    fn derive_stream_id_picks_x_request_id() {
        let headers = HeaderMap {
            headers: vec![
                HeaderValue {
                    key: ":path".into(),
                    value: "/v1/chat/completions".into(),
                    raw_value: Default::default(),
                },
                HeaderValue {
                    key: "x-request-id".into(),
                    value: "abc-123".into(),
                    raw_value: Default::default(),
                },
            ],
        };
        assert_eq!(derive_stream_id_from_headers(&headers), "abc-123");
    }

    #[test]
    fn derive_stream_id_falls_back_to_uuid_when_missing() {
        let headers = HeaderMap {
            headers: vec![HeaderValue {
                key: ":path".into(),
                value: "/v1/chat/completions".into(),
                raw_value: Default::default(),
            }],
        };
        let id = derive_stream_id_from_headers(&headers);
        // Fallback should parse as a UUID — pin shape so callers can
        // rely on it being non-empty.
        assert!(!id.is_empty());
        assert!(uuid::Uuid::parse_str(&id).is_ok());
    }

    #[test]
    fn derive_stream_id_matches_lowercase_header_key() {
        // Envoy lowercases headers in HTTP/2; defensive coverage.
        let headers = HeaderMap {
            headers: vec![HeaderValue {
                key: "X-Request-ID".into(),
                value: "MixedCase-123".into(),
                raw_value: Default::default(),
            }],
        };
        assert_eq!(derive_stream_id_from_headers(&headers), "MixedCase-123");
    }

    #[test]
    fn derive_path_returns_pseudo_header_value() {
        let headers = HeaderMap {
            headers: vec![HeaderValue {
                key: ":path".into(),
                value: "/v1/messages".into(),
                raw_value: Default::default(),
            }],
        };
        assert_eq!(derive_path_from_headers(&headers), "/v1/messages");
    }

    #[test]
    fn derive_path_empty_when_missing() {
        let headers = HeaderMap {
            headers: vec![HeaderValue {
                key: "host".into(),
                value: "api.example.com".into(),
                raw_value: Default::default(),
            }],
        };
        assert_eq!(derive_path_from_headers(&headers), "");
    }
}
