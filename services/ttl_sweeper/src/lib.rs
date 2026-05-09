//! TTL Sweeper — background worker that releases expired reservations
//! via Ledger.Release(reason=TTL_EXPIRED). Closes Step 7.5 P1.1 known
//! POC gap (TTL_EXPIRED reason out-of-scope sweeper).
//!
//! Design source: /tmp/codex-ttl-r2.txt (v2 LOCKED at round 2).

pub mod audit;
pub mod config;
pub mod canonical;
pub mod poll;
pub mod sequence;
pub mod state;
pub mod sweep;

pub mod proto {
    pub mod common {
        pub mod v1 {
            tonic::include_proto!("spendguard.common.v1");
        }
    }
    pub mod ledger {
        pub mod v1 {
            tonic::include_proto!("spendguard.ledger.v1");
        }
    }
}
