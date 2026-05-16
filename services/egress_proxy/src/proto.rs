//! Generated proto stubs (tonic-build emits via build.rs).
//!
//! Mirrors the layout used by other Rust services (ttl_sweeper,
//! outbox_forwarder) so the type imports look identical.

pub mod common {
    pub mod v1 {
        tonic::include_proto!("spendguard.common.v1");
    }
}
pub mod sidecar_adapter {
    pub mod v1 {
        tonic::include_proto!("spendguard.sidecar_adapter.v1");
    }
}
