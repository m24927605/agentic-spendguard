//! Cascade wire envelope decode helpers.
//!
//! Each gRPC-Web data frame's payload is a serialised
//! [`CascadeRequest`] (client → server unary/streaming request) or
//! [`CascadeResponseDelta`] (server → client streaming delta). This
//! module wraps `prost::Message::decode` with typed errors and shape
//! validation, plus the [`WireVersion`] gate that fails closed on
//! unknown versions per D18 design.md §3 decision 5.
//!
//! Per D18 review-standards (mirror of D17 W4): vendor field
//! additions are preserved transparently by prost's silent
//! unknown-field drop on decode, so byte-perfect preservation is
//! enforced at the framing layer ([`crate::passthrough`]), not at
//! the envelope layer.

use bytes::Bytes;
use prost::Message;

use crate::error::WindsurfCodecError;
use crate::version::{detect_version, is_known, WireVersion};
use crate::windsurf_proto::{CascadeRequest, CascadeResponseDelta};

/// Strip the 5-byte gRPC-Web length prefix from a buffer and return
/// the body plus the detected wire version.
///
/// Compressed payloads (`flag & 0x01 != 0`) are rejected at this
/// layer — the codec is decode-and-observe-only; gzip handling is
/// deferred to a future slice with fixture coverage. The pass-through
/// path in [`crate::passthrough`] still forwards them byte-perfect.
pub fn strip_grpc_web_prefix(buf: &Bytes) -> Result<(WireVersion, Bytes), WindsurfCodecError> {
    if buf.len() < 5 {
        return Err(WindsurfCodecError::TruncatedPrefix);
    }
    let compressed = buf[0] != 0;
    let len = u32::from_be_bytes(buf[1..5].try_into().unwrap()) as usize;
    if buf.len() < 5 + len {
        return Err(WindsurfCodecError::TruncatedBody {
            expected: 5 + len,
            got: buf.len(),
        });
    }
    let body = buf.slice(5..5 + len);
    if compressed {
        // No fixture coverage yet — reject explicitly. Reviewer
        // rejects merging a real gzip path without an accompanying
        // fixture in tests/fixtures/.
        return Err(WindsurfCodecError::MissingField("gzip_unsupported"));
    }
    let version = detect_version(&body);
    Ok((version, body))
}

/// Decode a single Cascade request frame from a 5-byte-prefixed
/// gRPC-Web payload.
///
/// Returns [`WindsurfCodecError::UnsupportedWireVersion`] when the
/// version stamp is not in [`crate::KNOWN_WIRE_VERSIONS`]. Returns
/// [`WindsurfCodecError::MissingField`] when the decoded envelope
/// has an empty `model_name`.
pub fn decode_request_frame(buf: &Bytes) -> Result<CascadeRequest, WindsurfCodecError> {
    let (version, body) = strip_grpc_web_prefix(buf)?;
    if !is_known(&version) {
        return Err(WindsurfCodecError::UnsupportedWireVersion(version));
    }
    let req = CascadeRequest::decode(body)?;
    if req.model_name.is_empty() {
        return Err(WindsurfCodecError::MissingField("model_name"));
    }
    Ok(req)
}

/// Decode a single Cascade response delta from a 5-byte-prefixed
/// gRPC-Web payload.
pub fn decode_response_frame(buf: &Bytes) -> Result<CascadeResponseDelta, WindsurfCodecError> {
    let (version, body) = strip_grpc_web_prefix(buf)?;
    if !is_known(&version) {
        return Err(WindsurfCodecError::UnsupportedWireVersion(version));
    }
    let delta = CascadeResponseDelta::decode(body)?;
    Ok(delta)
}

/// Decode a Cascade request body that has ALREADY had its 5-byte
/// gRPC-Web prefix stripped (the body extracted by [`crate::framing`]).
///
/// Used by the SLICE 80 fixture replay harness where the framing
/// reader has already split each frame into `[flag][len][payload]`.
pub fn decode_request_body(body: &[u8]) -> Result<CascadeRequest, WindsurfCodecError> {
    let req = CascadeRequest::decode(body)?;
    if req.model_name.is_empty() {
        return Err(WindsurfCodecError::MissingField("model_name"));
    }
    Ok(req)
}

