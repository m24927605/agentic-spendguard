//! SpendGuard Ledger service library crate.
//!
//! See `services/ledger/README.md` for module map.

pub mod config;
pub mod domain;
pub mod handlers;
pub mod persistence;
pub mod server;

pub mod proto {
    //! Generated protobuf modules.
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
