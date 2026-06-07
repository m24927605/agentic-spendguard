//! Byte-for-byte re-encode round-trip tests.
//!
//! D17 SLICE 7 ([`design.md`](../../docs/specs/coverage/D17_cursor_mitm/design.md)
//! §8 decision 3 + [`implementation.md`](../../docs/specs/coverage/D17_cursor_mitm/implementation.md)
//! §4): the codec MUST round-trip the wire bytes byte-identically for
//! frames SpendGuard did not modify, so vendor extensions and unknown
//! proto fields survive even though the codec's typed surface doesn't
//! know about them.
//!
//! ## Layered contract
//!
//! Three layers must each preserve byte-identity independently:
//!
//! 1. **Framing layer** — [`crate::framing`] reads raw payload bytes;
//!    a writer that re-emits `[flag][len BE][payload]` byte-for-byte
//!    requires no decode work on the payload, only on the framing
//!    metadata. This module ships [`reencode_frame`] for that.
//! 2. **Envelope layer** — prost 0.13 deterministically encodes proto3
//!    messages with no unknown fields. For messages that were decoded
//!    from a `prost::Message::decode` call and re-encoded without
//!    modification, the output is byte-identical *only when* the
//!    original was emitted by a prost-compatible encoder with no
//!    unknown fields. Real Cursor wire bytes can carry unknown fields
//!    (vendor additions) — those are silently dropped by prost 0.13's
//!    default codegen, which is the documented SLICE 7 limitation
//!    handled by the [`reencode_frame`] short-circuit: when the
//!    intent is "preserve unchanged", we re-emit the raw payload
//!    bytes; we only re-encode the envelope when the translator has
//!    actually modified a field.
//! 3. **Streaming layer** — concatenated frames in a `.cursor-rpc`
//!    fixture round-trip exactly when each frame round-trips and the
//!    frame order is preserved. [`reencode_frame_stream`] exercises
//!    that for the synthetic fixtures.
//!
//! ## What "byte-identical" means in SLICE 7
//!
//! For the synthetic fixtures shipped at SLICE 4 (no unknown fields,
//! all proto3 default decodes), the full chain
//! decode → re-encode → bytes is byte-identical.
//!
//! For real Cursor wire bytes with vendor unknown fields (SLICE 8
//! corpus), the framing layer preserves them via the raw-payload
//! short-circuit; the envelope layer drops them on translator
//! modifications. The SLICE 7 contract is that *modified frames are
//! byte-equivalent on the fields SpendGuard knows about*, and
//! *unmodified frames are byte-identical full stop*.

use std::io::Cursor;

use bytes::Bytes;
use prost::Message;

use crate::cursor_proto::{CursorChatRequest, CursorChatResponseChunk};
use crate::framing::{ConnectRpcReader, Frame};

