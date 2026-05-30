//! Phase A placeholder — full implementation in Phase D.
//!
//! Per spec §4.5 — per-(tenant, model) circuit breaker with
//! Closed → Open (10 consecutive failures) → HalfOpen (probe after 60s)
//! → Closed (probe success) or Open (probe fail).
//!
//! See `tokenizer-service-spec-v1alpha1.md` §4.5.

/// Failure threshold from spec §4.5. Open state engages after 10
/// consecutive failures from the (tenant, model) Tier 1 endpoint.
pub const DEFAULT_FAILURE_THRESHOLD: u32 = 10;

/// Open-state duration before half-open probe. Spec §4.5 says
/// "Open 5 min" so we use 300 seconds in production; tests can shorten
/// via the `CircuitBreakerConfig::open_duration` knob in Phase D.
pub const DEFAULT_OPEN_DURATION: std::time::Duration =
    std::time::Duration::from_secs(60);
