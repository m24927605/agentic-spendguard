//! `.windsurf-rpc` fixture replay harness.
//!
//! D18 SLICE 80 — mirrors D17 cursor_codec::replay. The codec MUST
//! be exercised against a committed `.windsurf-rpc` fixture corpus
//! without ever touching live `server.codeium.com` traffic. This
//! module owns the reader, the replay pipeline, and the
//! [`ReplayReport`] envelope every fixture test consumes.
//!
//! ## On-disk fixture layout
//!
//! Per `fixtures/README.md` (mirrors D17 cursor-rpc layout):
//!
//! ```text
//! +----------------------+-------------------+--------------------+---------------------+
//! | magic = b"SGWRPC\0\0" | version (u16 LE) | frame count (u32 LE) |  reserved (u16 LE)  |
//! |       8 bytes         |     2 bytes      |       4 bytes        |       2 bytes       |
//! +----------------------+-------------------+--------------------+---------------------+
//!
//! Per-frame record (repeated frame_count times):
//!
//! +--------------------+----------------------+----------------------+---------------------+---------------------+
//! | timestamp_ms (u64) | direction (u8)       | rpc_flag (u8)        | length (u32 BE)     | payload (length B)  |
//! |      8 bytes       |  0=client  1=server  |  gRPC-Web flag       |   gRPC-Web bytes    |                     |
//! +--------------------+----------------------+----------------------+---------------------+---------------------+
//! ```

use std::fs;
use std::io::Cursor;
use std::path::Path;

use bytes::Bytes;
use prost::Message as ProstMessage;
use thiserror::Error;

use crate::envelope::{decode_request_body, decode_response_body};
use crate::error::WindsurfCodecError;
use crate::framing::{Frame, GrpcWebReader};
use crate::openai_models::OpenAiChatRequest;
use crate::reencode::reencode_frame;
use crate::translate::cascade_request_to_openai;
use crate::windsurf_proto::{CascadeRequest, CascadeResponseDelta};

/// `.windsurf-rpc` magic bytes — 8 bytes, big-endian opaque tag.
pub const FIXTURE_MAGIC: &[u8; 8] = b"SGWRPC\0\0";
/// Envelope version the reader/writer currently support.
pub const FIXTURE_VERSION: u16 = 1;

/// `direction` byte on per-frame records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Client → server (Windsurf IDE → `server.codeium.com`).
    Client,
    /// Server → client (`server.codeium.com` → Windsurf IDE).
    Server,
}

impl Direction {
    fn from_byte(b: u8) -> Result<Self, ReplayError> {
        match b {
            0 => Ok(Direction::Client),
            1 => Ok(Direction::Server),
            other => Err(ReplayError::InvalidDirection(other)),
        }
    }
}

/// One frame record inside a `.windsurf-rpc` fixture.
#[derive(Debug, Clone)]
pub struct FixtureFrame {
    /// UNIX epoch milliseconds at capture / synthesis time.
    pub timestamp_ms: u64,
    /// Direction on the wire.
    pub direction: Direction,
    /// The gRPC-Web frame as it would appear on the wire.
    pub frame: Frame,
}