/// Re-emit a single Connect-RPC frame to its on-wire form.
///
/// `[flags:u8][length:u32 BE][payload]`. The payload is the raw bytes
/// already on the frame — this function does NOT re-encode the
/// envelope. Use [`reencode_frame_with_payload`] when the translator
/// has actually modified the payload.
pub fn reencode_frame(frame: &Frame) -> Vec<u8> {
    let mut buf = Vec::with_capacity(5 + frame.payload.len());
    buf.push(frame.flags);
    buf.extend_from_slice(&(frame.payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(&frame.payload);
    buf
}

/// Re-emit a frame with a fresh payload.
///
/// Used when the translator has rewritten the envelope (e.g. SLICE 6
/// MITM session reservation-gated message redaction). The new payload
/// is rewritten into a new `[flag][len BE][payload]` triple; the flag
/// is copied from the original frame.
pub fn reencode_frame_with_payload(orig: &Frame, new_payload: Bytes) -> Vec<u8> {
    let mut buf = Vec::with_capacity(5 + new_payload.len());
    buf.push(orig.flags);
    buf.extend_from_slice(&(new_payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(&new_payload);
    buf
}

/// Re-emit a stream of frames to their on-wire form.
///
/// Used by the SLICE 7 stream-level round-trip tests. The stream
/// reader (`Cursor<Vec<u8>>`) is consumed; the writer emits the same
/// bytes per [`reencode_frame`].
pub fn reencode_frame_stream(wire: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut reader = ConnectRpcReader::new(Cursor::new(wire));
    let mut out = Vec::with_capacity(wire.len());
    while let Some(frame) = reader.read_frame()? {
        out.extend_from_slice(&reencode_frame(&frame));
    }
    Ok(out)
}

/// Re-encode an entire `.cursor-rpc` fixture: decode each frame's
/// envelope, re-encode it via prost, wrap in a fresh Connect-RPC
/// frame.
///
/// For the synthetic fixtures (no unknown fields), this produces
/// byte-identical output. For real wire bytes the output is field-
/// equivalent but may differ on prost-vs-vendor wire-formatting
/// quirks (varint length minimisation, repeated-field merging). The
/// SLICE 7 contract handles that by preferring [`reencode_frame`]
/// over this function when the translator has not touched the
/// envelope.
pub fn reencode_decoded_stream_as_request(wire: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut reader = ConnectRpcReader::new(Cursor::new(wire));
    let mut out = Vec::with_capacity(wire.len());
    while let Some(frame) = reader.read_frame()? {
        // The framing layer doesn't know whether the payload is a
        // request or response chunk — the caller does. This helper
        // assumes request shape and is exercised by the request
        // round-trip tests below.
        let payload = if frame.is_end_of_stream() {
            // End-of-stream frames carry trailers / metadata, not an
            // envelope; preserve verbatim.
            frame.payload.clone()
        } else {
            let req = CursorChatRequest::decode(&frame.payload[..]).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("envelope decode failed: {e}"),
                )
            })?;
            let mut buf = Vec::new();
            req.encode(&mut buf)
                .map_err(|e| std::io::Error::other(format!("envelope encode failed: {e}")))?;
            Bytes::from(buf)
        };
        out.extend_from_slice(&reencode_frame_with_payload(&frame, payload));
    }
    Ok(out)
}

/// Same as [`reencode_decoded_stream_as_request`] but treats data
/// frames as [`CursorChatResponseChunk`]s. Used by streaming-side
/// tests.
pub fn reencode_decoded_stream_as_response(wire: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut reader = ConnectRpcReader::new(Cursor::new(wire));
    let mut out = Vec::with_capacity(wire.len());
    while let Some(frame) = reader.read_frame()? {
        let payload = if frame.is_end_of_stream() {
            frame.payload.clone()
        } else {
            let chunk = CursorChatResponseChunk::decode(&frame.payload[..]).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("envelope decode failed: {e}"),
                )
            })?;
            let mut buf = Vec::new();
            chunk
                .encode(&mut buf)
                .map_err(|e| std::io::Error::other(format!("envelope encode failed: {e}")))?;
            Bytes::from(buf)
        };
        out.extend_from_slice(&reencode_frame_with_payload(&frame, payload));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor_proto::Message as CursorMessage;
    use crate::framing::{FLAG_COMPRESSED, FLAG_END_OF_STREAM};

    /// Helper: build a single on-wire frame.
    fn wire_frame(flags: u8, payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(5 + payload.len());
        buf.push(flags);
        buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        buf.extend_from_slice(payload);
        buf
    }

    /// (1) Raw frame round-trip: byte-identical for arbitrary payload bytes.
    ///
    /// Synthetic check that the framing reader/writer don't introduce
    /// drift. Independent of envelope decode.
    #[test]
    fn raw_frame_round_trip_byte_identical() {
        let payload = b"\x01\x02\x03 arbitrary \xff\xfe";
        let on_wire = wire_frame(0x00, payload);
        let reencoded = reencode_frame_stream(&on_wire).unwrap();
        assert_eq!(reencoded, on_wire);
    }

    /// (2) Length-prefix correctness: 1-byte, 256-byte, 65536-byte payloads.
    #[test]
    fn length_prefix_byte_identical_across_lengths() {
        for &len in &[0usize, 1, 5, 255, 256, 1024, 65536] {
            let payload = vec![0xabu8; len];
            let on_wire = wire_frame(0x00, &payload);
            let reencoded = reencode_frame_stream(&on_wire).unwrap();
            assert_eq!(
                reencoded,
                on_wire,
                "round-trip drifted for length={len}: \
                 expected first 5 bytes {:?}, got {:?}",
                &on_wire[..5.min(on_wire.len())],
                &reencoded[..5.min(reencoded.len())]
            );
        }
    }

    /// (3) Multi-frame stream round-trip with EOS marker.
    #[test]
    fn multi_frame_round_trip_with_eos() {
        let mut on_wire = Vec::new();
        on_wire.extend(wire_frame(0x00, b"first"));
        on_wire.extend(wire_frame(0x00, b"second"));
        on_wire.extend(wire_frame(FLAG_END_OF_STREAM, b"grpc-status:0"));
        let reencoded = reencode_frame_stream(&on_wire).unwrap();
        assert_eq!(reencoded, on_wire);
    }

    /// (4) Compressed flag preserved in the round-trip.
    #[test]
    fn compressed_flag_preserved() {
        let payload = b"compressed bytes";
        let on_wire = wire_frame(FLAG_COMPRESSED, payload);
        let reencoded = reencode_frame_stream(&on_wire).unwrap();
        assert_eq!(reencoded, on_wire);
        // Sanity-check the recoded flag byte.
        assert_eq!(reencoded[0], FLAG_COMPRESSED);
    }

    /// (5) Synthetic unary request fixture round-trips byte-identically
    /// at the envelope level (decode → re-encode → bytes).
    #[test]
    fn synthetic_unary_request_envelope_round_trip() {
        // Build the same shape as the synthetic_unary fixture.
        let original = CursorChatRequest {
            messages: vec![CursorMessage {
                role: "user".to_string(),
                content: "synthetic prompt".to_string(),
            }],
            model: "gpt-4o-mini".to_string(),
            system: None,
            max_tokens: Some(128),
            temperature: Some(0.5),
        };
        let mut payload = Vec::new();
        original.encode(&mut payload).unwrap();
        let on_wire = wire_frame(0x00, &payload);

        let reencoded = reencode_decoded_stream_as_request(&on_wire).unwrap();
        assert_eq!(
            reencoded, on_wire,
            "envelope round-trip drifted for synthetic_unary"
        );
    }

    /// (6) Synthetic streaming response chunk fixture round-trips
    /// byte-identically at the envelope level.
    #[test]
    fn synthetic_streaming_chunk_envelope_round_trip() {
        let original = CursorChatResponseChunk {
            model: "gpt-4o-mini".to_string(),
            delta: "hello".to_string(),
            finish_reason: None,
            cumulative_output_tokens: Some(2),
        };
        let mut payload = Vec::new();
        original.encode(&mut payload).unwrap();
        let mut on_wire = wire_frame(0x00, &payload);

        // Append a terminal chunk + EOS marker.
        let terminal = CursorChatResponseChunk {
            model: String::new(),
            delta: String::new(),
            finish_reason: Some("stop".to_string()),
            cumulative_output_tokens: Some(42),
        };
        let mut term_payload = Vec::new();
        terminal.encode(&mut term_payload).unwrap();
        on_wire.extend(wire_frame(0x00, &term_payload));
        on_wire.extend(wire_frame(FLAG_END_OF_STREAM, b""));

        let reencoded = reencode_decoded_stream_as_response(&on_wire).unwrap();
        assert_eq!(
            reencoded, on_wire,
            "envelope round-trip drifted for synthetic_streaming"
        );
    }

    /// (7) Modified messages list: re-encode with a redacted message
    /// rewrites the bytes, length prefix is correct, and decode of
    /// re-encoded bytes yields the modified message.
    #[test]
    fn modified_messages_reencodes_with_correct_length_prefix() {
        let original = CursorChatRequest {
            messages: vec![CursorMessage {
                role: "user".to_string(),
                content: "original prompt with PII".to_string(),
            }],
            model: "gpt-4o".to_string(),
            system: None,
            max_tokens: None,
            temperature: None,
        };
        let mut orig_payload = Vec::new();
        original.encode(&mut orig_payload).unwrap();
        let orig_frame = Frame {
            flags: 0x00,
            payload: Bytes::from(orig_payload),
        };

        // Rewrite the request with the message redacted.
        let mut modified = original.clone();
        modified.messages[0].content = "[REDACTED]".to_string();
        let mut new_payload = Vec::new();
        modified.encode(&mut new_payload).unwrap();

        let reencoded = reencode_frame_with_payload(&orig_frame, Bytes::from(new_payload.clone()));

        // Sanity: first byte is the flag, next 4 are the length BE.
        assert_eq!(reencoded[0], 0x00);
        let len_bytes: [u8; 4] = reencoded[1..5].try_into().unwrap();
        let length = u32::from_be_bytes(len_bytes);
        assert_eq!(length as usize, new_payload.len());

        // Decode the re-encoded frame and check the redaction landed.
        let mut reader = ConnectRpcReader::new(Cursor::new(reencoded));
        let frame = reader.read_frame().unwrap().unwrap();
        let decoded = CursorChatRequest::decode(&frame.payload[..]).unwrap();
        assert_eq!(decoded.messages[0].content, "[REDACTED]");
    }

    /// (8) Modified temperature round-trip: only the temperature field
    /// changes; the modified frame decodes correctly.
    #[test]
    fn modified_temperature_reencodes_correctly() {
        let original = CursorChatRequest {
            messages: vec![CursorMessage {
                role: "user".to_string(),
                content: "calm down".to_string(),
            }],
            model: "gpt-4o".to_string(),
            system: None,
            max_tokens: None,
            temperature: Some(1.5), // too hot
        };
        let mut payload = Vec::new();
        original.encode(&mut payload).unwrap();
        let frame = Frame {
            flags: 0x00,
            payload: Bytes::from(payload),
        };

        // Tame the temperature.
        let mut tame = original.clone();
        tame.temperature = Some(0.2);
        let mut tame_payload = Vec::new();
        tame.encode(&mut tame_payload).unwrap();

        let reencoded = reencode_frame_with_payload(&frame, Bytes::from(tame_payload));
        let mut reader = ConnectRpcReader::new(Cursor::new(reencoded));
        let decoded_frame = reader.read_frame().unwrap().unwrap();
        let decoded = CursorChatRequest::decode(&decoded_frame.payload[..]).unwrap();
        assert_eq!(decoded.temperature, Some(0.2));
    }

    /// (9) Length-prefix correctness: a frame with a 65536-byte payload
    /// gets the correct big-endian u32 length and the byte after the
    /// prefix is the first payload byte.
    #[test]
    fn length_prefix_correctness_at_65k() {
        let big_payload = vec![0x42u8; 65536];
        let on_wire = wire_frame(0x00, &big_payload);
        // First 5 bytes: flag + length BE.
        assert_eq!(on_wire[0], 0x00);
        assert_eq!(&on_wire[1..5], &(65536u32).to_be_bytes());
        // 6th byte (index 5) is the first payload byte.
        assert_eq!(on_wire[5], 0x42);

        let reencoded = reencode_frame_stream(&on_wire).unwrap();
        assert_eq!(reencoded.len(), on_wire.len());
        assert_eq!(&reencoded[..5], &on_wire[..5]);
        assert_eq!(reencoded[5], 0x42);
    }
}
