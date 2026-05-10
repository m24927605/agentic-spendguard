//! Outbox Forwarder — closes Phase 2B audit chain loop:
//! audit_outbox (pending_forward=TRUE) → canonical_ingest.AppendEvents
//!
//! Design source: /tmp/codex-outbox-r2.txt (v2 LOCKED at round 2).
//! Happy-path-only POC: APPENDED/DEDUPED clear pending; all other
//! statuses keep pending (no silent audit drop).

pub mod config;
pub mod decode;
pub mod forward;
pub mod metrics;
pub mod poll;
pub mod state;

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