/// What a single fixture replay observed.
#[derive(Debug, Clone, Default)]
pub struct ReplayReport {
    /// Envelope `version` field.
    pub version: u16,
    /// Envelope `frame_count` field.
    pub frame_count: u32,
    /// Total frame records read off disk.
    pub frames_read: u32,
    /// Request frames decoded successfully.
    pub request_frames_decoded: u32,
    /// Response data frames decoded successfully.
    pub response_frames_decoded: u32,
    /// Trailers / end-of-stream frames seen on the wire.
    pub end_of_stream_frames: u32,
    /// Frames with the gRPC-Web compressed flag set.
    pub compressed_frames: u32,
    /// Maximum cumulative output-token count read from the terminal
    /// `usage.output_tokens` of the response stream.
    pub cumulative_output_tokens: Option<u32>,
    /// `true` when the first request frame round-trips byte-identical
    /// through [`reencode_frame`].
    pub request_bytes_round_trip: bool,
    /// `true` when **every** frame round-trips byte-identical at the
    /// framing layer.
    pub all_frames_round_trip: bool,
    /// Translated canonical OpenAI shape from the first request frame.
    pub translated_request: Option<OpenAiChatRequest>,
    /// Mocked sidecar reserve calls observed during replay.
    pub sidecar_reserve_calls: u32,
    /// Mocked sidecar commit calls observed during replay.
    pub sidecar_commit_calls: u32,
    /// `finish_reason` extracted from the terminal response delta.
    pub finish_reason: Option<String>,
    /// `true` when the fixture intentionally carries an upstream
    /// error trailers blob (e.g. `grpc-status:13`).
    pub upstream_error: bool,
    /// Decoded request envelopes (typed).
    pub decoded_requests: Vec<CascadeRequest>,
    /// Decoded response deltas (typed).
    pub decoded_responses: Vec<CascadeResponseDelta>,
    /// Set to `true` when at least one frame's wire-version stamp
    /// was rejected as unknown (the `cascade_chat_unknown_wire_version`
    /// fixture exercises this).
    pub unsupported_wire_version_seen: bool,
    /// Set to `true` when a known-version frame failed to decode at
    /// the prost layer (the `cascade_chat_truncated` fixture).
    pub decoder_skipped: bool,
}

/// Errors the replay harness emits.
#[derive(Debug, Error)]
pub enum ReplayError {
    /// I/O error reading the fixture file.
    #[error("fixture I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Magic bytes did not match [`FIXTURE_MAGIC`].
    #[error("fixture magic mismatch: got {got:?}, want {want:?}")]
    BadMagic {
        /// What was actually on disk.
        got: [u8; 8],
        /// What the reader expected.
        want: [u8; 8],
    },

    /// Envelope version was not [`FIXTURE_VERSION`].
    #[error("fixture version mismatch: got {got}, want {want}")]
    BadVersion {
        /// Version byte read off disk.
        got: u16,
        /// Version the reader supports.
        want: u16,
    },

    /// Envelope `frame_count` did not match number of frames found.
    #[error("fixture frame_count mismatch: header={header}, actual={actual}")]
    FrameCountMismatch {
        /// Count claimed by the envelope.
        header: u32,
        /// Actual number of per-frame records that decoded.
        actual: u32,
    },

    /// Direction byte was neither 0 nor 1.
    #[error("invalid direction byte: {0}")]
    InvalidDirection(u8),

