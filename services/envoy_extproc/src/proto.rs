//! tonic-generated proto modules.
//!
//! The build.rs invokes `tonic_build::compile_protos` against the vendored
//! ExtProc proto tree at `proto/`. The generated Rust modules are mounted
//! here under the same Rust path as the proto package names so call sites
//! read the same as the upstream Envoy proto.

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
