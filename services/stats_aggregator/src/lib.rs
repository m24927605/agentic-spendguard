//! SpendGuard stats_aggregator service crate.
//!
//! Spec ref [`stats-aggregator-spec-v1alpha1.md`].
//!
//! ## Layout
//!
//!   * [`crate::scheduler`] — hourly cron loop + Postgres advisory
//!     lock singleton enforcement (spec §8).
//!   * [`crate::aggregation`] — main SQL aggregation cycle (spec §4.1).
//!   * [`crate::drift_detector`] — 2σ comparison + signed CloudEvent
//!     emission (spec §7).
//!   * [`crate::run_length`] — per-(tenant, agent_id) run-length
//!     distribution (spec §6).
//!   * [`crate::config`] — env-driven configuration.
//!
//! SLICE_06 ships the full daemon: scheduler + advisory lock + aggregation
//! + drift detection + run-length cache + signed CloudEvent emission.
//!
//! [`stats-aggregator-spec-v1alpha1.md`]: ../../docs/stats-aggregator-spec-v1alpha1.md

pub mod aggregation;
pub mod config;
pub mod drift_detector;
pub mod run_length;
pub mod scheduler;

/// Generated protobuf types — CloudEvent envelope + canonical_ingest
/// AppendEvents client. Mirror of the tokenizer SLICE_05 pattern.
pub mod proto {
    pub mod common {
        pub mod v1 {
            tonic::include_proto!("spendguard.common.v1");
        }
    }
    pub mod canonical_ingest {
        pub mod v1 {
            tonic::include_proto!("spendguard.canonical_ingest.v1");
        }
    }
}
