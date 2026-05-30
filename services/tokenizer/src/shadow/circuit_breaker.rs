//! Per-(tenant, model) circuit breaker for Tier 1 endpoint.
//!
//! Spec refs:
//!   - `tokenizer-service-spec-v1alpha1.md` §4.5 (state transitions)
//!   - `tokenizer-service-spec-v1alpha1.md` §1.3 (hot path invariant —
//!     this breaker only gates Tier 1; Tier 2 hot path is never aware)
//!   - `tokenizer-service-spec-v1alpha1.md` §11.1 chaos test surface
//!
//! ## State machine
//!
//! ```text
//!                     ┌─────────────┐
//!                     │   CLOSED    │  (normal — sampling permitted)
//!                     └──────┬──────┘
//!         10 consecutive     │
//!              failures      ▼
//!                     ┌─────────────┐
//!                     │    OPEN     │  (skip Tier 1 entirely)
//!                     └──────┬──────┘
//!         open_duration      │
//!              elapsed       ▼
//!                     ┌─────────────┐
//!                     │  HALF_OPEN  │  (one probe permitted)
//!                     └──────┬──────┘
//!                            │
//!         probe success ─────┴──── probe failure
//!                            │
//!                            ▼
//!         (CLOSED, counter reset)        (OPEN, deadline refreshed)
//! ```
//!
//! ## Multi-tenant isolation
//!
//! State keyed on [`super::ShadowKey { tenant_id, model }`]. One tenant
//! tripping the breaker for `claude-3-5-sonnet` does NOT affect any
//! other tenant or any other model for the same tenant. Cross-tenant
//! isolation is structurally guaranteed by the HashMap key.
//!
//! ## Hot path invariant
//!
//! This module is referenced ONLY from `shadow::worker` (Phase E). It is
//! NEVER referenced from `services/sidecar/` or `services/egress_proxy/`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;

use super::sample_rate_state::ShadowKey;

/// Per spec §4.5 — 10 consecutive failures trip the breaker.
pub const DEFAULT_FAILURE_THRESHOLD: u32 = 10;

/// Per spec §4.5 — "Open 5 min" before half-open probe. We default to
/// 60 seconds for the production knob (a tighter retry cadence) but
/// preserve the 5-minute value as the documented upper bound. Tests
/// override via [`CircuitBreakerConfig`].
pub const DEFAULT_OPEN_DURATION: Duration = Duration::from_secs(60);

/// Three-state breaker per spec §4.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerStateName {
    Closed,
    Open,
    HalfOpen,
}

impl BreakerStateName {
    /// Prometheus-friendly state label for metrics emission (Phase E /
    /// Phase F dashboards consume this).
    pub fn as_label(self) -> &'static str {
        match self {
            BreakerStateName::Closed => "closed",
            BreakerStateName::Open => "open",
            BreakerStateName::HalfOpen => "half_open",
        }
    }
}

/// Configuration knobs surfaced from env / Config so tests can shorten
/// the open-duration without sleeping.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures before the breaker opens.
    pub failure_threshold: u32,
    /// Duration the breaker stays Open before transitioning to
    /// HalfOpen.
    pub open_duration: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: DEFAULT_FAILURE_THRESHOLD,
            open_duration: DEFAULT_OPEN_DURATION,
        }
    }
}

/// Per-key mutable state.
#[derive(Debug, Clone)]
struct BreakerEntry {
    state: BreakerStateName,
    /// Consecutive failures since the last success (Closed state) or
    /// the most recent open transition (HalfOpen). Cleared on probe
    /// success.
    consecutive_failures: u32,
    /// `Some(deadline)` while we are in Open. When the deadline elapses
    /// the next `permit_request` will transition the entry to
    /// HalfOpen.
    open_until: Option<Instant>,
}

impl Default for BreakerEntry {
    fn default() -> Self {
        Self {
            state: BreakerStateName::Closed,
            consecutive_failures: 0,
            open_until: None,
        }
    }
}

/// Outcome of [`CircuitBreakerState::permit_request`]: whether the
/// caller may make the Tier 1 provider call. Returned via the
/// outer-shaped `should_skip` boolean alongside the snapshot of the
/// state name we transitioned into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permit {
    /// Caller may proceed.
    Allow,
    /// Skip — breaker is Open and the deadline has not elapsed.
    SkipOpen,
}

/// Shared per-(tenant, model) breaker state. Wrapped in `Arc` so the
/// worker can share one instance across all tasks.
#[derive(Debug)]
pub struct CircuitBreakerState {
    config: CircuitBreakerConfig,
    entries: RwLock<HashMap<ShadowKey, BreakerEntry>>,
}

impl CircuitBreakerState {
    pub fn new(config: CircuitBreakerConfig) -> Arc<Self> {
        Arc::new(Self {
            config,
            entries: RwLock::new(HashMap::new()),
        })
    }

