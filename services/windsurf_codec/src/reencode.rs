//! Byte-for-byte re-encode round-trip helpers.
//!
//! D18 SLICE 78/79: the codec MUST round-trip the wire bytes
//! byte-identically for frames SpendGuard did not modify, so vendor
//! extensions and unknown proto fields survive even though the
//! codec's typed surface doesn't know about them.
//!
//! Per D18 design.md §4.4: pass-through fallback is the load-bearing
//! contract — decode failure on a known wire version logs
//! `decoder_skipped` but still forwards the upstream response
//! unchanged. This module ships the writer side of that.

use std::io::Cursor;

use bytes::Bytes;
use prost::Message;

use crate::framing::{Frame, GrpcWebReader};
use crate::windsurf_proto::{CascadeRequest, CascadeResponseDelta};

/// Re-emit a single gRPC-Web frame to its on-wire form.
///
/// `[flags:u8][length:u32 BE][payload]`. The payload is the raw
/// bytes already on the frame — this function does NOT re-encode
/// the envelope. Use [`reencode_frame_with_payload`] when the
/// translator has actually modified the payload.
pub fn reencode_frame(frame: &Frame) -> Vec<u8> {
    let mut buf = Vec::with_capacity(5 + frame.payload.len());
    buf.push(frame.flags);
    buf.extend_from_slice(&(frame.payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(&frame.payload);
    buf
}

/// Re-emit a frame with a fresh payload.
///
/// Used when the translator has rewritten the envelope (e.g. SLICE
/// 78 MITM session reservation-gated message redaction).
pub fn reencode_frame_with_payload(orig: &Frame, new_payload: Bytes) -> Vec<u8> {
    let mut buf = Vec::with_capacity(5 + new_payload.len());
    buf.push(orig.flags);
    buf.extend_from_slice(&(new_payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(&new_payload);
    buf
}

/// Re-emit a stream of frames to their on-wire form.
pub fn reencode_frame_stream(wire: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut reader = GrpcWebReader::new(Cursor::new(wire));
    let mut out = Vec::with_capacity(wire.len());
    while let Some(frame) = reader.read_frame()? {
        out.extend_from_slice(&reencode_frame(&frame));
    }
    Ok(out)
}

/// Re-encode an entire `.windsurf-rpc` fixture: decode each frame's
/// envelope, re-encode it via prost, wrap in a fresh gRPC-Web frame.
pub fn reencode_decoded_stream_as_request(wire: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut reader = GrpcWebReader::new(Cursor::new(wire));
    let mut out = Vec::with_capacity(wire.len());
    while let Some(frame) = reader.read_frame()? {
        let payload = if frame.is_end_of_stream() {
            frame.payload.clone()
        } else {
            let req = CascadeRequest::decode(&frame.payload[..]).map_err(|e| {
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
/// frames as [`CascadeResponseDelta`]s.
pub fn reencode_decoded_stream_as_response(wire: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut reader = GrpcWebReader::new(Cursor::new(wire));
    let mut out = Vec::with_capacity(wire.len());
    while let Some(frame) = reader.read_frame()? {
        let payload = if frame.is_end_of_stream() {
            frame.payload.clone()
        } else {
            let delta = CascadeResponseDelta::decode(&frame.payload[..]).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("envelope decode failed: {e}"),
                )
            })?;
            let mut buf = Vec::new();
            delta
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
    use crate::framing::{FLAG_COMPRESSED, FLAG_END_OF_STREAM};
    use crate::windsurf_proto::CascadeMessage;

    fn wire_frame(flags: u8, payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(5 + payload.len());
        buf.push(flags);
        buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        buf.extend_from_slice(payload);
        buf
    }

    /// (1) Raw frame round-trip: byte-identical for arbitrary payload.
    #[test]
    fn raw_frame_round_trip_byte_identical() {
        let payload = b"\x01\x02\x03 arbitrary \xff\xfe";
        let on_wire = wire_frame(0x00, payload);
        let reencoded = reencode_frame_stream(&on_wire).unwrap();
        assert_eq!(reencoded, on_wire);
    }

    /// (2) Length prefix correctness across lengths.
    #[test]
    fn length_prefix_byte_identical_across_lengths() {
        for &len in &[0usize, 1, 5, 255, 256, 1024, 65536] {
            let payload = vec![0xabu8; len];
            let on_wire = wire_frame(0x00, &payload);
            let reencoded = reencode_frame_stream(&on_wire).unwrap();
            assert_eq!(reencoded, on_wire, "drift at length={len}");
        }
    }

    /// (3) Multi-frame stream + EOS round-trip.
    #[test]
    fn multi_frame_round_trip_with_eos() {
        let mut on_wire = Vec::new();
        on_wire.extend(wire_frame(0x00, b"first"));
        on_wire.extend(wire_frame(0x00, b"second"));
        on_wire.extend(wire_frame(FLAG_END_OF_STREAM, b"grpc-status:0"));
        let reencoded = reencode_frame_stream(&on_wire).unwrap();
        assert_eq!(reencoded, on_wire);
    }

    /// (4) Compressed flag preserved.
    #[test]
    fn compressed_flag_preserved() {
        let payload = b"compressed bytes";
        let on_wire = wire_frame(FLAG_COMPRESSED, payload);
        let reencoded = reencode_frame_stream(&on_wire).unwrap();
        assert_eq!(reencoded, on_wire);
    }

    /// (5) Synthetic request fixture envelope round-trip.
    #[test]
    fn synthetic_request_envelope_round_trip() {
        let original = CascadeRequest {
            messages: vec![CascadeMessage {
                role: "user".to_string(),
                content: "synthetic prompt".to_string(),
            }],
            model_name: "gpt-4o".to_string(),
            max_tokens: Some(128),
            tool_declarations: vec![],
            workspace_id: None,
            cascade_wire_version: Some("cascade.v2.0".to_string()),
        };
        let mut payload = Vec::new();
        original.encode(&mut payload).unwrap();
        let on_wire = wire_frame(0x00, &payload);

        let reencoded = reencode_decoded_stream_as_request(&on_wire).unwrap();
        assert_eq!(reencoded, on_wire);
    }

    /// (6) Modified payload re-encodes with correct length prefix.
    #[test]
    fn modified_messages_reencodes_with_correct_length_prefix() {
        let original = CascadeRequest {
            messages: vec![CascadeMessage {
                role: "user".to_string(),
                content: "original prompt".to_string(),
            }],
            model_name: "gpt-4o".to_string(),
            max_tokens: None,
            tool_declarations: vec![],
            workspace_id: None,
            cascade_wire_version: Some("cascade.v2.0".to_string()),
        };
        let mut orig_payload = Vec::new();
        original.encode(&mut orig_payload).unwrap();
        let orig_frame = Frame {
            flags: 0x00,
            payload: Bytes::from(orig_payload),
        };

        let mut modified = original.clone();
        modified.messages[0].content = "[REDACTED]".to_string();
        let mut new_payload = Vec::new();
        modified.encode(&mut new_payload).unwrap();

        let reencoded = reencode_frame_with_payload(&orig_frame, Bytes::from(new_payload.clone()));
        assert_eq!(reencoded[0], 0x00);
        let len_bytes: [u8; 4] = reencoded[1..5].try_into().unwrap();
        assert_eq!(u32::from_be_bytes(len_bytes) as usize, new_payload.len());
    }
}
