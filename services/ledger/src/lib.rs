//! SpendGuard Ledger service library crate.
//!
//! See `services/ledger/README.md` for module map.

pub mod config;
pub mod domain;
pub mod handlers;
pub mod metrics;
pub mod persistence;
pub mod server;
// D13 COV_65 — Subscription usage importer stubs.  D14/D15/D16 light
// up the live Devin / Manus / Genspark backends; Anthropic + OpenAI
// stay stubbed until their Admin APIs ship.
pub mod subscription_importer;

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
