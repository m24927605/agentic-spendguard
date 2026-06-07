//! Byte-perfect pass-through tee.
//!
//! D18 SLICE 79: upstream traffic is forwarded byte-for-byte; the
//! decoder runs in parallel on a clone of the stream. Decode failure
//! NEVER blocks the request — it logs `decoder_skipped` and forwards
//! with no reservation. This is the load-bearing fail-safe per
//! design.md §4.4.
//!
//! Per design.md §3 decision 3 + §4 architecture: byte-perfect
//! pass-through; decode failure degrades to no-gate, never blocks
//! the request. The SOW addendum states this risk explicitly so the
//! customer signs for it.

use bytes::Bytes;

/// Reason code for a `decoder_skipped` audit event.
///
/// Per design.md §4.4: emitted whenever the codec's parallel decode
/// path errors on a known wire version. The egress proxy still
/// forwards the bytes — gating is best-effort and never blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoderSkipReason {
    /// Decode failed on a known wire version (vendor pushed an
    /// envelope change without bumping the version stamp).
    KnownVersionDecodeFailed,
    /// Decoder tap was slow / dead; the upstream forward outran it.
    TapBackpressureDropped,
    /// Frame was compressed but the codec doesn't yet handle gzip.
    /// Forward verbatim; no gating.
    CompressedNotSupported,
}

impl DecoderSkipReason {
    /// String form for the audit event `reason_code` field.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::KnownVersionDecodeFailed => "windsurf_decode_failed",
            Self::TapBackpressureDropped => "windsurf_decoder_tap_dropped",
            Self::CompressedNotSupported => "windsurf_compressed_unsupported",
        }
    }
}

/// Audit-event payload emitted by the egress proxy when the codec
/// skipped decoding for any reason.
///
/// Per design.md §4.4: this event is what dashboards aggregate to
/// detect a codec break in production — when the rate crosses the
/// SOW-stated alert threshold the operator knows to file a re-capture
/// ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecoderSkippedEvent {
    /// Reason code (machine-readable).
    pub reason_code: DecoderSkipReason,
    /// Inbound host the request targeted (e.g. `server.codeium.com`).
    pub inbound_host: String,
    /// Wire-version stamp if it was detected before decode failed.
    /// `None` when the failure happened at the framing layer.
    pub wire_version: Option<String>,
    /// Optional debug context (single line, redacted of any PII).
    pub debug_note: Option<String>,
}

impl DecoderSkippedEvent {
    /// Build a new skip event with the canonical event-kind tag the
    /// egress proxy's audit-row writer expects.
    pub fn new(reason_code: DecoderSkipReason, inbound_host: impl Into<String>) -> Self {
        Self {
            reason_code,
            inbound_host: inbound_host.into(),
            wire_version: None,
            debug_note: None,
        }
    }

    /// Attach a wire-version stamp.
    pub fn with_wire_version(mut self, v: impl Into<String>) -> Self {
        self.wire_version = Some(v.into());
        self
    }

    /// Attach a debug note (single line, redacted).
    pub fn with_debug_note(mut self, note: impl Into<String>) -> Self {
        self.debug_note = Some(note.into());
        self
    }

    /// The canonical audit `kind` field — `"decoder_skipped"`.
    pub fn kind(&self) -> &'static str {
        "decoder_skipped"
    }
}

/// Compute the byte-perfect passthrough buffer.
///
/// Per design.md §4.4: the codec never mutates upstream bytes. The
/// `observe()` model is that the egress proxy gets a clone of each
/// chunk into the decoder tap; the wire bytes flow upstream
/// untouched. This function is the canonical helper a test or a
/// non-async caller uses to assert the byte-perfect contract: given
/// a stream of frame bytes, the output is identical to the input.
pub fn passthrough_bytes(input: &Bytes) -> Bytes {
    input.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (1) Passthrough returns identical bytes.
    #[test]
    fn passthrough_preserves_bytes_byte_identical() {
        let input = Bytes::from_static(b"\x01\x02\x03 cascade payload \xff\xfe");
        let out = passthrough_bytes(&input);
        assert_eq!(out, input);
    }

    /// (2) Empty input → empty output.
    #[test]
    fn passthrough_empty_input() {
        let input = Bytes::new();
        let out = passthrough_bytes(&input);
        assert!(out.is_empty());
    }

    /// (3) Skip-reason as_str maps correctly.
    #[test]
    fn skip_reason_as_str() {
        assert_eq!(
            DecoderSkipReason::KnownVersionDecodeFailed.as_str(),
            "windsurf_decode_failed"
        );
        assert_eq!(
            DecoderSkipReason::TapBackpressureDropped.as_str(),
            "windsurf_decoder_tap_dropped"
        );
        assert_eq!(
            DecoderSkipReason::CompressedNotSupported.as_str(),
            "windsurf_compressed_unsupported"
        );
    }

    /// (4) Decoder skipped event builder.
    #[test]
    fn decoder_skipped_event_builder() {
        let evt = DecoderSkippedEvent::new(
            DecoderSkipReason::KnownVersionDecodeFailed,
            "server.codeium.com",
        )
        .with_wire_version("cascade.v2.1")
        .with_debug_note("body decode failed at offset 42");

        assert_eq!(evt.kind(), "decoder_skipped");
        assert_eq!(evt.reason_code, DecoderSkipReason::KnownVersionDecodeFailed);
        assert_eq!(evt.inbound_host, "server.codeium.com");
        assert_eq!(evt.wire_version.as_deref(), Some("cascade.v2.1"));
        assert_eq!(
            evt.debug_note.as_deref(),
            Some("body decode failed at offset 42")
        );
    }

    /// (5) Audit-event kind is the literal string the proxy expects.
    #[test]
    fn audit_event_kind_is_decoder_skipped() {
        let evt = DecoderSkippedEvent::new(
            DecoderSkipReason::CompressedNotSupported,
            "windsurf-server.codeium.com",
        );
        assert_eq!(evt.kind(), "decoder_skipped");
    }
}
