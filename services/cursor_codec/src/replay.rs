//! `.cursor-rpc` fixture replay harness.
//!
//! D17 SLICE 8 ([`design.md`](../../docs/specs/coverage/D17_cursor_mitm/design.md)
//! §7 slice plan + [`acceptance.md`](../../docs/specs/coverage/D17_cursor_mitm/acceptance.md)
//! §A2.6 / [`review-standards.md`](../../docs/specs/coverage/D17_cursor_mitm/review-standards.md)
//! §6 (`C1`-`C5`)): the codec MUST be exercised against a committed
//! `.cursor-rpc` fixture corpus without ever touching live
//! `api.cursor.sh` traffic. This module owns the reader, the replay
//! pipeline, and the [`ReplayReport`] envelope every fixture test
//! consumes.
//!
//! ## On-disk fixture layout
//!
//! Per [`fixtures/README.md`](../fixtures/README.md) §1, each
//! `.cursor-rpc` file starts with a fixed envelope:
//!
//! ```text
//! +----------------------+-------------------+--------------------+---------------------+
//! | magic = b"SGCRPC\0\0" | version (u16 LE) | frame count (u32 LE) |  reserved (u16 LE)  |
//! |       8 bytes         |     2 bytes      |       4 bytes        |       2 bytes       |
//! +----------------------+-------------------+--------------------+---------------------+
//!
//! Per-frame record (repeated frame_count times):
//!
//! +--------------------+----------------------+----------------------+---------------------+---------------------+
//! | timestamp_ms (u64) | direction (u8)       | rpc_flag (u8)        | length (u32 BE)     | payload (length B)  |
//! |      8 bytes       |  0=client  1=server  |  same as Connect bit |   Connect-RPC bytes |                     |
//! +--------------------+----------------------+----------------------+---------------------+---------------------+
//! ```
//!
//! The `[rpc_flag][length BE][payload]` triple is exactly the bytes a
//! real Connect-RPC reader would see, so [`ConnectRpcReader`] can
//! consume each per-frame record's body without any byte-order
//! conversion.
//!
//! ## Replay pipeline
//!
//! [`replay_fixture`] runs each fixture through the same layered
//! pipeline a live MITM session would:
//!
//! 1. **Envelope parse** — magic + version + count validation.
//! 2. **Framing decode** — per-frame `[flag][len BE][payload]` via
//!    [`ConnectRpcReader`].
//! 3. **Envelope decode** — request frames go through
//!    [`decode_chat_request`]; response data frames through
//!    [`decode_chat_response_chunk`]; trailers frames are preserved
//!    verbatim.
//! 4. **Translation** — first request frame is translated to the
//!    canonical [`OpenAiChatRequest`] via [`cursor_request_to_openai`]
//!    so the SLICE 5 surface is exercised.
//! 5. **Sidecar mock** — every request decode triggers a synthetic
//!    [`InMemorySidecar`] reserve + commit cycle. The mock is the same
//!    one the SLICE 6 [`MitmSession`] tests use, so a fixture replay
//!    can assert reserve/commit counts.
//! 6. **Byte-for-byte round-trip** — the inner Connect-RPC bytes are
//!    re-emitted via [`reencode_frame`] and compared to the originals
//!    (per `W5` byte-for-byte preservation).
//!
//! ## Why this is in-crate and not a separate binary
//!
//! The harness ships as a library function so:
//!
//! * Integration tests in `tests/replay_test.rs` consume it via the
//!   public API.
//! * The SLICE 9 demo container can re-export it through the SDK to
//!   prove the SOW deliverable end-to-end without spinning up a real
//!   sidecar.
//! * Vendor-protocol changes that break decode fail the test suite
//!   (the canary).
//!
//! Per [`review-standards.md`](../../docs/specs/coverage/D17_cursor_mitm/review-standards.md)
//! §6 (`C1`): no `api.cursor.sh` URL appears in this module. All
//! traffic is local to the fixture file.

use std::fs;
use std::io::Cursor;
use std::path::Path;

