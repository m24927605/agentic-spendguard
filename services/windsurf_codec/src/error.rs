//! Windsurf codec error surface.
//!
//! D18 SLICE 76 — typed errors. Each variant maps to a [`crate::version::WireVersion`]
//! or a framing-layer fault.

use crate::version::WireVersion;
use thiserror::Error;

/// Error returned when a Windsurf Cascade envelope payload fails to
/// decode or when the wire-layer framing is malformed.
///
/// Per D18 design.md §3 decision 5: unknown wire version returns
/// `UnsupportedWireVersion` and the request is NOT forwarded — the
/// caller emits `windsurf_wire_version_unsupported` as the reason
/// code and the codec's two-channel opt-in gate kicks in.
#[derive(Debug, Error)]
pub enum WindsurfCodecError {
    /// The framing buffer was too short to hold a 5-byte gRPC-Web
    /// length prefix.
    #[error("buffer too short for gRPC-Web length prefix")]
    TruncatedPrefix,

    /// The gRPC-Web payload body was shorter than the declared length
    /// prefix.
    #[error("gRPC-Web payload truncated: expected {expected} bytes, got {got}")]
    TruncatedBody {
        /// Number of bytes the length prefix declared.
        expected: usize,
        /// Number of bytes actually in the buffer.
        got: usize,
    },

    /// The prost decoder rejected the bytes — invalid varint,
    /// truncated nested message, unknown wire type, etc.
    #[error("cascade envelope decode failed: {0}")]
    Protobuf(#[from] prost::DecodeError),

    /// The wire-version stamp is not in the [`crate::KNOWN_WIRE_VERSIONS`]
    /// registry.
    #[error("unsupported wire version: {0}")]
    UnsupportedWireVersion(WireVersion),

    /// The decoded envelope was structurally valid protobuf but
    /// missing a field SpendGuard considers required.
    #[error("cascade envelope missing required field: {0}")]
    MissingField(&'static str),

    /// gRPC-Web-level I/O error reading framing layer bytes.
    #[error("grpc-web framing I/O error: {0}")]
    Io(#[from] std::io::Error),
}