/// Decode a Cascade response delta body that has ALREADY had its
/// 5-byte gRPC-Web prefix stripped.
pub fn decode_response_body(body: &[u8]) -> Result<CascadeResponseDelta, WindsurfCodecError> {
    let delta = CascadeResponseDelta::decode(body)?;
    Ok(delta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windsurf_proto::{CascadeMessage, CascadeUsage};

    fn encode_with_prefix<M: Message>(msg: &M) -> Bytes {
        let mut payload = Vec::new();
        msg.encode(&mut payload).unwrap();
        let mut buf = Vec::with_capacity(5 + payload.len());
        buf.push(0x00);
        buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        buf.extend_from_slice(&payload);
        Bytes::from(buf)
    }

    /// (1) Round-trip encode → decode of a minimal request with
    /// known wire version.
    #[test]
    fn round_trip_minimal_request_v2_0() {
        let original = CascadeRequest {
            messages: vec![CascadeMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            model_name: "gpt-4o".to_string(),
            max_tokens: Some(64),
            tool_declarations: vec![],
            workspace_id: None,
            cascade_wire_version: Some("cascade.v2.0".to_string()),
        };
        let buf = encode_with_prefix(&original);
        let decoded = decode_request_frame(&buf).unwrap();
        assert_eq!(decoded.model_name, "gpt-4o");
        assert_eq!(decoded.messages.len(), 1);
        assert_eq!(decoded.max_tokens, Some(64));
        assert_eq!(
            decoded.cascade_wire_version.as_deref(),
            Some("cascade.v2.0")
        );
    }

    /// (2) Unknown wire version → `UnsupportedWireVersion`.
    #[test]
    fn unknown_wire_version_rejected() {
        let original = CascadeRequest {
            messages: vec![],
            model_name: "gpt-4o".to_string(),
            max_tokens: None,
            tool_declarations: vec![],
            workspace_id: None,
            cascade_wire_version: Some("cascade.v9.9".to_string()),
        };
        let buf = encode_with_prefix(&original);
        let err = decode_request_frame(&buf).unwrap_err();
        assert!(
            matches!(err, WindsurfCodecError::UnsupportedWireVersion(_)),
            "expected UnsupportedWireVersion, got: {err:?}"
        );
    }

    /// (3) Truncated prefix → `TruncatedPrefix`.
    #[test]
    fn truncated_prefix_rejected() {
        let buf = Bytes::from_static(b"\x00\x00\x00"); // only 3 bytes
        let err = decode_request_frame(&buf).unwrap_err();
        assert!(matches!(err, WindsurfCodecError::TruncatedPrefix));
    }

    /// (4) Truncated body → `TruncatedBody`.
    #[test]
    fn truncated_body_rejected() {
        let mut buf = Vec::new();
        buf.push(0x00);
        buf.extend_from_slice(&100u32.to_be_bytes()); // declare 100 bytes
        buf.extend_from_slice(b"abc"); // actually have 3
        let buf = Bytes::from(buf);
        let err = decode_request_frame(&buf).unwrap_err();
        assert!(
            matches!(
                err,
                WindsurfCodecError::TruncatedBody {
                    expected: 105,
                    got: 8
                }
            ),
            "got: {err:?}"
        );
    }

    /// (5) Empty `model_name` → `MissingField`.
    #[test]
    fn empty_model_name_rejected() {
        let original = CascadeRequest {
            messages: vec![CascadeMessage {
                role: "user".into(),
                content: "x".into(),
            }],
            model_name: String::new(),
            max_tokens: None,
            tool_declarations: vec![],
            workspace_id: None,
            cascade_wire_version: Some("cascade.v2.0".to_string()),
        };
        let buf = encode_with_prefix(&original);
        let err = decode_request_frame(&buf).unwrap_err();
        assert!(
            matches!(err, WindsurfCodecError::MissingField("model_name")),
            "got: {err:?}"
        );
    }

    /// (6) Response delta round-trip with usage block.
    #[test]
    fn round_trip_response_delta_with_usage() {
        let original = CascadeResponseDelta {
            model_name: "gpt-4o".to_string(),
            text_chunk: Some("hello world".to_string()),
            finish_reason: Some("stop".to_string()),
            usage: Some(CascadeUsage {
                input_tokens: 7,
                output_tokens: 13,
            }),
            cascade_wire_version: Some("cascade.v2.1".to_string()),
        };
        let buf = encode_with_prefix(&original);
        let decoded = decode_response_frame(&buf).unwrap();
        assert_eq!(decoded.text_chunk.as_deref(), Some("hello world"));
        assert_eq!(decoded.finish_reason.as_deref(), Some("stop"));
        let usage = decoded.usage.unwrap();
        assert_eq!(usage.input_tokens, 7);
        assert_eq!(usage.output_tokens, 13);
    }

    /// (7) Compressed flag is rejected (no fixture coverage).
    #[test]
    fn compressed_flag_rejected() {
        let mut buf = Vec::new();
        buf.push(0x01); // compressed
        buf.extend_from_slice(&0u32.to_be_bytes());
        let buf = Bytes::from(buf);
        let err = decode_request_frame(&buf).unwrap_err();
        assert!(
            matches!(err, WindsurfCodecError::MissingField("gzip_unsupported")),
            "got: {err:?}"
        );
    }

    /// (8) decode_request_body: bypass prefix.
    #[test]
    fn decode_request_body_works_after_framing() {
        let original = CascadeRequest {
            messages: vec![],
            model_name: "claude-3.5-sonnet".to_string(),
            max_tokens: None,
            tool_declarations: vec![],
            workspace_id: None,
            cascade_wire_version: Some("cascade.v2.0".to_string()),
        };
        let mut payload = Vec::new();
        original.encode(&mut payload).unwrap();
        let decoded = decode_request_body(&payload).unwrap();
        assert_eq!(decoded.model_name, "claude-3.5-sonnet");
    }
}
