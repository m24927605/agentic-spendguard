//! Cursor wire envelope decode helpers.
//!
//! Each Connect-RPC data frame's payload is a serialised `CursorChatRequest`
//! (client → server unary request) or `CursorChatResponseChunk` (server →
//! client streaming chunk). This module wraps `prost::Message::decode`
//! with typed errors and shape validation.
//!
//! Per [`review-standards.md`](../../../docs/specs/coverage/D17_cursor_mitm/review-standards.md)
//! §4 (`W4`): vendor field additions are preserved transparently by
//! `prost` — proto3 unknown fields are silently dropped on decode in
//! prost 0.13, so the SLICE 7 byte-for-byte preservation contract is
//! enforced at the framing layer (re-emit the original wire bytes),
//! not at the envelope layer. SLICE 3 ships the typed decode path so
//! the SLICE 5 translator and the SLICE 6 reserve/commit path have
//! something to bind against.

use prost::Message;

use crate::cursor_proto::{CursorChatRequest, CursorChatResponseChunk};

/// Error returned when a Cursor envelope payload fails to decode.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// The prost decoder rejected the bytes — invalid varint, truncated
    /// nested message, unknown wire type, etc.
    #[error("cursor envelope decode failed: {0}")]
    Prost(#[from] prost::DecodeError),

    /// The decoded envelope was structurally valid protobuf but missing
    /// a field SpendGuard considers required (e.g. empty model string
    /// on a request). The SLICE 3 ruleset is intentionally narrow: only
    /// reject when the field is mandatory for the SLICE 5 translator to
    /// build a CanonicalChatRequest.
    #[error("cursor envelope missing required field: {field}")]
    MissingField {
        /// The proto field name that was empty / absent.
        field: &'static str,
    },
}

/// Decode a `CursorChatRequest` from a Connect-RPC data frame payload.
///
/// The caller is expected to have already validated framing (i.e. the
/// frame's flag byte is `0x00`, not `FLAG_COMPRESSED` or
/// `FLAG_END_OF_STREAM`). Compressed payloads MUST be decompressed before
/// calling — `decode_chat_request` does not inspect the Content-Encoding
/// header.
pub fn decode_chat_request(bytes: &[u8]) -> Result<CursorChatRequest, DecodeError> {
    let req = CursorChatRequest::decode(bytes)?;
    if req.model.is_empty() {
        return Err(DecodeError::MissingField { field: "model" });
    }
    Ok(req)
}

