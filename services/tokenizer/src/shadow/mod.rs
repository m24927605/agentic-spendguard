//! Tier 1 shadow drift detection — orchestrator module.
//!
//! Spec refs:
//!   - `tokenizer-service-spec-v1alpha1.md` §4 (Tier 1 shadow architecture)
//!   - `tokenizer-service-spec-v1alpha1.md` §4.1 (sample rate strategy)
//!   - `tokenizer-service-spec-v1alpha1.md` §4.2 (per-kind drift thresholds)
//!   - `tokenizer-service-spec-v1alpha1.md` §4.3 (1h cool-down window)
//!   - `tokenizer-service-spec-v1alpha1.md` §4.4 (`tokenizer_t1_samples` schema)
//!   - `tokenizer-service-spec-v1alpha1.md` §4.5 (circuit breaker)
//!   - `tokenizer-service-spec-v1alpha1.md` §11.1 (chaos test surface)
//!   - `stats-aggregator-spec-v1alpha1.md` §7.2 (drift_alert CloudEvent schema)
//!
//! ## Module layout
//!
//!   * [`sample_rate_state`] — per-(tenant, model) sample rate + cool-down
//!     state. Default 1% sampling; 100% during the cool-down window after
//!     a drift alert.
//!   * [`provider_clients`] — Anthropic + Gemini count_tokens HTTP clients.
//!   * [`circuit_breaker`] — per-(tenant, model) failure tracking with
//!     Closed → Open → HalfOpen → Closed transitions.
//!   * [`worker`] — async shadow loop spawned at boot; consumes the
//!     non-blocking channel populated by the gRPC server.
//!
//! ## Hot path invariant
//!
//! This module is referenced ONLY from `services/tokenizer/src/main.rs`
//! (worker spawn) and `services/tokenizer/src/server.rs` (best-effort
//! channel send after Tier 2 returns). It is NEVER referenced from
//! `services/sidecar/` or `services/egress_proxy/` — those crates use
//! the `spendguard-tokenizer` library form for Tier 2 hot path and
//! must not have access to Tier 1 surfaces (spec §1.3 invariant).
//!
//! Grep verification: `services/sidecar/ services/egress_proxy/` must
//! contain zero references to `shadow_worker`, `provider_clients`,
//! `sample_rate_state`, `circuit_breaker`, or `shadow::` paths.

pub mod circuit_breaker;
pub mod persistence;
pub mod provider_clients;
pub mod sample_rate_state;
pub mod sink;
pub mod worker;

// Re-exports for the small set of types main.rs / server.rs reach for.
pub use sample_rate_state::{SampleRateConfig, SampleRateState, ShadowKey};
pub use worker::{spawn_shadow_worker, ShadowEvent, ShadowWorkerHandle};
