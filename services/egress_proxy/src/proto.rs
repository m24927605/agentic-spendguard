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

// SLICE_10 Phase A: egress_proxy is also a CLIENT of output_predictor +
// run_cost_projector. The Predict + Project calls happen BEFORE the
// DecisionRequest reaches the sidecar so the proxy can populate
// ClaimEstimate with the full 17-column prediction metadata.
pub mod output_predictor {
    pub mod v1 {
        tonic::include_proto!("spendguard.output_predictor.v1");
    }
}
pub mod run_cost_projector {
    pub mod v1 {
        tonic::include_proto!("spendguard.run_cost_projector.v1");
    }
}
