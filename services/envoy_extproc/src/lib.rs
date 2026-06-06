//! Envoy AI Gateway ExtProc sidecar — library surface.
//!
//! Slice 1 ships only the skeleton: a tonic gRPC server impl of
//! `envoy.service.ext_proc.v3.ExternalProcessor.Process` whose Handshake
//! phase opens a sidecar UDS connection (mirroring `services/egress_proxy`).
//! Slices 2-4 wire token counting, decision translation, and audit emit.
//!
//! Spec refs:
//!   - docs/specs/coverage/D01_envoy_extproc/design.md §3 locked decisions
//!   - docs/specs/coverage/D01_envoy_extproc/implementation.md §1 module layout
//!   - docs/slices/COV_01_envoy_extproc_skeleton.md scope
//!
//! Re-exports the tonic-generated proto types under [`proto`] so external
//! tests can construct `ProcessingRequest` / `ProcessingResponse` messages
//! without depending on the implementation crate's path-mangled module
//! tree.

pub mod config;
pub mod handshake;
pub mod proto;
pub mod server;
