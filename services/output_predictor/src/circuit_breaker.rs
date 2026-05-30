//! Per-tenant circuit breaker for the customer Strategy C plugin call.
//!
//! Spec refs:
//!   - `output-predictor-plugin-contract-v1alpha1.md` §6 (state machine
//!     + thresholds)
//!   - `output-predictor-plugin-contract-v1alpha1.md` §6.2 (per-tenant
//!     scope — one tenant's plugin outage MUST NOT affect another tenant)
//!   - `output-predictor-plugin-contract-v1alpha1.md` §6.4 (half-open
//!     probe semantics)
//!   - `output-predictor-plugin-contract-v1alpha1.md` §1.3 / §5.3
//!     (invariant: breaker Open NEVER blocks reservation — strategy_c
//!     callers fall to Strategy B silently)
//!
//! ## State machine (spec §6.1)
//!
//! ```text
//!                     ┌─────────────┐
//!                     │   Closed    │   (normal — Predict permitted)
//!                     └──────┬──────┘
//!         10 consecutive     │
//!              failures      ▼
//!                     ┌─────────────┐
//!                     │    Open     │   (skip Predict; fall to B)
//!                     └──────┬──────┘
//!              5 min elapsed │
//!                            ▼
//!                     ┌─────────────┐
//!                     │  HalfOpen   │   (one probe permitted)
//!                     └──────┬──────┘
//!                            │
//!         probe success ─────┴──── probe failure
//!                            │
//!                            ▼
//!         (Closed, counter reset)        (Open, deadline refreshed)
//! ```
//!
//! ## Per-tenant isolation (spec §6.2)
//!
//! State keyed on `uuid::Uuid` (tenant_id). One tenant tripping the
//! breaker does NOT affect any other tenant's Predict path. Cross-tenant
//! isolation is structurally guaranteed by the HashMap key — the same
//! defense-in-depth posture the tokenizer SLICE_05 breaker uses for
//! `(tenant_id, model)` keys.
//!
//! ## Hot path invariant
//!
//! The breaker is consulted INSIDE strategy_c.rs which itself is one
//! arm of the `tokio::join!(a, b, c)` orchestration in server.rs. A
//! breaker-Open decision SHORTENS the C path to an immediate skip-then-
//! Permit::SkipOpen return; A and B continue computing in parallel.
//! The breaker NEVER blocks the overall Predict RPC — that's the
//! universal §1.8 plugin-failure-isolation invariant.
//!
//! ## Why not reuse tokenizer's CircuitBreakerState?
//!
//! tokenizer's breaker keys on `(tenant_id, model)` because shadow
//! sampling has provider-specific outage modes. The plugin breaker
//! keys on `tenant_id` alone because the spec contract is per-tenant
//! (one endpoint serves all models for that tenant). Sharing the
//! type would force the plugin caller to invent a sentinel model key
//! (e.g. `"_plugin_"`) and break the type-system safety guarantee that
//! a stray tokenizer key cannot leak into the plugin breaker map.
//!
//! The two breakers are independent state machines with identical
//! transition logic — duplicating ~50 lines of state-transition code
//! is cheaper than coupling two unrelated services through a shared
//! abstraction.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use uuid::Uuid;

/// Per spec §6.1 — 10 consecutive failures trip the breaker.
pub const DEFAULT_FAILURE_THRESHOLD: u32 = 10;

/// Per spec §6.1 — "Open 5 min" before half-open probe.
pub const DEFAULT_OPEN_DURATION: Duration = Duration::from_secs(300);

/// Three-state breaker per spec §6.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerStateName {
    Closed,
    Open,
    HalfOpen,
}