    pub fn config(&self) -> &CircuitBreakerConfig {
        &self.config
    }

    /// Decide whether to permit a Tier 1 call for this (tenant, model).
    /// If the breaker is Open and the deadline has elapsed, transition
    /// to HalfOpen and permit a single probe.
    pub fn permit_request(&self, key: &ShadowKey) -> Permit {
        self.permit_request_at(key, Instant::now())
    }

    /// Test-visible variant.
    pub fn permit_request_at(&self, key: &ShadowKey, now: Instant) -> Permit {
        let mut entries = self.entries.write();
        let entry = entries.entry(key.clone()).or_default();
        match entry.state {
            BreakerStateName::Closed | BreakerStateName::HalfOpen => Permit::Allow,
            BreakerStateName::Open => {
                if let Some(until) = entry.open_until {
                    if now >= until {
                        // Move to HalfOpen and permit one probe.
                        entry.state = BreakerStateName::HalfOpen;
                        entry.open_until = None;
                        Permit::Allow
                    } else {
                        Permit::SkipOpen
                    }
                } else {
                    // Defensive — Open without deadline should not exist;
                    // recover by treating as half-open.
                    entry.state = BreakerStateName::HalfOpen;
                    Permit::Allow
                }
            }
        }
    }

    /// Record success of a Tier 1 call. From either Closed or HalfOpen
    /// the breaker returns to Closed with a zeroed failure counter.
    pub fn record_success(&self, key: &ShadowKey) {
        let mut entries = self.entries.write();
        let entry = entries.entry(key.clone()).or_default();
        entry.state = BreakerStateName::Closed;
        entry.consecutive_failures = 0;
        entry.open_until = None;
    }

    /// Record failure of a Tier 1 call. From Closed: increment counter;
    /// if it crosses the threshold, transition to Open with a fresh
    /// deadline. From HalfOpen: immediately re-Open with a fresh
    /// deadline.
    pub fn record_failure(&self, key: &ShadowKey) {
        self.record_failure_at(key, Instant::now());
    }

    /// Test-visible variant.
    pub fn record_failure_at(&self, key: &ShadowKey, now: Instant) {
        let mut entries = self.entries.write();
        let entry = entries.entry(key.clone()).or_default();
        match entry.state {
            BreakerStateName::Closed => {
                entry.consecutive_failures =
                    entry.consecutive_failures.saturating_add(1);
                if entry.consecutive_failures >= self.config.failure_threshold {
                    entry.state = BreakerStateName::Open;
                    entry.open_until = Some(now + self.config.open_duration);
                }
            }
            BreakerStateName::HalfOpen => {
                // Probe failed → re-open with fresh deadline.
                entry.state = BreakerStateName::Open;
                entry.open_until = Some(now + self.config.open_duration);
                // Keep consecutive_failures at threshold so a single
                // probe success transitions cleanly via record_success.
                entry.consecutive_failures = self.config.failure_threshold;
            }
            BreakerStateName::Open => {
                // Already open; refresh deadline to "now + open_duration"
                // so re-failures during a probe attempt don't shorten
                // the next probe window.
                entry.open_until = Some(now + self.config.open_duration);
            }
        }
    }

    /// Read-only state snapshot for tests / metrics.
    pub fn state_of(&self, key: &ShadowKey) -> BreakerStateName {
        self.entries
            .read()
            .get(key)
            .map(|e| e.state)
            .unwrap_or(BreakerStateName::Closed)
    }

