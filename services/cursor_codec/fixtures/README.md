# Cursor MITM Fixture Format

> **EXPERIMENTAL.** Fixtures under this directory drive the offline parser /
> decoder tests for `services/cursor_codec`. Live Cursor traffic is NOT used
> in CI per [`review-standards.md`](../../../docs/specs/coverage/D17_cursor_mitm/review-standards.md)
> §6 (`C1`). SLICE 1 ships **synthetic** fixtures only; SLICE 8 will land real
> captures into a sibling `recorded/` directory with the same on-disk layout.

## 1. File extension and on-disk layout

Each fixture file has the extension `.cursor-rpc` and the following on-disk
layout (little-endian unless otherwise noted):

```
+----------------------+-------------------+--------------------+---------------------+
| magic = b"SGCRPC\0\0" | version (u16 LE) | frame count (u32 LE) |  reserved (u16 LE)  |
|       8 bytes         |     2 bytes      |       4 bytes        |       2 bytes       |
+----------------------+-------------------+--------------------+---------------------+

Per-frame record (repeated frame_count times):

+--------------------+----------------------+----------------------+---------------------+---------------------+
| timestamp_ms (u64) | direction (u8)       | rpc_flag (u8)        | length (u32 BE)     | payload (length B)  |
|      8 bytes       |  0=client  1=server  |  same as Connect bit |   Connect-RPC bytes |                     |
+--------------------+----------------------+----------------------+---------------------+---------------------+
```

Key properties of this layout:

* The `length` and `rpc_flag` bytes are stored **exactly as Connect-RPC writes
  them on the wire** — big-endian length, `0x00`/`0x02`/etc. flag byte. The
  parser implementation in `src/framing.rs` can therefore consume the bytes
  `[rpc_flag][length BE][payload]` from each record without any per-record
  byte-order conversion.
* The outer envelope (`magic` / `version` / `frame count` / `timestamp_ms`) is
  stored little-endian so the dev harness can produce + read it on x86/aarch64
  hosts without endian-flipping.
* `version` starts at `1` and is bumped whenever the envelope layout changes.
  The reader rejects any version it does not recognise — there is no implicit
  forward compatibility.
* `direction` distinguishes a client-to-server frame (the unary `ChatRequest`)
  from a server-to-client frame (a `ChatResponseChunk` or the EOS trailers
  frame).
* `timestamp_ms` is a UNIX epoch milliseconds value at capture time. For
  synthetic fixtures it is the wall clock at fixture generation; for recorded
  fixtures it is the wall clock at MITM capture.

## 2. Synthetic vs recorded

`synthetic/` holds fixtures crafted from the **public Connect-RPC framing
spec** plus a minimal placeholder protobuf envelope. They are deterministic,
small (always <64 KiB per `review-standards.md` §6 `C4`), and explicitly
labelled in their filenames as `synthetic_*`. Their purpose is to exercise
the SLICE 2 framing parser and the SLICE 3 envelope decode path.

`recorded/` is **reserved for SLICE 8**. It will hold real captures of
`api.cursor.sh` traffic intercepted by the SpendGuard egress proxy under a
SOW customer's deployment. Recorded fixtures will carry a sidecar
`<name>.cursor-rpc.manifest.json` documenting Cursor client version, capture
date, and field-by-field hex evidence per `review-standards.md` §2 `R2`.

SLICE 1 does NOT create the `recorded/` directory; it is reserved by name
only. The decision to defer is explicit: capturing real traffic requires the
egress-proxy SAN extension + D02 leaf cert wiring that SLICE 8 lands.

## 3. Versioning + version tags

Every fixture filename SHOULD include a Cursor client version range hint per
`review-standards.md` §2 `R4`. Synthetic fixtures use the literal token
`synthetic` in place of the version range, since they are not tied to a real
Cursor client version. Examples:

```
synthetic_unary_v1.cursor-rpc
synthetic_streaming_chunked_v1.cursor-rpc
```

Recorded fixtures (SLICE 8) will look like:

```
recorded_unary_cursor_0.42.x.cursor-rpc
recorded_streaming_cursor_0.42.x.cursor-rpc
```

## 4. Reader contract

`ConnectRpcReader::read_frame` consumes one `[rpc_flag][length BE][payload]`
tuple per call. The outer envelope (`magic` / `version` / `frame count`) is
parsed by a higher-level `FixtureReader` (SLICE 8) which then hands each
inner tuple to `ConnectRpcReader`. SLICE 2 ships `ConnectRpcReader` only;
the envelope reader is wired in SLICE 8.

For the SLICE 2 unit tests, fixtures are read with a small in-test helper that
opens the file, validates the envelope, and yields each `[rpc_flag][length
BE][payload]` block as an in-memory byte slice. The same byte slice is what a
real Connect-RPC reader would see on the wire.

## 5. Size budget

Per `review-standards.md` §6 (`C4`), no individual fixture file may exceed
64 KiB. The synthetic fixtures in this slice are <2 KiB each because the
placeholder envelope is small and there are at most a handful of frames per
fixture. SLICE 8 will likely push closer to 64 KiB but never over.

## 6. Generating new fixtures

A small Rust helper in `tests/fixture_helpers.rs` (SLICE 2 will create it)
exposes `write_fixture(path, frames)` which produces the on-disk layout
described in §1. Synthetic fixtures committed to this directory are
generated either by that helper or by a `xxd -r` paste in a code review
where the reviewer can read the hex manually. Either way they are
**hand-auditable** — a reviewer can `xxd` a fixture and verify the header,
the frame count, and at least one frame's length prefix matches the
documented layout.