    /// Codec-level decode failure escalated from
    /// [`WindsurfCodecError`].
    #[error("codec decode error: {0}")]
    Codec(#[from] WindsurfCodecError),
}

/// Read a `.windsurf-rpc` fixture from disk.
pub fn read_fixture(path: &Path) -> Result<(u16, Vec<FixtureFrame>), ReplayError> {
    let bytes = fs::read(path)?;
    read_fixture_bytes(&bytes)
}

/// Read a `.windsurf-rpc` fixture from in-memory bytes.
pub fn read_fixture_bytes(bytes: &[u8]) -> Result<(u16, Vec<FixtureFrame>), ReplayError> {
    if bytes.len() < 16 {
        return Err(ReplayError::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            format!("fixture too short for envelope: {} bytes", bytes.len()),
        )));
    }

    let mut magic = [0u8; 8];
    magic.copy_from_slice(&bytes[..8]);
    if &magic != FIXTURE_MAGIC {
        return Err(ReplayError::BadMagic {
            got: magic,
            want: *FIXTURE_MAGIC,
        });
    }

    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if version != FIXTURE_VERSION {
        return Err(ReplayError::BadVersion {
            got: version,
            want: FIXTURE_VERSION,
        });
    }

    let frame_count = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]);

    let mut offset: usize = 16;
    let mut frames = Vec::with_capacity(frame_count as usize);
    let mut frames_seen: u32 = 0;

    while offset < bytes.len() {
        if offset + 14 > bytes.len() {
            return Err(ReplayError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!(
                    "fixture truncated at offset {offset}: expected 14 bytes of per-frame header"
                ),
            )));
        }

        let timestamp_ms = u64::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]);
        offset += 8;

        let direction = Direction::from_byte(bytes[offset])?;
        offset += 1;

        let frame_start = offset;
        let rpc_flag = bytes[offset];
        let length = u32::from_be_bytes([
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
            bytes[offset + 4],
        ]);
        let frame_end = frame_start + 5 + length as usize;
        if frame_end > bytes.len() {
            return Err(ReplayError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!(
                    "fixture frame at offset {offset}: declared length {length} \
                     exceeds remaining bytes ({})",
                    bytes.len() - frame_start - 5
                ),
            )));
        }
        let frame_bytes = &bytes[frame_start..frame_end];

        let mut reader = GrpcWebReader::new(Cursor::new(frame_bytes.to_vec()));
        let parsed = reader
            .read_frame()
            .map_err(ReplayError::Io)?
            .ok_or_else(|| {
                ReplayError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("fixture frame at offset {offset}: GrpcWebReader yielded no frame"),
                ))
            })?;

        debug_assert_eq!(parsed.flags, rpc_flag);
        debug_assert_eq!(parsed.payload.len() as u32, length);

        frames.push(FixtureFrame {
            timestamp_ms,
            direction,
            frame: parsed,
        });
        offset = frame_end;
        frames_seen += 1;
    }

    if frames_seen != frame_count {
        return Err(ReplayError::FrameCountMismatch {
            header: frame_count,
            actual: frames_seen,
        });
    }

    Ok((version, frames))
}

/// Replay a `.windsurf-rpc` fixture from disk.
pub fn replay_fixture(path: &Path) -> Result<ReplayReport, ReplayError> {
    crate::assert_experimental_banner_emitted();
    let (version, frames) = read_fixture(path)?;
    replay_frames(version, &frames)
}

/// Replay a `.windsurf-rpc` fixture from in-memory bytes.
pub fn replay_fixture_bytes(bytes: &[u8]) -> Result<ReplayReport, ReplayError> {
    crate::assert_experimental_banner_emitted();
    let (version, frames) = read_fixture_bytes(bytes)?;
    replay_frames(version, &frames)
}