/// Decode a `CursorChatResponseChunk` from a Connect-RPC data frame payload.
///
/// Chunks are decoded individually as Cursor's server-streaming surface
/// emits them. The terminal end-of-stream frame is detected at the
/// framing layer (`Frame::is_end_of_stream`), so callers should NOT call
/// `decode_chat_response_chunk` on the trailers frame.
pub fn decode_chat_response_chunk(bytes: &[u8]) -> Result<CursorChatResponseChunk, DecodeError> {
    let chunk = CursorChatResponseChunk::decode(bytes)?;
    Ok(chunk)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor_proto::Message as CursorMessage;

    /// (1) Round-trip encode → decode of a minimal request.
    #[test]
    fn round_trip_minimal_request() {
        let original = CursorChatRequest {
            messages: vec![CursorMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            model: "claude-3.5-sonnet".to_string(),
            system: None,
            max_tokens: None,
            temperature: None,
        };
        let mut buf = Vec::new();
        original.encode(&mut buf).unwrap();

        let decoded = decode_chat_request(&buf).unwrap();
        assert_eq!(decoded.model, "claude-3.5-sonnet");
        assert_eq!(decoded.messages.len(), 1);
        assert_eq!(decoded.messages[0].role, "user");
        assert_eq!(decoded.messages[0].content, "hello");
    }

    /// (2) Round-trip encode → decode of a request with optional fields populated.
    #[test]
    fn round_trip_request_with_optionals() {
        let original = CursorChatRequest {
            messages: vec![
                CursorMessage {
                    role: "system".to_string(),
                    content: "be brief".to_string(),
                },
                CursorMessage {
                    role: "user".to_string(),
                    content: "what is 2+2".to_string(),
                },
            ],
            model: "gpt-4o".to_string(),
            system: Some("be brief".to_string()),
            max_tokens: Some(128),
            temperature: Some(0.5),
        };
        let mut buf = Vec::new();
        original.encode(&mut buf).unwrap();

        let decoded = decode_chat_request(&buf).unwrap();
        assert_eq!(decoded.model, "gpt-4o");
        assert_eq!(decoded.system.as_deref(), Some("be brief"));
        assert_eq!(decoded.max_tokens, Some(128));
        // float compare: prost preserves bit pattern of f32.
        assert_eq!(decoded.temperature, Some(0.5_f32));
        assert_eq!(decoded.messages.len(), 2);
    }

    /// (3) Malformed bytes are rejected with `DecodeError::Prost`.
    #[test]
    fn malformed_bytes_rejected() {
        // 0xff 0xff is not a valid protobuf varint prefix for any field
        // tag we expect.
        let bad = [0xffu8; 16];
        let err = decode_chat_request(&bad).unwrap_err();
        assert!(
            matches!(err, DecodeError::Prost(_)),
            "expected Prost decode error, got: {err}"
        );
    }

    /// (4) Empty `model` field is rejected with `MissingField`.
    #[test]
    fn empty_model_rejected_as_missing_field() {
        let original = CursorChatRequest {
            messages: vec![CursorMessage {
                role: "user".to_string(),
                content: "x".to_string(),
            }],
            model: String::new(), // explicitly empty
            system: None,
            max_tokens: None,
            temperature: None,
        };
        let mut buf = Vec::new();
        original.encode(&mut buf).unwrap();

        let err = decode_chat_request(&buf).unwrap_err();
        match err {
            DecodeError::MissingField { field } => assert_eq!(field, "model"),
            other => panic!("expected MissingField, got: {other}"),
        }
    }

    /// (5) Round-trip encode → decode of a streaming response chunk.
    #[test]
    fn round_trip_response_chunk() {
        let original = CursorChatResponseChunk {
            model: "claude-3.5-sonnet".to_string(),
            delta: "hello ".to_string(),
            finish_reason: None,
            cumulative_output_tokens: Some(2),
        };
        let mut buf = Vec::new();
        original.encode(&mut buf).unwrap();

        let decoded = decode_chat_response_chunk(&buf).unwrap();
        assert_eq!(decoded.delta, "hello ");
        assert_eq!(decoded.model, "claude-3.5-sonnet");
        assert_eq!(decoded.finish_reason, None);
        assert_eq!(decoded.cumulative_output_tokens, Some(2));
    }

    /// (6) Terminal chunk carries finish_reason.
    #[test]
    fn terminal_chunk_finish_reason() {
        let original = CursorChatResponseChunk {
            model: String::new(),
            delta: String::new(),
            finish_reason: Some("stop".to_string()),
            cumulative_output_tokens: Some(42),
        };
        let mut buf = Vec::new();
        original.encode(&mut buf).unwrap();

        let decoded = decode_chat_response_chunk(&buf).unwrap();
        assert_eq!(decoded.finish_reason.as_deref(), Some("stop"));
        assert_eq!(decoded.cumulative_output_tokens, Some(42));
    }

    /// (7) Partial decode: malformed response chunk rejected.
    #[test]
    fn malformed_response_chunk_rejected() {
        // Truncated varint — prost will error out partway through.
        let bad = [0x08, 0xff]; // tag 1 (varint) followed by an invalid varint
        let err = decode_chat_response_chunk(&bad).unwrap_err();
        assert!(
            matches!(err, DecodeError::Prost(_)),
            "expected Prost decode error, got: {err}"
        );
    }
}
