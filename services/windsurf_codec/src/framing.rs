//! gRPC-Web framing reader for Windsurf Cascade.
//!
//! Windsurf IDE Cascade speaks protobuf-over-gRPC-Web to
//! `server.codeium.com` / `windsurf-server.codeium.com`. The framing
//! layer is public (<https://github.com/grpc/grpc-web>); only the
//! Cascade envelope inside each frame is proprietary (see
//! [`crate::windsurf_proto`]).
//!
//! A single gRPC-Web frame on the wire is:
//!
//! ```text
//! +--------+-----------------+---------------------------+
//! | flags  | length (u32 BE) |   payload (length bytes)  |
//! | 1 byte |    4 bytes      |                           |
//! +--------+-----------------+---------------------------+
//! ```
//!
//! `flags` is a bitfield:
//!
//! * `0x01` — compressed payload (algorithm advertised in headers).
//! * `0x02` — end-of-stream / trailers frame.
//! * `0x80` — gRPC-Web trailers frame (alternative encoding).
//! * any other bit set is malformed and rejected.
//!
//! [`GrpcWebReader`] consumes a `Read`-implementing source and yields
//! [`Frame`] values one at a time via [`GrpcWebReader::read_frame`].
//! The reader is intentionally synchronous: SLICE 76 ships a
//! `std::io::Read` adapter so the framing parser can be exercised by
//! unit tests against `Cursor<Vec<u8>>` and by SLICE 80 fixture replay
//! against `BufReader<File>`. SLICE 78+ grafts an async layer on for
//! the live egress proxy path; the framing semantics are the same.

use std::io::{self, ErrorKind, Read};

use bytes::Bytes;

/// Default frame ceiling — 8 MiB. gRPC's default is 4 MiB; we leave
/// headroom for streamed multi-file context blobs. Callers can
/// override via [`GrpcWebReader::with_max_frame_len`].
pub const DEFAULT_MAX_FRAME_LEN: u32 = 8 * 1024 * 1024;

/// Compressed-payload flag bit.
pub const FLAG_COMPRESSED: u8 = 0x01;
/// End-of-stream / trailers flag bit.
pub const FLAG_END_OF_STREAM: u8 = 0x02;
/// gRPC-Web trailers marker bit. Some gRPC-Web implementations set
/// `0x80` for trailers; we accept either `0x02` or `0x80` as
/// end-of-stream so cross-vendor Cascade captures don't fail the
/// reader.
pub const FLAG_GRPC_WEB_TRAILERS: u8 = 0x80;

/// The set of valid gRPC-Web flag bits. Any other bit set is rejected.
const VALID_FLAG_MASK: u8 = FLAG_COMPRESSED | FLAG_END_OF_STREAM | FLAG_GRPC_WEB_TRAILERS;

/// A decoded gRPC-Web frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// The raw flag byte from the wire.
    pub flags: u8,
    /// The payload bytes (`length` bytes, exactly as on the wire — not
    /// decompressed even when `flags & FLAG_COMPRESSED != 0`).
    pub payload: Bytes,
}

impl Frame {
    /// `true` when the payload is gzip / br / zstd compressed per the
    /// Content-Encoding negotiated in the gRPC-Web headers.
    pub fn is_compressed(&self) -> bool {
        self.flags & FLAG_COMPRESSED != 0
    }

    /// `true` when this is the terminal end-of-stream trailers frame.
    /// Accepts either the standard `0x02` bit or the gRPC-Web `0x80`
    /// trailers marker.
    pub fn is_end_of_stream(&self) -> bool {
        self.flags & (FLAG_END_OF_STREAM | FLAG_GRPC_WEB_TRAILERS) != 0
    }
}

/// Reader for a stream of gRPC-Web framed payloads.
///
/// Generic over any `Read` source: `Cursor<Vec<u8>>` for tests,
/// `BufReader<File>` for fixture replay, future `impl Read` adapters
/// for the egress-proxy hot path.
pub struct GrpcWebReader<R: Read> {
    inner: R,
    max_frame_len: u32,
}

