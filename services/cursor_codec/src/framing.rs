//! Connect-RPC framing reader.
//!
//! Cursor IDE Agent speaks Connect-RPC (connect.build) to `api.cursor.sh`.
//! The framing layer is public (<https://connectrpc.com/docs/protocol>); only
//! the envelope inside each frame is proprietary (see [`crate::envelope`]).
//!
//! A single Connect-RPC frame on the wire is:
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
//! * `0x02` — end-of-stream / trailers frame on server-streaming.
//! * any other bit set is malformed and rejected.
//!
//! [`ConnectRpcReader`] consumes a `Read`-implementing source and yields
//! [`Frame`] values one at a time via [`ConnectRpcReader::read_frame`]. The
//! reader is intentionally synchronous: SLICE 2 ships a `std::io::Read`
//! adapter so the framing parser can be exercised by unit tests against
//! `Cursor<Vec<u8>>` and by SLICE 8 fixture replay against `BufReader<File>`.
//! SLICE 5+ will graft an async layer over this for the live egress proxy
//! path, but the framing semantics are the same.
//!
//! Per [`review-standards.md`](../../../docs/specs/coverage/D17_cursor_mitm/review-standards.md)
//! §4 (`W1`-`W3`):
//!
//! * `W1`: framing is parsed as 5-byte prefix + payload.
//! * `W2`: compression flag is acted on (we expose it; envelope decode
//!   gates on it).
//! * `W3`: end-of-stream flag is detected; callers can distinguish the
//!   terminal frame from data frames via [`Frame::is_end_of_stream`].

use std::io::{self, ErrorKind, Read};

use bytes::Bytes;

/// Default frame ceiling — 8 MiB. Connect-RPC's spec uses gRPC's 4 MiB
/// default; we leave headroom for streamed multi-file context blobs.
/// Callers can override via [`ConnectRpcReader::with_max_frame_len`].
pub const DEFAULT_MAX_FRAME_LEN: u32 = 8 * 1024 * 1024;

/// The set of valid Connect-RPC flag bits. Any other bit set is rejected.
const VALID_FLAG_MASK: u8 = FLAG_COMPRESSED | FLAG_END_OF_STREAM;

/// Compressed-payload flag bit.
pub const FLAG_COMPRESSED: u8 = 0x01;
/// End-of-stream / trailers flag bit.
pub const FLAG_END_OF_STREAM: u8 = 0x02;

/// A decoded Connect-RPC frame.
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
    /// Content-Encoding negotiated in the Connect-RPC headers.
    pub fn is_compressed(&self) -> bool {
        self.flags & FLAG_COMPRESSED != 0
    }

    /// `true` when this is the terminal end-of-stream trailers frame on
    /// a server-streaming response.
    pub fn is_end_of_stream(&self) -> bool {
        self.flags & FLAG_END_OF_STREAM != 0
    }
}

/// Reader for a stream of Connect-RPC framed payloads.
///
/// Generic over any `Read` source: `Cursor<Vec<u8>>` for tests,
/// `BufReader<File>` for fixture replay, future `impl Read` adapters for
/// the egress-proxy hot path.
pub struct ConnectRpcReader<R: Read> {
    inner: R,
    max_frame_len: u32,
}

