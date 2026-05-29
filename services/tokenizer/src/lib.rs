//! Tokenizer gRPC service crate (centralised form per
//! [`tokenizer-service-spec-v1alpha1.md`] §2.1(a)).
//!
//! ## Layout
//!
//!   * [`crate::server`] — `TokenizerService` impl wrapping
//!     [`spendguard_tokenizer::Tokenizer`] in a tonic `Service`.
//!   * [`crate::dispatch`] — service-side dispatch helpers (proto
//!     ↔ library-struct conversions).
//!   * [`crate::config`] — env-driven configuration (mirrors
//!     `services/sidecar/src/config.rs` pattern).
//!
//! SLICE_03 ships:
//!
//!   * `Tokenize` RPC: forwards to the in-process library.
//!   * `ShadowVerify` RPC: returns `Status::unimplemented` with
//!     a stable error message — SLICE_05 wires the Tier 1 worker.
//!
//! [`tokenizer-service-spec-v1alpha1.md`]: ../../docs/tokenizer-service-spec-v1alpha1.md

pub mod config;
pub mod dispatch;
pub mod server;

/// Generated protobuf types — `tonic::include_proto!` requires this
/// module path so server / client codegen lands inside the crate's
/// public namespace.
pub mod proto {
    pub mod tokenizer {
        pub mod v1 {
            tonic::include_proto!("spendguard.tokenizer.v1");
        }
    }
}
