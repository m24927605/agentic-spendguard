//! tonic-generated proto modules.
//!
//! The build.rs invokes `tonic_build::compile_protos` against:
//!   * The vendored Envoy ExtProc proto tree at `proto/` (SLICE 1).
//!   * The SpendGuard sidecar adapter + common protos at `../../proto/`
//!     (SLICE 3 — the new sidecar_client.rs is a tonic CLIENT of
//!     `SidecarAdapter`, and the handshake_smoke integration test stands
//!     up a mock `SidecarAdapter` server, so both server + client stubs
//!     are emitted).
//!
//! The generated Rust modules are mounted here under the same Rust path
//! as the proto package names so call sites read the same as the source
//! protos.

pub mod envoy {
    pub mod config {
        pub mod core {
            pub mod v3 {
                tonic::include_proto!("envoy.config.core.v3");
            }
        }
    }
    // Rust raw-identifier escape — `type` is a keyword; the upstream proto package is `envoy.type.v3`.
    pub mod r#type {
        pub mod v3 {
            tonic::include_proto!("envoy.r#type.v3");
        }
    }
    pub mod extensions {
        pub mod filters {
            pub mod http {
                pub mod ext_proc {
                    pub mod v3 {
                        tonic::include_proto!("envoy.extensions.filters.http.ext_proc.v3");
                    }
                }
            }
        }
    }
    pub mod service {
        pub mod ext_proc {
            pub mod v3 {
                tonic::include_proto!("envoy.service.ext_proc.v3");
            }
        }
    }
}

/// SpendGuard sidecar adapter + common message protos. SLICE 3 wires
/// these in for the RequestDecision client; the same modules are used
/// by the handshake_smoke integration test's mock SidecarAdapter server.
pub mod spendguard {
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
}