impl<R: Read> ConnectRpcReader<R> {
    /// Construct a reader with the default frame ceiling
    /// ([`DEFAULT_MAX_FRAME_LEN`]).
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            max_frame_len: DEFAULT_MAX_FRAME_LEN,
        }
    }

    /// Override the maximum allowed frame payload length.
    ///
    /// Callers pass a tighter cap when they know the deployment's request
    /// shape (e.g. SOW customer with a 1 MiB hard ceiling) or a looser cap
    /// when they specifically need to accept larger Cursor context blobs.
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
    ///   was read. Indicates a clean end-of-stream after a previous
    ///   complete frame.
    /// * `Err(io::Error)` — the stream was truncated mid-frame, the
    ///   length prefix exceeded `max_frame_len`, or the flag byte
    ///   carried a reserved bit.
    ///
    /// Truncation mid-frame returns `io::Error` with [`ErrorKind::UnexpectedEof`]
    /// so callers can distinguish a clean EOF (`Ok(None)`) from a partial
    /// frame (which is a protocol violation).
    pub fn read_frame(&mut self) -> io::Result<Option<Frame>> {
        // Read the flag byte. Use a 1-byte buffer and check for clean
        // EOF: if zero bytes come back, the previous frame was the last
        // one and we report `Ok(None)`.
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
                format!("connect-rpc: invalid flag byte {flags:#04x}: reserved bits set"),
            ));
        }

        // Read the 4-byte big-endian length prefix. After we have read
        // the flag byte the rest of the frame MUST follow, so any short
        // read here is `UnexpectedEof`.
        let mut len_buf = [0u8; 4];
        self.inner.read_exact(&mut len_buf).map_err(|e| {
            if e.kind() == ErrorKind::UnexpectedEof {
                io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "connect-rpc: truncated frame: missing length prefix",
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
                    "connect-rpc: oversized frame: length {length} exceeds max {}",
                    self.max_frame_len
                ),
            ));
        }

        // Read the payload. Zero-length payloads are valid (Connect-RPC
        // empty messages or empty trailers blobs).
        let mut payload = vec![0u8; length as usize];
        if length > 0 {
            self.inner.read_exact(&mut payload).map_err(|e| {
                if e.kind() == ErrorKind::UnexpectedEof {
                    io::Error::new(
                        ErrorKind::UnexpectedEof,
                        format!(
                            "connect-rpc: truncated frame: payload short of declared length {length}"
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

    /// Helper: build a single on-wire frame [flags][length BE][payload].
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
        let mut reader = ConnectRpcReader::new(Cursor::new(Vec::<u8>::new()));
        assert!(reader.read_frame().unwrap().is_none());
    }

    /// (2) Single data frame round-trip.
    #[test]
    fn single_frame_round_trip() {
        let payload = b"hello world";
        let bytes = frame_bytes(0x00, payload);
        let mut reader = ConnectRpcReader::new(Cursor::new(bytes));

        let frame = reader.read_frame().unwrap().unwrap();
        assert_eq!(frame.flags, 0x00);
        assert_eq!(frame.payload.as_ref(), payload);
        assert!(!frame.is_compressed());
        assert!(!frame.is_end_of_stream());

        // After the single frame is consumed, the next read yields
        // clean EOF — `Ok(None)`.
        assert!(reader.read_frame().unwrap().is_none());
    }

    /// (3) Multi-frame stream is read in order.
    #[test]
    fn multi_frame_stream_reads_in_order() {
        let mut wire = Vec::new();
        wire.extend(frame_bytes(0x00, b"first"));
        wire.extend(frame_bytes(0x00, b"second"));
        wire.extend(frame_bytes(0x00, b"third"));

        let mut reader = ConnectRpcReader::new(Cursor::new(wire));
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
        // Trailers frame — Connect-RPC EOS marker. Payload is the
        // length-delimited trailers blob; for the framing layer test we
        // just need a non-empty body so we can confirm we read it.
        wire.extend(frame_bytes(FLAG_END_OF_STREAM, b"grpc-status:0"));

        let mut reader = ConnectRpcReader::new(Cursor::new(wire));
        let data = reader.read_frame().unwrap().unwrap();
        assert!(!data.is_end_of_stream());

        let trailers = reader.read_frame().unwrap().unwrap();
        assert!(trailers.is_end_of_stream());
        assert_eq!(trailers.payload.as_ref(), b"grpc-status:0");
    }

    /// (5) Truncated frame mid-payload → `UnexpectedEof`.
    #[test]
    fn truncated_frame_mid_payload_errors() {
        // Declare a 10-byte payload but only provide 3 bytes.
        let mut wire = Vec::new();
        wire.push(0x00); // flag
        wire.extend_from_slice(&10u32.to_be_bytes()); // length
        wire.extend_from_slice(b"abc"); // truncated payload

        let mut reader = ConnectRpcReader::new(Cursor::new(wire));
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
        // Only the flag byte arrives; length prefix is missing.
        let wire = vec![0x00];
        let mut reader = ConnectRpcReader::new(Cursor::new(wire));
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
        // Cap the reader at 16 bytes and feed a 64-byte declaration.
        let mut wire = Vec::new();
        wire.push(0x00);
        wire.extend_from_slice(&64u32.to_be_bytes());
        // Payload would follow on the wire; we never get that far.

        let mut reader = ConnectRpcReader::new(Cursor::new(wire)).with_max_frame_len(16);
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
        // Connect-RPC empty-message frames are 5 bytes total:
        // [flag=0x00][length=0u32 BE].
        let wire = frame_bytes(0x00, &[]);
        assert_eq!(wire.len(), 5);

        let mut reader = ConnectRpcReader::new(Cursor::new(wire));
        let frame = reader.read_frame().unwrap().unwrap();
        assert_eq!(frame.flags, 0x00);
        assert!(frame.payload.is_empty());
        assert!(reader.read_frame().unwrap().is_none());
    }

    /// (8) Malformed flag byte (reserved bit set) → `InvalidData`.
    #[test]
    fn malformed_flag_byte_errors() {
        // 0x80 is reserved per the framing spec — must reject.
        let wire = frame_bytes(0x80, b"payload");
        let mut reader = ConnectRpcReader::new(Cursor::new(wire));
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
        let mut reader = ConnectRpcReader::new(Cursor::new(wire));
        let frame = reader.read_frame().unwrap().unwrap();
        assert!(frame.is_compressed());
        assert!(!frame.is_end_of_stream());
        assert_eq!(frame.payload.as_ref(), payload);
    }

    /// (10) Default frame cap is `DEFAULT_MAX_FRAME_LEN`.
    #[test]
    fn default_frame_cap_is_eight_mib() {
        // Sanity check the public constant. SLICE 5+ may want a tighter
        // cap; a regression here would let oversize frames through.
        assert_eq!(DEFAULT_MAX_FRAME_LEN, 8 * 1024 * 1024);
    }
}