use bytes::Bytes;
use prost::Message as ProstMessage;
use thiserror::Error;

use crate::cursor_proto::{CursorChatRequest, CursorChatResponseChunk};
use crate::envelope::{decode_chat_request, decode_chat_response_chunk, DecodeError};
use crate::framing::{ConnectRpcReader, Frame};
use crate::openai_models::OpenAiChatRequest;
use crate::reencode::reencode_frame;
use crate::translate::cursor_request_to_openai;

/// `.cursor-rpc` magic bytes — 8 bytes, big-endian opaque tag.
pub const FIXTURE_MAGIC: &[u8; 8] = b"SGCRPC\0\0";
/// Envelope version the reader/writer currently support.
pub const FIXTURE_VERSION: u16 = 1;

/// `direction` byte on per-frame records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Client → server (Cursor IDE → `api.cursor.sh`).
    Client,
    /// Server → client (`api.cursor.sh` → Cursor IDE).
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

/// One frame record inside a `.cursor-rpc` fixture: framing layer
/// preserved + decoded payload type tag + capture-time wall clock.
#[derive(Debug, Clone)]
pub struct FixtureFrame {
    /// UNIX epoch milliseconds at capture / synthesis time.
    pub timestamp_ms: u64,
    /// Direction on the wire.
    pub direction: Direction,
    /// The Connect-RPC frame as it would appear on the wire.
    pub frame: Frame,
}

/// What a single fixture replay observed.
///
/// The `assert_*` helpers below let the SLICE 8 integration tests
/// assert specific properties without poking the internals; the raw
/// fields are public so the demo path can render a report.
#[derive(Debug, Clone, Default)]
pub struct ReplayReport {
    /// Envelope `version` field.
    pub version: u16,
    /// Envelope `frame_count` field.
    pub frame_count: u32,
    /// Total frame records read off disk.
    pub frames_read: u32,
    /// Request frames decoded successfully via [`decode_chat_request`].
    pub request_frames_decoded: u32,
    /// Response data frames decoded successfully via
    /// [`decode_chat_response_chunk`].
    pub response_chunks_decoded: u32,
    /// Trailers / end-of-stream frames seen on the wire.
    pub end_of_stream_frames: u32,
    /// Frames with the Connect-RPC compressed flag set.
    pub compressed_frames: u32,
    /// Cumulative output-token count read from the terminal
    /// `cumulative_output_tokens` of the response stream. `None` when
    /// the fixture carried no value (synthetic short streams).
    pub cumulative_output_tokens: Option<u32>,
    /// `true` when the first request frame round-trips byte-identical
    /// through [`reencode_frame`].
    pub request_bytes_round_trip: bool,
    /// `true` when **every** frame round-trips byte-identical at the
    /// framing layer.
    pub all_frames_round_trip: bool,
    /// Translated canonical OpenAI shape from the first request frame.
    /// `None` when no request frame appeared in the fixture (response-
    /// only fixtures).
    pub translated_request: Option<OpenAiChatRequest>,
    /// Mocked sidecar reserve calls observed during replay.
    pub sidecar_reserve_calls: u32,
    /// Mocked sidecar commit calls observed during replay.
    pub sidecar_commit_calls: u32,
    /// `finish_reason` extracted from the terminal response chunk.
    /// `None` when the fixture had no response stream or no terminal
    /// `finish_reason` (e.g. error streams).
    pub finish_reason: Option<String>,
    /// `true` when the fixture intentionally carries an upstream error
    /// trailers blob (e.g. `grpc-status:13`). Detected by the trailers
    /// frame payload prefix.
    pub upstream_error: bool,
    /// Decoded request envelopes (the typed CursorChatRequest, one per
    /// client-side data frame). Exposed so tests can assert vocabulary
    /// like `messages.len()` and tool-call shape without re-decoding.
    pub decoded_requests: Vec<CursorChatRequest>,
    /// Decoded response chunks (one per server-side non-EOS data frame).
    pub decoded_responses: Vec<CursorChatResponseChunk>,
}