impl<R: Read> GrpcWebReader<R> {
    /// Construct a reader with the default frame ceiling
    /// ([`DEFAULT_MAX_FRAME_LEN`]).
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            max_frame_len: DEFAULT_MAX_FRAME_LEN,
        }
    }

    /// Override the maximum allowed frame payload length.
    pub fn with_max_frame_len(mut self, max_frame_len: u32) -> Self {
        self.max_frame_len = max_frame_len;
        self
    }

    /// Read one frame from the stream.
    ///
    /// Returns:
    ///
    /// * `Ok(Some(frame))` — a frame was decoded.
    /// * `Ok(None)` — the stream is at EOF before any prefix byte
    ///   was read.
    /// * `Err(io::Error)` — the stream was truncated mid-frame, the
    ///   length prefix exceeded `max_frame_len`, or the flag byte
    ///   carried a reserved bit.
    pub fn read_frame(&mut self) -> io::Result<Option<Frame>> {
        let mut flag_buf = [0u8; 1];
        match self.inner.read(&mut flag_buf) {
            Ok(0) => return Ok(None),
            Ok(_) => {}
            Err(e) => return Err(e),
        }

        let flags = flag_buf[0];
        if flags & !VALID_FLAG_MASK != 0 {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                format!("grpc-web: invalid flag byte {flags:#04x}: reserved bits set"),
            ));
        }

        let mut len_buf = [0u8; 4];
        self.inner.read_exact(&mut len_buf).map_err(|e| {
            if e.kind() == ErrorKind::UnexpectedEof {
                io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "grpc-web: truncated frame: missing length prefix",
                )
            } else {
                e
            }
        })?;

        let length = u32::from_be_bytes(len_buf);
        if length > self.max_frame_len {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                format!(
                    "grpc-web: oversized frame: length {length} exceeds max {}",
                    self.max_frame_len
                ),
            ));
        }

        let mut payload = vec![0u8; length as usize];
        if length > 0 {
            self.inner.read_exact(&mut payload).map_err(|e| {
                if e.kind() == ErrorKind::UnexpectedEof {
                    io::Error::new(
                        ErrorKind::UnexpectedEof,
                        format!(
                            "grpc-web: truncated frame: payload short of declared length {length}"
                        ),
                    )
                } else {
                    e
                }
            })?;
        }

        Ok(Some(Frame {
            flags,
            payload: Bytes::from(payload),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn frame_bytes(flags: u8, payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(5 + payload.len());
        buf.push(flags);
        buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        buf.extend_from_slice(payload);
        buf
    }

    /// (1) Empty stream → `Ok(None)`.
    #[test]
    fn empty_stream_yields_none() {
        let mut reader = GrpcWebReader::new(Cursor::new(Vec::<u8>::new()));
        assert!(reader.read_frame().unwrap().is_none());
    }

    /// (2) Single data frame round-trip.
    #[test]
    fn single_frame_round_trip() {
        let payload = b"hello cascade";
        let bytes = frame_bytes(0x00, payload);
        let mut reader = GrpcWebReader::new(Cursor::new(bytes));

        let frame = reader.read_frame().unwrap().unwrap();
        assert_eq!(frame.flags, 0x00);
        assert_eq!(frame.payload.as_ref(), payload);
        assert!(!frame.is_compressed());
        assert!(!frame.is_end_of_stream());

        assert!(reader.read_frame().unwrap().is_none());
    }

    /// (3) Multi-frame stream is read in order.
    #[test]
    fn multi_frame_stream_reads_in_order() {
        let mut wire = Vec::new();
        wire.extend(frame_bytes(0x00, b"first"));
        wire.extend(frame_bytes(0x00, b"second"));
        wire.extend(frame_bytes(0x00, b"third"));

        let mut reader = GrpcWebReader::new(Cursor::new(wire));
        let f1 = reader.read_frame().unwrap().unwrap();
        let f2 = reader.read_frame().unwrap().unwrap();
        let f3 = reader.read_frame().unwrap().unwrap();
        assert_eq!(f1.payload.as_ref(), b"first");
        assert_eq!(f2.payload.as_ref(), b"second");
        assert_eq!(f3.payload.as_ref(), b"third");
        assert!(reader.read_frame().unwrap().is_none());
    }

    /// (4) End-of-stream flag is detected on the trailers frame.
    #[test]
    fn end_of_stream_flag_detected() {
        let mut wire = Vec::new();
        wire.extend(frame_bytes(0x00, b"data"));
        wire.extend(frame_bytes(FLAG_END_OF_STREAM, b"grpc-status:0"));

        let mut reader = GrpcWebReader::new(Cursor::new(wire));
        let data = reader.read_frame().unwrap().unwrap();
        assert!(!data.is_end_of_stream());

        let trailers = reader.read_frame().unwrap().unwrap();
        assert!(trailers.is_end_of_stream());
        assert_eq!(trailers.payload.as_ref(), b"grpc-status:0");
    }

    /// (4b) gRPC-Web `0x80` trailers marker is also recognised.
    #[test]
    fn grpc_web_trailers_flag_recognised_as_eos() {
        let wire = frame_bytes(FLAG_GRPC_WEB_TRAILERS, b"grpc-status:0");
        let mut reader = GrpcWebReader::new(Cursor::new(wire));
        let frame = reader.read_frame().unwrap().unwrap();
        assert!(frame.is_end_of_stream());
        assert_eq!(frame.flags, FLAG_GRPC_WEB_TRAILERS);
    }

    /// (5) Truncated frame mid-payload → `UnexpectedEof`.
    #[test]
    fn truncated_frame_mid_payload_errors() {
        let mut wire = Vec::new();
        wire.push(0x00);
        wire.extend_from_slice(&10u32.to_be_bytes());
        wire.extend_from_slice(b"abc");

        let mut reader = GrpcWebReader::new(Cursor::new(wire));
        let err = reader.read_frame().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
        assert!(
            err.to_string().contains("truncated frame"),
            "error message should mention truncation, got: {err}"
        );
    }

    /// (5b) Truncated frame at length prefix → `UnexpectedEof`.
    #[test]
    fn truncated_frame_at_length_prefix_errors() {
        let wire = vec![0x00];
        let mut reader = GrpcWebReader::new(Cursor::new(wire));
        let err = reader.read_frame().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
        assert!(
            err.to_string().contains("missing length prefix"),
            "error message should mention length prefix, got: {err}"
        );
    }

    /// (6) Oversized frame (length > max_frame_len) → `InvalidData`.
    #[test]
    fn oversized_frame_errors() {
        let mut wire = Vec::new();
        wire.push(0x00);
        wire.extend_from_slice(&64u32.to_be_bytes());

        let mut reader = GrpcWebReader::new(Cursor::new(wire)).with_max_frame_len(16);
        let err = reader.read_frame().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        assert!(
            err.to_string().contains("oversized frame"),
            "error message should mention oversize, got: {err}"
        );
    }

    /// (7) Zero-length payload is a valid frame.
    #[test]
    fn zero_length_payload_is_valid() {
        let wire = frame_bytes(0x00, &[]);
        assert_eq!(wire.len(), 5);

        let mut reader = GrpcWebReader::new(Cursor::new(wire));
        let frame = reader.read_frame().unwrap().unwrap();
        assert_eq!(frame.flags, 0x00);
        assert!(frame.payload.is_empty());
        assert!(reader.read_frame().unwrap().is_none());
    }

    /// (8) Malformed flag byte (reserved bit set) → `InvalidData`.
    #[test]
    fn malformed_flag_byte_errors() {
        // 0x40 is reserved per the gRPC-Web framing spec — must reject.
        let wire = frame_bytes(0x40, b"payload");
        let mut reader = GrpcWebReader::new(Cursor::new(wire));
        let err = reader.read_frame().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        assert!(
            err.to_string().contains("invalid flag byte"),
            "error message should mention flag byte, got: {err}"
        );
    }

    /// (9) Compressed-flag payload is returned verbatim (not decompressed).
    #[test]
    fn compressed_flag_returns_payload_verbatim() {
        let payload = b"\x1f\x8b\x08\x00fake-gzip-prefix";
        let wire = frame_bytes(FLAG_COMPRESSED, payload);
        let mut reader = GrpcWebReader::new(Cursor::new(wire));
        let frame = reader.read_frame().unwrap().unwrap();
        assert!(frame.is_compressed());
        assert!(!frame.is_end_of_stream());
        assert_eq!(frame.payload.as_ref(), payload);
    }

    /// (10) Default frame cap is `DEFAULT_MAX_FRAME_LEN`.
    #[test]
    fn default_frame_cap_is_eight_mib() {
        assert_eq!(DEFAULT_MAX_FRAME_LEN, 8 * 1024 * 1024);
    }
}
