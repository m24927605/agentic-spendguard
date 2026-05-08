//! Provider Webhook Receiver — translates provider HTTPS webhooks to
//! Ledger gRPC ops (Phase 2B Step 11; design /tmp/codex-webhook-r3.txt LOCKED).

pub mod config;
pub mod domain;
pub mod handlers;
pub mod persistence;
pub mod server;

/// Generated proto stubs for the Ledger gRPC client.
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
