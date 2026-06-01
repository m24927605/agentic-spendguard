//! SpendGuard run_cost_projector service crate.
//!
//! Spec ref [`run-cost-projector-spec-v1alpha1.md`].
//!
//! ## Layout
//!
//!   * [`crate::server`] — `RunCostProjector` gRPC impl orchestrating
//!     signal layering per spec §6.
//!   * [`crate::state_cache`] — in-memory RunState cache with TTL +
//!     LRU bounded eviction per spec §7.
//!   * [`crate::recovery`] — canonical_events replay on cold cache miss
//!     per spec §7.4 (30-min replay window only).
//!   * [`crate::signal_1`] — induced from history (run_length_distribution_cache)
//!     per spec §3.
//!   * [`crate::signal_2`] — per-step dynamic re-projection + drift detection
//!     per spec §4.
//!   * [`crate::signal_3`] — explicit hint override per spec §5.
//!   * [`crate::layering`] — Signal 1/2/3 layering + RUN_* code precedence
//!     (BUDGET > STEPS > DRIFT) per spec §6.
//!   * [`crate::config`] — env-driven configuration (mirrors output_predictor).
//!
//! ## SLICE_09 ships
//!
//!   * Project RPC: cumulative cost + signal layering + RUN_* emission.
//!   * TerminateRun RPC: idempotent cache eviction.
//!   * Cold start: spec §3.2 default = 10 steps when no historical data.
//!   * Failure modes per spec §10 (run_length cache unreachable, cache miss,
//!     projector unreachable from sidecar — last handled by sidecar Phase E).
//!
//! ## Hot path invariant
//!
//! This crate is linked from sidecar (Phase E activates RUN_* pass-through
//! that SLICE_02 stubbed). It is NOT linked from egress_proxy — that wiring
//! is SLICE_10. The verification gate `grep` in the implementation plan
//! enforces this invariant.
//!
//! [`run-cost-projector-spec-v1alpha1.md`]: ../../docs/run-cost-projector-spec-v1alpha1.md

pub mod config;
pub mod layering;
pub mod recovery;
pub mod server;
pub mod signal_1;
pub mod signal_2;
pub mod signal_3;
pub mod state_cache;

/// Generated protobuf types — `tonic::include_proto!` requires this
/// module path so server / client codegen lands inside the crate's
/// public namespace.
pub mod proto {
    pub mod run_cost_projector {
        pub mod v1 {
            tonic::include_proto!("spendguard.run_cost_projector.v1");
        }
    }
}
