pub mod classify;
pub mod config;
pub mod domain;
pub mod handlers;
pub mod metrics;
pub mod persistence;
pub mod server;
pub mod verifier;

// SLICE_13 Phase C: expose verify-chain replay as a library entry point
// so the calibration-report CLI can call it inline. This is an additive
// module — does NOT touch the existing `src/bin/verify_chain.rs` stub
// (per slice constraint to keep SLICE_01-12 shipped files unchanged
// except this library export).
pub mod verify_chain_lib;

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