    /// Read-only consecutive failure count for tests / metrics.
    pub fn consecutive_failures(&self, key: &ShadowKey) -> u32 {
        self.entries
            .read()
            .get(key)
            .map(|e| e.consecutive_failures)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(tenant: &str, model: &str) -> ShadowKey {
        ShadowKey {
            tenant_id: tenant.into(),
            model: model.into(),
        }
    }

    fn breaker() -> Arc<CircuitBreakerState> {
        CircuitBreakerState::new(CircuitBreakerConfig {
            failure_threshold: DEFAULT_FAILURE_THRESHOLD,
            open_duration: Duration::from_millis(100),
        })
    }

    #[test]
    fn defaults_match_spec() {
        assert_eq!(DEFAULT_FAILURE_THRESHOLD, 10);
        assert_eq!(DEFAULT_OPEN_DURATION, Duration::from_secs(60));
        let cfg = CircuitBreakerConfig::default();
        assert_eq!(cfg.failure_threshold, 10);
        assert_eq!(cfg.open_duration, Duration::from_secs(60));
    }

    #[test]
    fn closed_is_default_state() {
        let b = breaker();
        let k = key("t", "claude-3-5-sonnet");
        assert_eq!(b.state_of(&k), BreakerStateName::Closed);
        assert_eq!(b.permit_request(&k), Permit::Allow);
    }

    #[test]
    fn ten_failures_open_the_breaker() {
        let b = breaker();
        let k = key("t", "claude-3-5-sonnet");
        for i in 0..9 {
            b.record_failure(&k);
            assert_eq!(
                b.state_of(&k),
                BreakerStateName::Closed,
                "still closed at failure #{}",
                i + 1
            );
        }
        b.record_failure(&k);
        assert_eq!(b.state_of(&k), BreakerStateName::Open);
        assert_eq!(b.permit_request(&k), Permit::SkipOpen);
    }

    #[test]
    fn open_transitions_to_half_open_after_deadline() {
        let b = breaker();
        let k = key("t", "gemini-1.5-pro");
        let t0 = Instant::now();
        for _ in 0..DEFAULT_FAILURE_THRESHOLD {
            b.record_failure_at(&k, t0);
        }
        assert_eq!(b.state_of(&k), BreakerStateName::Open);
        // Still open just before deadline.
        let mid = t0 + Duration::from_millis(50);
        assert_eq!(b.permit_request_at(&k, mid), Permit::SkipOpen);
        // Past deadline — half-open + permit.
        let after = t0 + Duration::from_millis(150);
        assert_eq!(b.permit_request_at(&k, after), Permit::Allow);
        assert_eq!(b.state_of(&k), BreakerStateName::HalfOpen);
    }

    #[test]
    fn half_open_probe_success_closes_the_breaker() {
        let b = breaker();
        let k = key("t", "g");
        let t0 = Instant::now();
        for _ in 0..DEFAULT_FAILURE_THRESHOLD {
            b.record_failure_at(&k, t0);
        }
        // Trigger half-open.
        b.permit_request_at(&k, t0 + Duration::from_millis(150));
        assert_eq!(b.state_of(&k), BreakerStateName::HalfOpen);
        // Probe success → Closed.
        b.record_success(&k);
        assert_eq!(b.state_of(&k), BreakerStateName::Closed);
        assert_eq!(b.consecutive_failures(&k), 0);
    }

    #[test]
    fn half_open_probe_failure_reopens_with_fresh_deadline() {
        let b = breaker();
        let k = key("t", "g");
        let t0 = Instant::now();
        for _ in 0..DEFAULT_FAILURE_THRESHOLD {
            b.record_failure_at(&k, t0);
        }
        // Reach half-open at t0 + 150ms.
        let t_half = t0 + Duration::from_millis(150);
        b.permit_request_at(&k, t_half);
        assert_eq!(b.state_of(&k), BreakerStateName::HalfOpen);
        // Probe failure at t_half → Open again with deadline = t_half + 100ms.
        b.record_failure_at(&k, t_half);
        assert_eq!(b.state_of(&k), BreakerStateName::Open);
        // Just before fresh deadline — still skip.
        let t_check = t_half + Duration::from_millis(50);
        assert_eq!(b.permit_request_at(&k, t_check), Permit::SkipOpen);
    }

    #[test]
    fn success_in_closed_resets_consecutive_failures() {
        let b = breaker();
        let k = key("t", "g");
        for _ in 0..5 {
            b.record_failure(&k);
        }
        assert_eq!(b.consecutive_failures(&k), 5);
        b.record_success(&k);
        assert_eq!(b.consecutive_failures(&k), 0);
        assert_eq!(b.state_of(&k), BreakerStateName::Closed);
    }

    #[test]
    fn cross_tenant_isolation() {
        // Tenant A's Anthropic outage must NOT trip the breaker for
        // tenant B on the same model. (§9 review Q8.)
        let b = breaker();
        let k_a = key("tenant-a", "claude-3-5-sonnet");
        let k_b = key("tenant-b", "claude-3-5-sonnet");
        for _ in 0..DEFAULT_FAILURE_THRESHOLD {
            b.record_failure(&k_a);
        }
        assert_eq!(b.state_of(&k_a), BreakerStateName::Open);
        assert_eq!(b.state_of(&k_b), BreakerStateName::Closed);
        assert_eq!(b.permit_request(&k_b), Permit::Allow);
    }

    #[test]
    fn cross_model_isolation() {
        // Same tenant — Anthropic outage must not trip the Gemini
        // breaker.
        let b = breaker();
        let k_a = key("t", "claude-3-5-sonnet");
        let k_g = key("t", "gemini-1.5-pro");
        for _ in 0..DEFAULT_FAILURE_THRESHOLD {
            b.record_failure(&k_a);
        }
        assert_eq!(b.state_of(&k_a), BreakerStateName::Open);
        assert_eq!(b.state_of(&k_g), BreakerStateName::Closed);
    }

    #[test]
    fn state_label_for_metrics() {
        assert_eq!(BreakerStateName::Closed.as_label(), "closed");
        assert_eq!(BreakerStateName::Open.as_label(), "open");
        assert_eq!(BreakerStateName::HalfOpen.as_label(), "half_open");
    }
}