fn replay_frames(version: u16, frames: &[FixtureFrame]) -> Result<ReplayReport, ReplayError> {
    let mut report = ReplayReport {
        version,
        frame_count: frames.len() as u32,
        frames_read: frames.len() as u32,
        all_frames_round_trip: true,
        ..ReplayReport::default()
    };

    for ff in frames {
        if ff.frame.is_end_of_stream() {
            report.end_of_stream_frames += 1;
            let pl = ff.frame.payload.as_ref();
            if let Some(pos) = find_subslice(pl, b"grpc-status:") {
                let after = &pl[pos + b"grpc-status:".len()..];
                if let Some(&first) = after.iter().find(|&&b| b.is_ascii_digit()) {
                    if first != b'0' {
                        report.upstream_error = true;
                    }
                }
            }
            continue;
        }
        if ff.frame.is_compressed() {
            report.compressed_frames += 1;
            continue;
        }
        match ff.direction {
            Direction::Client => {
                match decode_request_body(ff.frame.payload.as_ref()) {
                    Ok(req) => {
                        // Per design.md §3 decision 5: the unknown wire
                        // version fixture intentionally embeds a stamp
                        // outside the known set so we exercise the
                        // gating path here.
                        if let Some(v) = req.cascade_wire_version.as_deref() {
                            if !crate::version::KNOWN_WIRE_VERSIONS.contains(&v) {
                                report.unsupported_wire_version_seen = true;
                                continue;
                            }
                        }
                        report.decoded_requests.push(req);
                        report.request_frames_decoded += 1;
                    }
                    Err(WindsurfCodecError::Protobuf(_)) => {
                        // Per design.md §4.4: known-version decode
                        // failure on the request side degrades to
                        // `decoder_skipped` — codec is best-effort
                        // gating. The truncated fixture exercises
                        // this.
                        report.decoder_skipped = true;
                        continue;
                    }
                    Err(e) => return Err(ReplayError::Codec(e)),
                }
            }
            Direction::Server => match decode_response_body(ff.frame.payload.as_ref()) {
                Ok(delta) => {
                    if let Some(reason) = delta.finish_reason.as_ref() {
                        report.finish_reason = Some(reason.clone());
                    }
                    if let Some(usage) = delta.usage.as_ref() {
                        report.cumulative_output_tokens = Some(usage.output_tokens);
                    }
                    report.decoded_responses.push(delta);
                    report.response_frames_decoded += 1;
                }
                Err(WindsurfCodecError::Protobuf(_)) => {
                    report.decoder_skipped = true;
                    continue;
                }
                Err(e) => return Err(ReplayError::Codec(e)),
            },
        }
    }

    if let Some(first_req) = report.decoded_requests.first() {
        report.translated_request = Some(cascade_request_to_openai(first_req));
    }

    report.sidecar_reserve_calls = report.request_frames_decoded;
    if report.finish_reason.is_some() && !report.upstream_error {
        report.sidecar_commit_calls = report.request_frames_decoded;
    }

    let mut first_req_round_trip = None;
    for ff in frames {
        let original_wire = on_wire_bytes(&ff.frame);
        let reencoded = reencode_frame(&ff.frame);
        let matches = reencoded == original_wire;
        if !matches {
            report.all_frames_round_trip = false;
        }
        if ff.direction == Direction::Client && first_req_round_trip.is_none() {
            first_req_round_trip = Some(matches);
        }
    }
    report.request_bytes_round_trip = first_req_round_trip.unwrap_or(true);

    Ok(report)
}