impl BreakerStateName {
    /// Prometheus-friendly state label for the
    /// `customer_predictor_circuit_breaker_state{tenant_id, state}`
    /// metric emitted per spec §9.1.
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

/// Per-tenant mutable state.
#[derive(Debug, Clone)]
struct BreakerEntry {
    state: BreakerStateName,
    /// Consecutive failures since the last success (Closed state) or
    /// the most recent open transition (HalfOpen). Cleared on probe
    /// success.
    consecutive_failures: u32,
    /// `Some(deadline)` while we are in Open. When the deadline elapses
    /// the next `permit_request` will transition the entry to HalfOpen.
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

/// Outcome of [`PluginCircuitBreaker::permit_request`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permit {
    /// Caller may issue the Predict RPC.
    Allow,
    /// Skip — breaker is Open and the deadline has not elapsed.
    /// strategy_c.rs treats this as a silent "no C this call" and
    /// the selector falls to B per spec §5.1.
    SkipOpen,
}

/// Shared per-tenant breaker state. Wrapped in `Arc` so the
/// output_predictor server can share one instance across all concurrent
/// Predict tasks.
#[derive(Debug)]
pub struct PluginCircuitBreaker {
    config: CircuitBreakerConfig,
    entries: RwLock<HashMap<Uuid, BreakerEntry>>,
}

impl PluginCircuitBreaker {
    pub fn new(config: CircuitBreakerConfig) -> Arc<Self> {
        Arc::new(Self {
            config,
            entries: RwLock::new(HashMap::new()),
        })
    }

    pub fn config(&self) -> &CircuitBreakerConfig {
        &self.config
    }

    /// Decide whether to permit a Predict call for this tenant.
    /// If the breaker is Open and the deadline has elapsed, transition
    /// to HalfOpen and permit a single probe call.
    pub fn permit_request(&self, tenant: &Uuid) -> Permit {
        self.permit_request_at(tenant, Instant::now())
    }

