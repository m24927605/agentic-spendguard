# Windsurf Cascade Wire Protocol — Hex Evidence

> **EXPERIMENTAL.** This document is the field-by-field hex evidence
> backing the protobuf description in
> [`src/proto/windsurf.proto`](src/proto/windsurf.proto). It is
> SpendGuard's own observation of the Windsurf IDE Cascade runtime
> wire format; no vendor source is included. See
> [`RECON.md`](RECON.md) for the framing-layer recon and
> [`README.md`](README.md) for the legal posture.
>
> Per [`design.md`](../../docs/specs/coverage/D18_windsurf_mitm/design.md)
> §3 decision 6: the proto schema is a black-box reconstruction;
> field names are SpendGuard-chosen; field numbers match the wire as
> observed. No vendor `.proto` is included verbatim.

## 1. Document scope

This is the D18 SLICE 80 deliverable. The synthetic fixture corpus
under [`fixtures/synthetic/`](fixtures/synthetic/) is hand-auditable:
a reviewer can `xxd` each fixture and verify the documented wire-
shape claim. The synthetic fixtures are constructed from the public
gRPC-Web framing spec plus SpendGuard's reconstructed envelope
description; they exercise the codec the same way real captures
would, without exposing real Windsurf session traffic.

## 2. Synthetic capture provenance

| Fixture | Frames | Bytes | Wire version | Capture date |
|---------|-------:|------:|--------------|--------------|
| `cascade_chat_simple.windsurf-rpc` | 5 | 362 | `cascade.v2.0` | 2026-06-07 (synthetic) |
| `cascade_chat_with_tools.windsurf-rpc` | 3 | 392 | `cascade.v2.1` | 2026-06-07 (synthetic) |
| `cascade_chat_streaming.windsurf-rpc` | 11 | 645 | `cascade.v2.0` | 2026-06-07 (synthetic) |
| `cascade_chat_error.windsurf-rpc` | 2 | 195 | `cascade.v2.0` | 2026-06-07 (synthetic) |
| `cascade_chat_unknown_wire_version.windsurf-rpc` | 2 | 162 | `cascade.v9.9` (UNKNOWN) | 2026-06-07 (synthetic) |
| `cascade_chat_truncated.windsurf-rpc` | 2 | 104 | n/a (garbage body) | 2026-06-07 (synthetic) |

All fixtures stay well under the 64 KiB cap.

Real-Windsurf capture evidence lives in SOW-customer-side artifacts
(`recorded_<*>_windsurf_<version>.windsurf-rpc`) which the codec
replays through the same harness; the capture window is documented
in the sidecar `.manifest.json` per the convention in
[`fixtures/PROVENANCE.md`](fixtures/PROVENANCE.md).

## 3. On-disk envelope (file header)

Per [`fixtures/README.md`](fixtures/README.md) §1, every fixture
starts with:

```
+----------------------+-------------------+--------------------+---------------------+
| magic = b"SGWRPC\0\0" | version (u16 LE) | frame count (u32 LE) |  reserved (u16 LE)  |
|       8 bytes         |     2 bytes      |       4 bytes        |       2 bytes       |
+----------------------+-------------------+--------------------+---------------------+
```

### Hex evidence — `cascade_chat_simple.windsurf-rpc`

```
00000000: 5347 5752 5043 0000 0100 0500 0000 0000  SGWRPC..........
                          ^^   ^^^^^^^^^^^^^^^^^^
                          ver  frame_count=5      reserved=0
                          =1   (LE)               (LE)
```

* Bytes `00..07`: ASCII `"SGWRPC"` + two `\0` padding bytes —
  `FIXTURE_MAGIC` (`b"SGWRPC\0\0"`).
* Bytes `08..09`: `0x0001` little-endian = `version=1`
  (`FIXTURE_VERSION`).
* Bytes `0a..0d`: `0x05000000` little-endian = `frame_count=5`.
* Bytes `0e..0f`: `0x0000` reserved.