fn on_wire_bytes(frame: &Frame) -> Bytes {
    let mut out = Vec::with_capacity(5 + frame.payload.len());
    out.push(frame.flags);
    out.extend_from_slice(&(frame.payload.len() as u32).to_be_bytes());
    out.extend_from_slice(&frame.payload);
    Bytes::from(out)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

// ============================================================================
// Fixture writer (used by tests + the SLICE 80 regenerate_fixtures example)
// ============================================================================

/// Build the on-disk bytes for a `.windsurf-rpc` fixture given the
/// frame records.
pub fn write_fixture_bytes(frames: &[FixtureFrame]) -> Vec<u8> {
    let mut out = Vec::with_capacity(
        16 + frames
            .iter()
            .map(|f| 14 + f.frame.payload.len())
            .sum::<usize>(),
    );
    out.extend_from_slice(FIXTURE_MAGIC);
    out.extend_from_slice(&FIXTURE_VERSION.to_le_bytes());
    out.extend_from_slice(&(frames.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());

    for ff in frames {
        out.extend_from_slice(&ff.timestamp_ms.to_le_bytes());
        out.push(match ff.direction {
            Direction::Client => 0,
            Direction::Server => 1,
        });
        out.push(ff.frame.flags);
        out.extend_from_slice(&(ff.frame.payload.len() as u32).to_be_bytes());
        out.extend_from_slice(&ff.frame.payload);
    }
    out
}

/// Build a [`FixtureFrame`] from a request envelope + timestamp.
pub fn request_frame(timestamp_ms: u64, req: &CascadeRequest) -> FixtureFrame {
    let mut payload = Vec::new();
    req.encode(&mut payload).expect("encode request");
    FixtureFrame {
        timestamp_ms,
        direction: Direction::Client,
        frame: Frame {
            flags: 0x00,
            payload: Bytes::from(payload),
        },
    }
}

/// Build a [`FixtureFrame`] from a response delta envelope + timestamp.
pub fn response_delta_frame(
    timestamp_ms: u64,
    delta: &CascadeResponseDelta,
    flags: u8,
) -> FixtureFrame {
    let mut payload = Vec::new();
    delta.encode(&mut payload).expect("encode response delta");
    FixtureFrame {
        timestamp_ms,
        direction: Direction::Server,
        frame: Frame {
            flags,
            payload: Bytes::from(payload),
        },
    }
}

/// Build a trailers / end-of-stream [`FixtureFrame`].
pub fn trailers_frame(timestamp_ms: u64, payload: Bytes) -> FixtureFrame {
    FixtureFrame {
        timestamp_ms,
        direction: Direction::Server,
        frame: Frame {
            flags: crate::framing::FLAG_END_OF_STREAM,
            payload,
        },
    }
}

/// Build a frame with a raw-byte payload (used for the truncated
/// fixture where we deliberately emit a body that won't decode).
pub fn raw_frame(
    timestamp_ms: u64,
    direction: Direction,
    flags: u8,
    payload: Bytes,
) -> FixtureFrame {
    FixtureFrame {
        timestamp_ms,
        direction,
        frame: Frame { flags, payload },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windsurf_proto::{CascadeMessage, CascadeUsage};

    fn minimal_request() -> CascadeRequest {
        CascadeRequest {
            messages: vec![CascadeMessage {
                role: "user".to_string(),
                content: "ping".to_string(),
            }],
            model_name: "gpt-4o".to_string(),
            max_tokens: Some(16),
            tool_declarations: vec![],
            workspace_id: None,
            cascade_wire_version: Some("cascade.v2.0".to_string()),
        }
    }

    fn minimal_delta(
        text: Option<&str>,
        finish: Option<&str>,
        tokens: Option<(u32, u32)>,
    ) -> CascadeResponseDelta {
        CascadeResponseDelta {
            model_name: "gpt-4o".to_string(),
            text_chunk: text.map(|s| s.to_string()),
            finish_reason: finish.map(|s| s.to_string()),
            usage: tokens.map(|(input, output)| CascadeUsage {
                input_tokens: input,
                output_tokens: output,
            }),
            cascade_wire_version: Some("cascade.v2.0".to_string()),
        }
    }

    /// (1) Envelope round-trip: write + read produce the same frames.
    #[test]
    fn envelope_round_trip() {
        let frames = vec![
            request_frame(1_700_000_000_000, &minimal_request()),
            response_delta_frame(
                1_700_000_000_010,
                &minimal_delta(Some("ok"), Some("stop"), Some((5, 1))),
                0x00,
            ),
            trailers_frame(1_700_000_000_020, Bytes::from_static(b"grpc-status:0")),
        ];
        let bytes = write_fixture_bytes(&frames);
        let (version, parsed) = read_fixture_bytes(&bytes).unwrap();
        assert_eq!(version, FIXTURE_VERSION);
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].direction, Direction::Client);
        assert_eq!(parsed[1].direction, Direction::Server);
        assert!(parsed[2].frame.is_end_of_stream());
    }

    /// (2) Bad magic rejected.
    #[test]
    fn bad_magic_rejected() {
        let mut bytes = write_fixture_bytes(&[]);
        bytes[0] = b'X';
        let err = read_fixture_bytes(&bytes).unwrap_err();
        assert!(matches!(err, ReplayError::BadMagic { .. }), "{err:?}");
    }

    /// (3) Bad version rejected.
    #[test]
    fn bad_version_rejected() {
        let mut bytes = write_fixture_bytes(&[]);
        bytes[8] = 0x99;
        let err = read_fixture_bytes(&bytes).unwrap_err();
        assert!(matches!(err, ReplayError::BadVersion { .. }), "{err:?}");
    }

    /// (4) Frame count mismatch rejected.
    #[test]
    fn frame_count_mismatch_rejected() {
        let frames = vec![request_frame(0, &minimal_request())];
        let mut bytes = write_fixture_bytes(&frames);
        bytes[10..14].copy_from_slice(&5u32.to_le_bytes());
        let err = read_fixture_bytes(&bytes).unwrap_err();
        assert!(matches!(err, ReplayError::FrameCountMismatch { .. }));
    }

    /// (5) Truncated payload rejected.
    #[test]
    fn truncated_payload_rejected() {
        let frames = vec![request_frame(0, &minimal_request())];
        let mut bytes = write_fixture_bytes(&frames);
        bytes.truncate(bytes.len() - 3);
        let err = read_fixture_bytes(&bytes).unwrap_err();
        assert!(matches!(
            err,
            ReplayError::Io(_) | ReplayError::FrameCountMismatch { .. }
        ));
    }

    /// (6) Happy-path replay: 1 reserve + 1 commit.
    #[test]
    fn replay_minimal_happy_path() {
        let frames = vec![
            request_frame(0, &minimal_request()),
            response_delta_frame(
                1,
                &minimal_delta(Some("ok"), Some("stop"), Some((5, 7))),
                0x00,
            ),
            trailers_frame(2, Bytes::from_static(b"grpc-status:0")),
        ];
        let bytes = write_fixture_bytes(&frames);
        let report = replay_fixture_bytes(&bytes).unwrap();
        assert_eq!(report.request_frames_decoded, 1);
        assert_eq!(report.response_frames_decoded, 1);
        assert_eq!(report.finish_reason.as_deref(), Some("stop"));
        assert_eq!(report.cumulative_output_tokens, Some(7));
        assert_eq!(report.sidecar_reserve_calls, 1);
        assert_eq!(report.sidecar_commit_calls, 1);
        assert!(!report.upstream_error);
        assert!(!report.unsupported_wire_version_seen);
        assert!(!report.decoder_skipped);
        assert!(report.translated_request.is_some());
    }

    /// (7) Upstream error: no commit.
    #[test]
    fn replay_upstream_error_short_circuits_commit() {
        let frames = vec![
            request_frame(0, &minimal_request()),
            trailers_frame(
                1,
                Bytes::from_static(b"grpc-status:13\rgrpc-message:upstream busted"),
            ),
        ];
        let bytes = write_fixture_bytes(&frames);
        let report = replay_fixture_bytes(&bytes).unwrap();
        assert_eq!(report.request_frames_decoded, 1);
        assert!(report.upstream_error);
        assert_eq!(report.sidecar_reserve_calls, 1);
        assert_eq!(report.sidecar_commit_calls, 0);
    }

    /// (8) Unknown wire version: flagged.
    #[test]
    fn replay_unknown_wire_version_flagged() {
        let mut req = minimal_request();
        req.cascade_wire_version = Some("cascade.v9.9".to_string());
        let frames = vec![
            request_frame(0, &req),
            trailers_frame(1, Bytes::from_static(b"grpc-status:0")),
        ];
        let bytes = write_fixture_bytes(&frames);
        let report = replay_fixture_bytes(&bytes).unwrap();
        assert!(report.unsupported_wire_version_seen);
        assert_eq!(report.request_frames_decoded, 0);
        // No reserve (request was unsupported wire version).
        assert_eq!(report.sidecar_reserve_calls, 0);
    }

    /// (9) Truncated body: decoder_skipped set.
    #[test]
    fn replay_truncated_body_marks_decoder_skipped() {
        // Use a deliberately malformed payload that won't decode.
        let frames = vec![
            raw_frame(
                0,
                Direction::Client,
                0x00,
                Bytes::from_static(b"\xff\xff\xff garbage"),
            ),
            trailers_frame(1, Bytes::from_static(b"grpc-status:0")),
        ];
        let bytes = write_fixture_bytes(&frames);
        let report = replay_fixture_bytes(&bytes).unwrap();
        assert!(report.decoder_skipped);
        assert_eq!(report.request_frames_decoded, 0);
    }

    /// (10) find_subslice helper sanity.
    #[test]
    fn find_subslice_helper() {
        assert_eq!(find_subslice(b"hello world", b"world"), Some(6));
        assert_eq!(find_subslice(b"hello world", b"xyz"), None);
        assert_eq!(find_subslice(b"", b"x"), None);
        assert_eq!(find_subslice(b"abc", b""), None);
    }
}