    /// Test-visible variant.
    pub fn permit_request_at(&self, tenant: &Uuid, now: Instant) -> Permit {
        let mut entries = self.entries.write();
        let entry = entries.entry(*tenant).or_default();
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
                    // Defensive — Open without deadline should never
                    // exist; recover by treating as HalfOpen.
                    entry.state = BreakerStateName::HalfOpen;
                    Permit::Allow
                }
            }
        }
    }

    /// Record success of a Predict call (real or probe). From either
    /// Closed or HalfOpen the breaker returns to Closed with a zeroed
    /// failure counter.
    pub fn record_success(&self, tenant: &Uuid) {
        let mut entries = self.entries.write();
        let entry = entries.entry(*tenant).or_default();
        entry.state = BreakerStateName::Closed;
        entry.consecutive_failures = 0;
        entry.open_until = None;
    }

    /// Record failure of a Predict call. From Closed: increment counter;
    /// if it crosses the threshold, transition to Open with a fresh
    /// deadline. From HalfOpen: immediately re-Open with a fresh
    /// deadline. From Open: refresh deadline (re-failures during a
    /// probe attempt don't shorten the next probe window).
    pub fn record_failure(&self, tenant: &Uuid) {
        self.record_failure_at(tenant, Instant::now());
    }

    /// Test-visible variant.
    pub fn record_failure_at(&self, tenant: &Uuid, now: Instant) {
        let mut entries = self.entries.write();
        let entry = entries.entry(*tenant).or_default();
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
                entry.consecutive_failures = self.config.failure_threshold;
            }
            BreakerStateName::Open => {
                entry.open_until = Some(now + self.config.open_duration);
            }
        }
    }

    /// Operator-triggered reset per spec §6.3 (force-reset circuit
    /// breaker via control plane API). Returns to Closed with zeroed
    /// counter regardless of current state.
    pub fn force_reset(&self, tenant: &Uuid) {
        let mut entries = self.entries.write();
        let entry = entries.entry(*tenant).or_default();
        entry.state = BreakerStateName::Closed;
        entry.consecutive_failures = 0;
        entry.open_until = None;
    }

    /// R2 B2 — record a successful HealthCheck probe (driven by the
    /// 30s loop in main.rs per spec §6.3). HealthCheck OK from any
    /// state transitions the breaker to Closed with zeroed counter —
    /// the health channel is independent of the Predict hot path and
    /// represents an authoritative "the plugin is live" signal that
    /// supersedes accumulated Predict failures (which may have been
    /// a transient burst).
    ///
    /// Semantically identical to `record_success` today; kept as a
    /// distinct method so the call site in main.rs documents that the
    /// signal came from HealthCheck not Predict. Future spec §6.3
    /// refinement may grow asymmetric transitions (e.g. require N
    /// consecutive HealthCheck OKs to close from Open).
    pub fn record_health_ok(&self, tenant: &Uuid) {
        let mut entries = self.entries.write();
        let entry = entries.entry(*tenant).or_default();
        entry.state = BreakerStateName::Closed;
        entry.consecutive_failures = 0;
        entry.open_until = None;
    }

    /// R2 B2 — record a failed HealthCheck probe. Per spec §6.3
    /// failure mode 8 (NOT_SERVING), one failed health probe is enough
    /// to flip the breaker Open (this is the dedicated health channel,
    /// not the noisy Predict path that gates on N consecutive failures
    /// — health probes already have their own debouncing via the 30s
    /// cadence, so the deadline-refresh shape mirrors `record_failure`
    /// in Open state).
    pub fn record_health_fail(&self, tenant: &Uuid) {
        self.record_health_fail_at(tenant, Instant::now());
    }

    /// Test-visible variant.
    pub fn record_health_fail_at(&self, tenant: &Uuid, now: Instant) {
        let mut entries = self.entries.write();
        let entry = entries.entry(*tenant).or_default();
        entry.state = BreakerStateName::Open;
        entry.open_until = Some(now + self.config.open_duration);
        entry.consecutive_failures = self.config.failure_threshold;
    }

    /// Read-only state snapshot for tests / metrics.
    pub fn state_of(&self, tenant: &Uuid) -> BreakerStateName {
        self.entries
            .read()
            .get(tenant)
            .map(|e| e.state)
            .unwrap_or(BreakerStateName::Closed)
    }

    /// Read-only consecutive failure count for tests / metrics.
    pub fn consecutive_failures(&self, tenant: &Uuid) -> u32 {
        self.entries
            .read()
            .get(tenant)
            .map(|e| e.consecutive_failures)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(seed: u8) -> Uuid {
        // Deterministic tenant fixture; first byte distinguishes tenants
        // so tests can be read at a glance.
        Uuid::from_bytes([seed; 16])
    }

    fn breaker() -> Arc<PluginCircuitBreaker> {
        PluginCircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: DEFAULT_FAILURE_THRESHOLD,
            open_duration: Duration::from_millis(100),
        })
    }

    #[test]
    fn defaults_match_spec() {
        // §6.1: 10 consecutive failures → Open; Open 5 min → HalfOpen.
        assert_eq!(DEFAULT_FAILURE_THRESHOLD, 10);
        assert_eq!(DEFAULT_OPEN_DURATION, Duration::from_secs(300));
        let cfg = CircuitBreakerConfig::default();
        assert_eq!(cfg.failure_threshold, 10);
        assert_eq!(cfg.open_duration, Duration::from_secs(300));
    }

    #[test]
    fn closed_is_default_state() {
        let b = breaker();
        let k = t(1);
        assert_eq!(b.state_of(&k), BreakerStateName::Closed);
        assert_eq!(b.permit_request(&k), Permit::Allow);
    }

    #[test]
    fn ten_failures_open_the_breaker() {
        let b = breaker();
        let k = t(1);
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
        let k = t(1);
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
        let k = t(1);
        let t0 = Instant::now();
        for _ in 0..DEFAULT_FAILURE_THRESHOLD {
            b.record_failure_at(&k, t0);
        }
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
        let k = t(1);
        let t0 = Instant::now();
        for _ in 0..DEFAULT_FAILURE_THRESHOLD {
            b.record_failure_at(&k, t0);
        }
        let t_half = t0 + Duration::from_millis(150);
        b.permit_request_at(&k, t_half);
        assert_eq!(b.state_of(&k), BreakerStateName::HalfOpen);
        b.record_failure_at(&k, t_half);
        assert_eq!(b.state_of(&k), BreakerStateName::Open);
        let t_check = t_half + Duration::from_millis(50);
        assert_eq!(b.permit_request_at(&k, t_check), Permit::SkipOpen);
    }

    #[test]
    fn success_in_closed_resets_consecutive_failures() {
        let b = breaker();
        let k = t(1);
        for _ in 0..5 {
            b.record_failure(&k);
        }
        assert_eq!(b.consecutive_failures(&k), 5);
        b.record_success(&k);
        assert_eq!(b.consecutive_failures(&k), 0);
        assert_eq!(b.state_of(&k), BreakerStateName::Closed);
    }

    #[test]
    fn force_reset_returns_to_closed_from_open() {
        // Spec §6.3: control plane operator may force-reset.
        let b = breaker();
        let k = t(1);
        for _ in 0..DEFAULT_FAILURE_THRESHOLD {
            b.record_failure(&k);
        }
        assert_eq!(b.state_of(&k), BreakerStateName::Open);
        b.force_reset(&k);
        assert_eq!(b.state_of(&k), BreakerStateName::Closed);
        assert_eq!(b.consecutive_failures(&k), 0);
        assert_eq!(b.permit_request(&k), Permit::Allow);
    }

    #[test]
    fn cross_tenant_isolation() {
        // Spec §6.2: tenant A's plugin outage MUST NOT trip the breaker
        // for tenant B. This is the universal §1.9 multi-tenant
        // isolation invariant — structurally guaranteed by HashMap key.
        let b = breaker();
        let k_a = t(0xAA);
        let k_b = t(0xBB);
        for _ in 0..DEFAULT_FAILURE_THRESHOLD {
            b.record_failure(&k_a);
        }
        assert_eq!(b.state_of(&k_a), BreakerStateName::Open);
        assert_eq!(b.state_of(&k_b), BreakerStateName::Closed);
        assert_eq!(b.permit_request(&k_b), Permit::Allow);
    }

    #[test]
    fn record_health_ok_closes_breaker_from_open() {
        // R2 B2 spec §6.3: a successful HealthCheck probe is an
        // authoritative "plugin live" signal that supersedes accumulated
        // Predict failures. Drive the breaker Open via 10 consecutive
        // Predict failures, then issue record_health_ok — the breaker
        // must return to Closed.
        let b = breaker();
        let k = t(1);
        for _ in 0..DEFAULT_FAILURE_THRESHOLD {
            b.record_failure(&k);
        }
        assert_eq!(b.state_of(&k), BreakerStateName::Open);
        b.record_health_ok(&k);
        assert_eq!(b.state_of(&k), BreakerStateName::Closed);
        assert_eq!(b.consecutive_failures(&k), 0);
        assert_eq!(b.permit_request(&k), Permit::Allow);
    }

    #[test]
    fn record_health_ok_closes_breaker_from_half_open() {
        let b = breaker();
        let k = t(1);
        let t0 = Instant::now();
        for _ in 0..DEFAULT_FAILURE_THRESHOLD {
            b.record_failure_at(&k, t0);
        }
        b.permit_request_at(&k, t0 + Duration::from_millis(150));
        assert_eq!(b.state_of(&k), BreakerStateName::HalfOpen);
        b.record_health_ok(&k);
        assert_eq!(b.state_of(&k), BreakerStateName::Closed);
    }

    #[test]
    fn record_health_fail_opens_breaker_from_closed() {
        // R2 B2 spec §6.3 NOT_SERVING: one failed HealthCheck probe is
        // enough to open the breaker (the health channel has its own
        // 30s debouncing so this is the equivalent of "many Predict
        // failures").
        let b = breaker();
        let k = t(1);
        assert_eq!(b.state_of(&k), BreakerStateName::Closed);
        b.record_health_fail(&k);
        assert_eq!(b.state_of(&k), BreakerStateName::Open);
        assert_eq!(b.permit_request(&k), Permit::SkipOpen);
    }

    #[test]
    fn record_health_fail_refreshes_open_deadline() {
        let b = breaker();
        let k = t(1);
        let t0 = Instant::now();
        for _ in 0..DEFAULT_FAILURE_THRESHOLD {
            b.record_failure_at(&k, t0);
        }
        assert_eq!(b.state_of(&k), BreakerStateName::Open);
        let t1 = t0 + Duration::from_millis(150);
        // record_health_fail at t1 should refresh the deadline so the
        // 100ms test breaker stays Open through t1+50ms.
        b.record_health_fail_at(&k, t1);
        let t_mid = t1 + Duration::from_millis(50);
        assert_eq!(b.permit_request_at(&k, t_mid), Permit::SkipOpen);
    }

    #[test]
    fn record_health_fail_isolated_per_tenant() {
        // Spec §6.2 cross-tenant isolation also applies to health-driven
        // transitions.
        let b = breaker();
        let k_a = t(0xAA);
        let k_b = t(0xBB);
        b.record_health_fail(&k_a);
        assert_eq!(b.state_of(&k_a), BreakerStateName::Open);
        assert_eq!(b.state_of(&k_b), BreakerStateName::Closed);
    }

    #[test]
    fn state_label_for_metrics() {
        // Per spec §9.1: customer_predictor_circuit_breaker_state{tenant,state}
        assert_eq!(BreakerStateName::Closed.as_label(), "closed");
        assert_eq!(BreakerStateName::Open.as_label(), "open");
        assert_eq!(BreakerStateName::HalfOpen.as_label(), "half_open");
    }
}