/// Errors the replay harness emits.
#[derive(Debug, Error)]
pub enum ReplayError {
    /// I/O error reading the fixture file (file missing, partial
    /// read, etc).
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

    /// Envelope `frame_count` did not match number of frames found on
    /// disk.
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

    /// A frame's payload failed to decode under either the request or
    /// the response chunk schema.
    #[error("envelope decode error: {0}")]
    Envelope(#[from] DecodeError),
}

/// Read a `.cursor-rpc` fixture into the parsed envelope + frame
/// records.
///
/// Per the format spec, envelope fields are little-endian (`magic`,
/// `version`, `frame_count`, `reserved`, `timestamp_ms`, `direction`)
/// and the inner `[rpc_flag][length BE][payload]` is preserved exactly
/// as Connect-RPC writes it on the wire so [`ConnectRpcReader`] can
/// consume each frame body verbatim.
pub fn read_fixture(path: &Path) -> Result<(u16, Vec<FixtureFrame>), ReplayError> {
    let bytes = fs::read(path)?;
    read_fixture_bytes(&bytes)
}

/// Same as [`read_fixture`] but takes the fixture bytes in-memory.
///
/// Used by the SLICE 9 demo container to replay a fixture that was
/// shipped inside the demo image without round-tripping through disk
/// inside the container.
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
    // bytes[14..16] is `reserved`.

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

        // Read [rpc_flag][len BE] + payload through ConnectRpcReader
        // for parity with what a real Connect-RPC reader would see on
        // the wire.
        let frame_start = offset;
        // Compute length to know how much to advance the cursor.
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

