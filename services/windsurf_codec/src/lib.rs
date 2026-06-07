//! # spendguard-windsurf-codec — Windsurf IDE / Codeium Cascade MITM codec (EXPERIMENTAL, SOW only)
//!
//! Per [`design.md`](../../docs/specs/coverage/D18_windsurf_mitm/design.md) §1:
//!
//! > **DO NOT SHIP AS A GA FEATURE.** Gated to Enterprise SOW
//! > customers who signed the maintenance addendum. Codec breaks
//! > whenever Codeium changes the wire protocol. Customer accepts
//! > break-window risk.
//!
//! This crate reverse-engineers the Windsurf IDE Cascade runtime's
//! outbound wire format toward `server.codeium.com` /
//! `windsurf-server.codeium.com` so SpendGuard can interpose budget
//! reservation / commit / release on a Cascade session. The framing
//! layer is public gRPC-Web; the Cascade envelope is reconstructed
//! from black-box capture (no vendor source).
//!
//! ## Crate scope (D18 SLICE 75-82)
//!
//! * [`framing`] — gRPC-Web length-prefixed frame reader (SLICE 76).
//! * [`windsurf_proto`] — prost-generated types for the observed
//!   Cascade envelope (SLICE 75).
//! * [`envelope`] — typed decode helpers + version gate (SLICE 76).
//! * [`version`] — wire-version registry (SLICE 76).
//! * [`error`] — typed error surface (SLICE 76).
//! * [`routing`] — Codeium endpoint detection (SLICE 77).
//! * [`experimental`] — two-channel opt-in gate + boot warning
//!   (SLICE 77).
//! * [`openai_models`] — canonical OpenAI shape the translator
//!   pivots through (SLICE 78).
//! * [`translate`] — Cascade ↔ OpenAI translation (SLICE 78).
//! * [`reencode`] — byte-for-byte re-encode round-trip helpers
//!   (SLICE 78).
//! * [`passthrough`] — byte-perfect tee + `decoder_skipped` audit
//!   (SLICE 79).
//! * [`forward`] — per-connection MITM forward state machine
//!   (SLICE 78).
//! * [`replay`] — `.windsurf-rpc` fixture replay harness (SLICE 80).
//!
//! ## Feature gates
//!
//! The crate exposes a `mitm` feature flag. With the workspace-level
//! `windsurf-mitm-experimental` feature on (set in
//! `services/egress_proxy` and `services/cli`), the consumer enables
//! `mitm` here. Without it, the public API still compiles, but
//! downstream wiring is `cfg`-gated out.
//!
//! ## Legal posture
//!
//! See [`README.md`](../README.md) for the full customer-facing
//! disclaimer. In brief: this is reverse-engineered interop, not
//! vendor-endorsed. Customers who enable the codec do so under an
//! Enterprise SOW that acknowledges (a) the codec breaks on vendor
//! release, (b) the customer is responsible for confirming their own
//! Codeium/Windsurf terms permit on-host MITM.
//!
//! ## Loud experimental banner
//!
//! Per design.md §3 decision 2 (mirror of D17 E2): every public
//! entry point in this crate calls
//! [`assert_experimental_banner_emitted`] on first use per process.

#![deny(missing_docs)]

use std::sync::OnceLock;

pub mod envelope;
pub mod error;
pub mod experimental;
pub mod framing;
pub mod openai_models;
pub mod passthrough;
pub mod reencode;
pub mod replay;
pub mod routing;
pub mod translate;
pub mod version;
pub mod windsurf_proto;

pub mod forward;

pub use envelope::{
    decode_request_body, decode_request_frame, decode_response_body, decode_response_frame,
    strip_grpc_web_prefix,
};
pub use error::WindsurfCodecError;
pub use experimental::{
    emit_boot_warning, windsurf_codec_enabled, ExperimentalConfig, WindsurfExperimentalConfig,
};
pub use forward::{
    CountedUpstream, InMemorySidecar, MitmForward, ReleaseReason, SessionError, SessionResult,
    SidecarDecision, SidecarLane, UpstreamConnector,
};
pub use framing::{
    Frame, GrpcWebReader, DEFAULT_MAX_FRAME_LEN, FLAG_COMPRESSED, FLAG_END_OF_STREAM,
    FLAG_GRPC_WEB_TRAILERS,
};
pub use openai_models::{
    OpenAiChatRequest, OpenAiChatResponseChunk, OpenAiChunkChoice, OpenAiChunkDelta, OpenAiMessage,
    OpenAiUsage,
};
pub use passthrough::{passthrough_bytes, DecoderSkipReason, DecoderSkippedEvent};
pub use replay::{
    raw_frame, read_fixture, read_fixture_bytes, replay_fixture, replay_fixture_bytes,
    request_frame, response_delta_frame, trailers_frame, write_fixture_bytes, Direction,
    FixtureFrame, ReplayError, ReplayReport, FIXTURE_MAGIC, FIXTURE_VERSION,
};
pub use routing::{
    classify_cascade_route, is_cascade_chat_route, is_cascade_host, CascadeRoutingDecision,
    WINDSURF_CASCADE_HOSTS, WINDSURF_CASCADE_PATH_REGEX,
};
pub use translate::{
    cascade_request_to_openai, extract_cascade_output_tokens, extract_openai_output_tokens,
    openai_chunk_to_cascade,
};
pub use version::{detect_version, is_known, WireVersion, KNOWN_WIRE_VERSIONS};

/// Emit the experimental banner to stderr on first call per process.
///
/// Safe to call from every public entry point — [`OnceLock`]
/// guarantees the message lands exactly once even under concurrent
/// callers. The banner is the second of the three "loud
/// experimental" markers per
/// [`design.md`](../../docs/specs/coverage/D18_windsurf_mitm/design.md)
/// §3 decision 2.
pub fn assert_experimental_banner_emitted() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        eprintln!(
            "[EXPERIMENTAL] windsurf-mitm codec active. \
             Vendor protocol: undocumented. \
             Support tier: SOW only. \
             SOW: services/windsurf_codec/SOW.md. \
             DO NOT SHIP IN GA CONFIG."
        );
    });
}

#[cfg(test)]
mod lib_tests {
    use super::*;

    #[test]
    fn banner_does_not_panic_on_repeated_calls() {
        assert_experimental_banner_emitted();
        assert_experimental_banner_emitted();
        assert_experimental_banner_emitted();
    }
}