The replay harness rejects:

* Wrong magic with [`ReplayError::BadMagic`](src/replay.rs).
* Wrong version with [`ReplayError::BadVersion`](src/replay.rs).
* Frame count mismatch with
  [`ReplayError::FrameCountMismatch`](src/replay.rs).

## 4. Per-frame record (the on-wire framing)

```
+--------------------+----------------------+----------------------+---------------------+---------------------+
| timestamp_ms (u64) | direction (u8)       | rpc_flag (u8)        | length (u32 BE)     | payload (length B)  |
|      8 bytes       |  0=client  1=server  |  gRPC-Web flag       |   gRPC-Web bytes    |                     |
+--------------------+----------------------+----------------------+---------------------+---------------------+
```

The `[rpc_flag][length BE][payload]` triple is stored **exactly as a
gRPC-Web writer emits them on the wire**. The replay harness extracts
these bytes verbatim and feeds them to
[`GrpcWebReader`](src/framing.rs) for the framing-layer contract.

### Hex evidence — first frame in `cascade_chat_simple.windsurf-rpc`

After the 16-byte envelope, the first frame starts at offset `0x10`:

```
00000010: 00ac 4d49 8b01 0000 0000 0000 <len-be>  ................
          ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ ^^ ^^^^^^^^
          timestamp_ms LE                 │  │
                                          direction=0
                                          (Client)
                                                 rpc_flag=0x00
                                                 length BE
```

* Bytes `10..17`: little-endian u64 timestamp_ms.
* Byte `18`: `0x00` → `Direction::Client`.
* Byte `19`: `0x00` → gRPC-Web flag byte; data frame.
* Bytes `1a..1d`: u32 BE → payload length.
* Bytes `1e..` for `length` bytes: the protobuf-encoded
  `CascadeRequest` payload.

## 5. Field-by-field protobuf evidence

The synthetic fixtures construct each protobuf field deliberately so
hex-level inspection lines up against
[`src/proto/windsurf.proto`](src/proto/windsurf.proto). Each
sub-section here picks one or two fixtures that exercise the field.

### 5.1 `messages` (tag 1, repeated `CascadeMessage`)

Wire-type 2 (length-delimited).

In `cascade_chat_streaming.windsurf-rpc`, the request payload encodes
4 `CascadeMessage` entries in proto order: system + user + assistant
+ user. Tag `0x0a` repeats four times in the payload. Inner tags
`0x0a` (role, tag 1) and `0x12` (content, tag 2) match the proto
declaration.

### 5.2 `model_name` (tag 2, string)

Tag `0x12` (wire-type 2). Visible in every fixture's request payload.
`cascade_chat_with_tools.windsurf-rpc` advertises
`claude-3.5-sonnet`; `cascade_chat_simple.windsurf-rpc` advertises
`gpt-4o`.

### 5.3 `max_tokens` (tag 3, optional uint32)

Tag `0x18` (wire-type 0, varint). The streaming fixture sets
`max_tokens=1024` → varint `0x80 0x08`.

### 5.4 `tool_declarations` (tag 4, repeated `CascadeToolDecl`)

Tag `0x22` (wire-type 2). `cascade_chat_with_tools.windsurf-rpc`
attaches 2 declarations; the inner `name` (tag 1) + `schema` (tag 2)
fields decode correctly under the codec's typed surface.

### 5.5 `workspace_id` (tag 5, optional string)

Tag `0x2a` (wire-type 2). Every fixture stamps a synthetic workspace
id (`FAKE_WORKSPACE_*`). The redaction policy in
[`fixtures/PROVENANCE.md`](fixtures/PROVENANCE.md) §5.5 documents how
real captures sanitise this.

### 5.6 `cascade_wire_version` (tag 99, optional string)