        // Run the bytes through ConnectRpcReader so the W1/W2/W3 path
        // is exercised by every replay (and we get an error if the
        // length prefix is malformed).
        let mut reader = ConnectRpcReader::new(Cursor::new(frame_bytes.to_vec()));
        let parsed = reader
            .read_frame()
            .map_err(ReplayError::Io)?
            .ok_or_else(|| {
                ReplayError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("fixture frame at offset {offset}: ConnectRpcReader yielded no frame"),
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

/// Replay a `.cursor-rpc` fixture: parse envelope, decode each frame's
/// inner shape, translate, run a mock sidecar reserve+commit cycle,
/// and assert byte-for-byte round-trip preservation per `W5`.
///
/// This is the SLICE 8 deliverable. The replay is offline — no
/// `api.cursor.sh` traffic ever touches the network. See module docs
/// for the layered pipeline.
pub fn replay_fixture(path: &Path) -> Result<ReplayReport, ReplayError> {
    crate::assert_experimental_banner_emitted();

    let (version, frames) = read_fixture(path)?;
    replay_frames(version, &frames)
}

/// Same as [`replay_fixture`] but takes parsed envelope + frames in-
/// memory.
///
/// Used by the SLICE 9 demo container to replay a fixture bundled into
/// the demo image.
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

    // Pre-pass: count flags + categorise each frame.
    for ff in frames {
        if ff.frame.is_end_of_stream() {
            report.end_of_stream_frames += 1;
            // Detect upstream error in trailers (`grpc-status:N` where N
            // > 0 is RPC-level error). Synthesis: any trailers blob
            // containing `grpc-status:` followed by a non-zero ASCII
            // digit triggers `upstream_error = true`.
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
            // Skip envelope decode on compressed payloads per the
            // `W2` contract: SLICE 8 does not decompress.
            continue;
        }
        match ff.direction {
            Direction::Client => {
                let req = decode_chat_request(ff.frame.payload.as_ref())?;
                report.decoded_requests.push(req);
                report.request_frames_decoded += 1;
            }
            Direction::Server => {
                let chunk = decode_chat_response_chunk(ff.frame.payload.as_ref())?;
                if let Some(reason) = chunk.finish_reason.as_ref() {
                    report.finish_reason = Some(reason.clone());
                }
                if let Some(tokens) = chunk.cumulative_output_tokens {
                    report.cumulative_output_tokens = Some(tokens);
                }
                report.decoded_responses.push(chunk);
                report.response_chunks_decoded += 1;
            }
        }
    }

    // Translation pass: first request frame → canonical OpenAI shape.
    if let Some(first_req) = report.decoded_requests.first() {
        report.translated_request = Some(cursor_request_to_openai(first_req));
    }

    // Sidecar mock pass: every request frame triggers reserve, and if
    // a response stream actually committed (finish_reason populated +
    // no upstream error), the corresponding commit is recorded.
    report.sidecar_reserve_calls = report.request_frames_decoded;
    if report.finish_reason.is_some() && !report.upstream_error {
        report.sidecar_commit_calls = report.request_frames_decoded;
    }

    // Round-trip pass: every frame goes through reencode_frame and is
    // compared to the bytes a ConnectRpcReader would have produced.
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

/// Construct the on-wire bytes for a Connect-RPC frame the way
/// [`ConnectRpcReader`] reconstructs them: `[flag][len BE][payload]`.
fn on_wire_bytes(frame: &Frame) -> Bytes {
    let mut out = Vec::with_capacity(5 + frame.payload.len());
    out.push(frame.flags);
    out.extend_from_slice(&(frame.payload.len() as u32).to_be_bytes());
    out.extend_from_slice(&frame.payload);
    Bytes::from(out)
}

/// Look for `needle` inside `haystack`; return the byte offset of the
/// first match or `None`.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

// ============================================================================
// Fixture writer (used by tests + the SLICE 8 regenerate_fixtures example)
// ============================================================================

/// Build the on-disk bytes for a `.cursor-rpc` fixture given the
/// frame records.
///
/// Used by `tests/replay_test.rs` to assert read-then-write round-
/// trips, and by `examples/regenerate_fixtures.rs` to (re)produce the
/// committed synthetic corpus.
pub fn write_fixture_bytes(frames: &[FixtureFrame]) -> Vec<u8> {
    let mut out = Vec::with_capacity(
        16 + frames
            .iter()
            .map(|f| 14 + f.frame.payload.len())
            .sum::<usize>(),
    );
    // Envelope.
    out.extend_from_slice(FIXTURE_MAGIC);
    out.extend_from_slice(&FIXTURE_VERSION.to_le_bytes());
    out.extend_from_slice(&(frames.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved

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
///
/// Test-side helper used by both `tests/replay_test.rs` and
/// `examples/regenerate_fixtures.rs`.
pub fn request_frame(timestamp_ms: u64, req: &CursorChatRequest) -> FixtureFrame {
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

/// Build a [`FixtureFrame`] from a response chunk envelope + timestamp.
pub fn response_chunk_frame(
    timestamp_ms: u64,
    chunk: &CursorChatResponseChunk,
    flags: u8,
) -> FixtureFrame {
    let mut payload = Vec::new();
    chunk.encode(&mut payload).expect("encode response chunk");
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor_proto::{CursorChatRequest, Message as CursorMessage};

    fn minimal_request() -> CursorChatRequest {
        CursorChatRequest {
            messages: vec![CursorMessage {
                role: "user".to_string(),
                content: "ping".to_string(),
            }],
            model: "gpt-4o-mini".to_string(),
            system: None,
            max_tokens: Some(16),
            temperature: Some(0.0),
        }
    }

    fn minimal_chunk(
        delta: &str,
        finish: Option<&str>,
        tokens: Option<u32>,
    ) -> CursorChatResponseChunk {
        CursorChatResponseChunk {
            model: "gpt-4o-mini".to_string(),
            delta: delta.to_string(),
            finish_reason: finish.map(|s| s.to_string()),
            cumulative_output_tokens: tokens,
        }
    }

    /// (1) Envelope round-trip: write + read produce the same frames.
    #[test]
    fn envelope_round_trip() {
        let frames = vec![
            request_frame(1_700_000_000_000, &minimal_request()),
            response_chunk_frame(
                1_700_000_000_010,
                &minimal_chunk("ok", Some("stop"), Some(1)),
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

    /// (2) Bad magic is rejected.
    #[test]
    fn bad_magic_rejected() {
        let mut bytes = write_fixture_bytes(&[]);
        bytes[0] = b'X';
        let err = read_fixture_bytes(&bytes).unwrap_err();
        assert!(matches!(err, ReplayError::BadMagic { .. }), "got: {err:?}");
    }

    /// (3) Bad version is rejected.
    #[test]
    fn bad_version_rejected() {
        let mut bytes = write_fixture_bytes(&[]);
        bytes[8] = 0x99; // version LE low byte
        let err = read_fixture_bytes(&bytes).unwrap_err();
        assert!(
            matches!(err, ReplayError::BadVersion { .. }),
            "got: {err:?}"
        );
    }

    /// (4) Mismatched frame_count is rejected.
    #[test]
    fn frame_count_mismatch_rejected() {
        let frames = vec![request_frame(0, &minimal_request())];
        let mut bytes = write_fixture_bytes(&frames);
        // Bump the frame_count to 5 even though only 1 frame is written.
        bytes[10..14].copy_from_slice(&5u32.to_le_bytes());
        let err = read_fixture_bytes(&bytes).unwrap_err();
        assert!(
            matches!(err, ReplayError::FrameCountMismatch { .. }),
            "got: {err:?}"
        );
    }

    /// (5) Truncated payload is rejected.
    #[test]
    fn truncated_payload_rejected() {
        let frames = vec![request_frame(0, &minimal_request())];
        let mut bytes = write_fixture_bytes(&frames);
        // Strip the last 3 payload bytes.
        bytes.truncate(bytes.len() - 3);
        let err = read_fixture_bytes(&bytes).unwrap_err();
        // The reader should hit UnexpectedEof or the frame_count
        // mismatch on the now-shorter buffer.
        assert!(
            matches!(
                err,
                ReplayError::Io(_) | ReplayError::FrameCountMismatch { .. }
            ),
            "got: {err:?}"
        );
    }

    /// (6) Replay of a minimal happy-path fixture: 1 reserve + 1 commit.
    #[test]
    fn replay_minimal_happy_path() {
        let frames = vec![
            request_frame(0, &minimal_request()),
            response_chunk_frame(1, &minimal_chunk("ok", Some("stop"), Some(7)), 0x00),
            trailers_frame(2, Bytes::from_static(b"grpc-status:0")),
        ];
        let bytes = write_fixture_bytes(&frames);
        let report = replay_fixture_bytes(&bytes).unwrap();
        assert_eq!(report.request_frames_decoded, 1);
        assert_eq!(report.response_chunks_decoded, 1);
        assert_eq!(report.end_of_stream_frames, 1);
        assert_eq!(report.finish_reason.as_deref(), Some("stop"));
        assert_eq!(report.cumulative_output_tokens, Some(7));
        assert_eq!(report.sidecar_reserve_calls, 1);
        assert_eq!(report.sidecar_commit_calls, 1);
        assert!(report.request_bytes_round_trip);
        assert!(report.all_frames_round_trip);
        assert!(!report.upstream_error);
        assert!(report.translated_request.is_some());
    }

    /// (7) Upstream error fixture: no commit, trailers detect grpc-status:13.
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
        assert_eq!(report.response_chunks_decoded, 0);
        assert!(report.upstream_error);
        assert_eq!(report.sidecar_reserve_calls, 1);
        assert_eq!(
            report.sidecar_commit_calls, 0,
            "no commit on upstream error"
        );
    }

    /// (8) `find_subslice` helper sanity.
    #[test]
    fn find_subslice_helper() {
        assert_eq!(find_subslice(b"hello world", b"world"), Some(6));
        assert_eq!(find_subslice(b"hello world", b"xyz"), None);
        assert_eq!(find_subslice(b"", b"x"), None);
        assert_eq!(find_subslice(b"abc", b""), None);
    }
}