Tag `0x9a 0x06` (high-tag varint for tag 99 wire-type 2). Pinned per
fixture. Unknown stamps trigger `windsurf_wire_version_unsupported`
(SLICE 76 envelope decode); missing stamps fall back to the SHA-256
preamble hash check via the env var.

### 5.7 `CascadeResponseDelta.usage.output_tokens` (tag 2 inside tag 4)

Tag `0x22 <len> 0x10 <varint>` on the terminal delta. The replay
harness asserts the maximum observed `output_tokens` in
`ReplayReport.cumulative_output_tokens`. Monotonicity is documented
by the streaming fixture's terminal delta carrying the final tally
of `47`.

### 5.8 `CascadeResponseDelta.finish_reason` (tag 3, optional string)

* `cascade_chat_simple.windsurf-rpc`: `"stop"` on the terminal delta.
* `cascade_chat_with_tools.windsurf-rpc`: `"tool_calls"` on the
  terminal delta.
* `cascade_chat_streaming.windsurf-rpc`: `"stop"` on the terminal
  delta after 8 streaming chunks.

### 5.9 End-of-stream trailers (`flags = 0x02` or `0x80`)

The gRPC-Web trailers blob carries `grpc-status:<n>` and an
optional `grpc-message:<text>`. The replay harness detects the
trailers frame via `Frame::is_end_of_stream()` and inspects the
payload prefix to flag `upstream_error` when the status is non-zero.

Hex evidence — `cascade_chat_error.windsurf-rpc`:

```
                  02 00 00 00 3a 67 72 70 63 2d 73 74 61 74 75 73  ....:grpc-status
                  ^^ ^^^^^^^^^^^ ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
                  │     │        "grpc-status..."
                  │     length=0x3a=58 bytes
                  flag=0x02 (EOS)
                  ...3a 31 33 0d 67 72 70 63 2d 6d 65 73 73 61 67 65 3a 75
                       :13.grpc-message:u
                  ...70 73 74 72 65 61 6d 20 70 72 6f 76 69 64 65 72 20
                       pstream provider
                  ...72 65 74 75 72 6e 65 64 20 35 30 30
                       returned 500
```

* Trailers payload length = `0x3a` = 58 bytes.
* Payload: `grpc-status:13\rgrpc-message:upstream provider returned 500`.
* `13` = gRPC `INTERNAL` code → `upstream_error = true` → no commit
  per design.md §4.4.

## 6. Replay harness cross-check

The replay harness (`src/replay.rs::replay_fixture`) and the
integration tests under [`tests/integration_test.rs`](tests/integration_test.rs)
are the canonical cross-check: every claim in §5 above has a
corresponding test that loads the fixture, replays it, and asserts
the documented shape. If any test fails after a codec change, this
document is the source of truth for what the wire should look like
and `src/replay.rs`'s reader is the one that needs to match.

The expected-report manifests under
[`fixtures/synthetic/*.windsurf-rpc.manifest.json`](fixtures/synthetic/)
encode the same evidence in machine-readable form so a reviewer can
diff manifest-vs-replay-report without re-reading hex.

## 7. Real-capture evidence (NOT shipped here)

Per the legal posture in [`SOW.md`](SOW.md) and
[`design.md`](../../docs/specs/coverage/D18_windsurf_mitm/design.md)
§1, real Windsurf capture evidence lives in SOW-customer-side
artifacts. The capture workflow is documented in
[`fixtures/PROVENANCE.md`](fixtures/PROVENANCE.md) §6.

## 8. Standing "do not" list

* This document MUST NOT paste a vendor `.proto` verbatim.
* This document MUST NOT carry a Codeium / Windsurf copyright header.
* This document MUST NOT reference a CI-invoked capture path.
* This document MUST NOT advertise live `server.codeium.com` /
  `windsurf-server.codeium.com` traffic as part of the codec's
  tested surface — only the recorded fixtures are tested; live
  traffic is the SOW customer's deployment.
