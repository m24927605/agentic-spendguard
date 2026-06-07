# Cursor Wire Protocol — Hex Evidence

> **EXPERIMENTAL.** This document is the field-by-field hex evidence
> backing the protobuf description in
> [`src/proto/cursor.proto`](src/proto/cursor.proto). It is
> SpendGuard's own observation of the Cursor IDE Agent wire format;
> no vendor source is included. See [`RECON.md`](RECON.md) for the
> framing-layer recon and [`README.md`](README.md) for the legal
> posture.
>
> Per [`design.md`](../../docs/specs/coverage/D17_cursor_mitm/design.md)
> §8 decision 6 and
> [`review-standards.md`](../../docs/specs/coverage/D17_cursor_mitm/review-standards.md)
> §2 (`R1`-`R5`): the proto schema is a black-box reconstruction;
> field names are SpendGuard-chosen; field numbers match the wire as
> observed. No vendor `.proto` is included verbatim.

## 1. Document scope

This is the D17 SLICE 8 deliverable that
[`review-standards.md`](../../docs/specs/coverage/D17_cursor_mitm/review-standards.md)
§2 (`R2`) demands:

> `PROTOCOL.md` documents capture date, Cursor client version range,
> and field-by-field observation evidence (hex excerpts). Reviewer
> rejects "TODO" placeholders here.

The SLICE 8 charter is "fixtures + replay harness without live capture"
because the legal posture in
[`design.md`](../../docs/specs/coverage/D17_cursor_mitm/design.md) §1
forbids running the Cursor binary through MITM in CI. Real-capture
hex evidence lives in SOW-customer-side artifacts on the customer's
infrastructure and is referenced (NOT pasted) here by manifest name.

The evidence below is derived from the SLICE 8 synthetic fixtures
under [`fixtures/synthetic/`](fixtures/synthetic/). Each fixture is
hand-auditable: a reviewer can `xxd` the file and verify the
documented wire-shape claim. The synthetic fixtures are constructed
from the public Connect-RPC framing spec
([connectrpc.com/docs/protocol](https://connectrpc.com/docs/protocol))
plus SpendGuard's reconstructed envelope description; they exercise
the codec the same way real captures would, without exposing real
Cursor session traffic.

## 2. Synthetic capture provenance

| Fixture | Frames | Bytes | Cursor version | Capture date |
|---------|-------:|------:|----------------|--------------|
| `synthetic_unary_v1.cursor-rpc` (SLICE 1) | 1 | 64 | synthetic | 2026-05-24 (SLICE 1) |
| `synthetic_streaming_chunked_v1.cursor-rpc` (SLICE 1) | 5 | 258 | synthetic | 2026-05-24 (SLICE 1) |
| `synthetic_multiturn_v1.cursor-rpc` (SLICE 8) | 5 | 361 | synthetic | 2026-06-07 |
| `synthetic_tool_calls_v1.cursor-rpc` (SLICE 8) | 4 | 431 | synthetic | 2026-06-07 |
| `synthetic_error_response_v1.cursor-rpc` (SLICE 8) | 2 | 157 | synthetic | 2026-06-07 |
| `synthetic_long_stream_v1.cursor-rpc` (SLICE 8) | 15 | 438 | synthetic | 2026-06-07 |

All fixtures stay well under the 64 KiB `C4` cap.

Real-Cursor capture evidence lives in SOW-customer-side artifacts
(`recorded_<*>_cursor_<version>.cursor-rpc`) which the codec replays
through the same harness; the capture window is documented in the
sidecar `.manifest.json` per `R4`. SLICE 8 ships the synthetic
corpus only.

## 3. On-disk envelope (file header)

Per [`fixtures/README.md`](fixtures/README.md) §1, every fixture
starts with:

```
+----------------------+-------------------+--------------------+---------------------+
| magic = b"SGCRPC\0\0" | version (u16 LE) | frame count (u32 LE) |  reserved (u16 LE)  |
|       8 bytes         |     2 bytes      |       4 bytes        |       2 bytes       |
+----------------------+-------------------+--------------------+---------------------+
```

### Hex evidence — `synthetic_multiturn_v1.cursor-rpc`

```
00000000: 5347 4352 5043 0000 0100 0500 0000 0000  SGCRPC..........
                          ^^   ^^^^^^^^^^^^^^^^^^
                          ver  frame_count=5      reserved=0
                          =1   (LE)               (LE)
```

* Bytes `00..07`: ASCII `"SGCRPC"` + two `\0` padding bytes — the
  fixed [`FIXTURE_MAGIC`](src/replay.rs).
* Bytes `08..09`: `0x0001` little-endian = `version=1`
  ([`FIXTURE_VERSION`](src/replay.rs)).
* Bytes `0a..0d`: `0x05000000` little-endian = `frame_count=5`.
* Bytes `0e..0f`: `0x0000` reserved.

The replay harness rejects:

* Wrong magic with [`ReplayError::BadMagic`](src/replay.rs).
* Wrong version with [`ReplayError::BadVersion`](src/replay.rs).
* Frame count mismatch with
  [`ReplayError::FrameCountMismatch`](src/replay.rs).

## 4. Per-frame record (the on-wire framing)

Per [`fixtures/README.md`](fixtures/README.md) §1:

```
+--------------------+----------------------+----------------------+---------------------+---------------------+
| timestamp_ms (u64) | direction (u8)       | rpc_flag (u8)        | length (u32 BE)     | payload (length B)  |
|      8 bytes       |  0=client  1=server  |  same as Connect bit |   Connect-RPC bytes |                     |
+--------------------+----------------------+----------------------+---------------------+---------------------+
```

Crucially, the `[rpc_flag][length BE][payload]` triple is stored
**exactly as a Connect-RPC writer emits them on the wire**. The
replay harness extracts these bytes verbatim and feeds them to
[`ConnectRpcReader`](src/framing.rs) for the W1 5-byte-prefix
contract.

### Hex evidence — first frame in `synthetic_multiturn_v1.cursor-rpc`

After the 16-byte envelope, the first frame starts at offset `0x10`:

```
00000010: 0008 9289 8f01 0000 0000 0000 00cc 0a29  ...............)
          ^^^^^^^^^^^^^^^^^^^^^^^^^^ ^^ ^^ ^^^^^^^^
          timestamp_ms = 0x0000_018f_8989_9200    │  │  │
          (little-endian u64; capture wall time)  │  │  │
                                                  │  │  │
                                       direction=0│  │  │
                                       (Client)   │  │  │
                                                  rpc_flag=0x00 (data frame)
                                                     length=0x000000cc=204 bytes BE
                                                        payload: protobuf-encoded
                                                        CursorChatRequest
```

* Bytes `10..17`: `0x0000018f89899200` little-endian — capture
  timestamp in UNIX epoch milliseconds.
* Byte `18`: `0x00` → `Direction::Client` (client → server).
* Byte `19`: `0x00` → Connect-RPC flag byte; neither `0x01`
  (compressed) nor `0x02` (end-of-stream) — a normal data frame
  (`W1`/`W2`/`W3`).
* Bytes `1a..1d`: `0x000000cc` big-endian → payload length = 204
  bytes.
* Bytes `1e..` for 204 bytes: the protobuf-encoded
  [`CursorChatRequest`](src/proto/cursor.proto) payload.

## 5. Field-by-field protobuf evidence

The synthetic fixtures construct each protobuf field deliberately so
hex-level inspection lines up against
[`src/proto/cursor.proto`](src/proto/cursor.proto). Each
sub-section here picks one or two fixtures that exercise the field.

### 5.1 `messages` (tag 1, repeated `Message`)

Wire-type 2 (length-delimited), per-message length-delimited.

In `synthetic_multiturn_v1.cursor-rpc`, offset `0x1e..` (the protobuf
payload starts here):

```
00000020:           0a29 0a06 7379 7374 656d 121f
                    ^^ ^^ ^^^^^^^^^^^^^^^^^^^^^^^^
                    │  │  inner Message (tag 1=role, len 6; tag 2=content, len 31)
                    │  outer Message length-prefix = 0x29 = 41 bytes
                    outer tag = 0x0a → tag 1 (messages), wire-type 2

00000030: 596f 7520 6172 6520 4375 7273 6f72 2041
00000040: 6765 6e74 2e20 4265 2074 6572 7365 2e0a
          ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ ^^
          "You are Cursor Agent. Be terse."     │
                                             0x0a tag → next messages entry
```

* Tag `0x0a` repeats four times in the payload, once per
  `Message` entry. The translator preserves order.
* Inner tags `0x0a` (role, tag 1) and `0x12` (content, tag 2) match
  the proto declaration.

Decoded result: 4 messages, first role=`system` content=
`You are Cursor Agent. Be terse.`. Confirmed by replay harness
assertion in
[`tests/replay_test.rs`](tests/replay_test.rs)::`replay_multiturn_conversation_lands_full_cycle`.

### 5.2 `model` (tag 2, string)

Tag `0x12` (wire-type 2). In `synthetic_multiturn_v1.cursor-rpc`,
visible near offset `0xb0`:

```
000000b0: 636c 6175 6465 2d33 2e35 2d73 6f6e 6e65  claude-3.5-sonne
000000c0: 7474
          ^^^^
          "claude-3.5-sonnet" terminator + next tag
```

The fixture documents that the `model` string is stored verbatim
(no Cursor-internal prefix at the wire boundary).

### 5.3 `system` (tag 3, optional string)

Tag `0x1a` (wire-type 2). The same multi-turn fixture exhibits both
the top-level `system` field AND a leading `role=system` message,
exercising the SLICE 5 translator's precedence rule (`role=system`
in messages wins; top-level `system` dropped). See
[`src/translate.rs`](src/translate.rs) §3.

### 5.4 `max_tokens` (tag 4, optional uint32) and `temperature` (tag 5, optional float)

Tag `0x20` (wire-type 0, varint) and `0x2d` (wire-type 5, fixed32).
Visible at the tail of every fixture's request payload. Long-stream
fixture sets `max_tokens=1024` → encoded as varint `0x80 0x08`.

### 5.5 `CursorChatResponseChunk.cumulative_output_tokens` (tag 4, optional uint32)

Monotonically non-decreasing across the 12+1 chunks in
`synthetic_long_stream_v1.cursor-rpc`. The replay harness asserts
the monotonicity in
[`tests/replay_test.rs`](tests/replay_test.rs)::`replay_long_stream_handles_at_least_ten_chunks`.

### 5.6 `CursorChatResponseChunk.finish_reason` (tag 3, optional string)

* `synthetic_multiturn_v1.cursor-rpc`: `"stop"` on the terminal
  data chunk (frame index 3).
* `synthetic_tool_calls_v1.cursor-rpc`: `"tool_calls"` on the
  terminal data chunk — exercises the non-stop terminal case
  Cursor Agent mode produces.
* `synthetic_long_stream_v1.cursor-rpc`: `"stop"` on the terminal
  data chunk (frame index 13).

### 5.7 End-of-stream trailers (`flags = 0x02`)

The Connect-RPC trailers blob carries `grpc-status:<n>` and an
optional `grpc-message:<text>`. The replay harness detects the
trailers frame via `Frame::is_end_of_stream()` and inspects the
payload prefix to flag `upstream_error` when the status is
non-zero.

Hex evidence — `synthetic_error_response_v1.cursor-rpc`:

```
00000060: 0000 3a67 7270 632d 7374 6174 7573 3a31  ..:grpc-status:1
00000070: 330d 6772 7063 2d6d 6573 7361 6765 3a75  3.grpc-message:u
00000080: 7073 7472 6561 6d20 7072 6f76 6964 6572  pstream provider
00000090: 2072 6574 7572 6e65 6420 3530 30          returned 500
```

* Trailers payload length = `0x3a` = 58 bytes.
* Payload: `grpc-status:13\rgrpc-message:upstream provider returned 500`.
* `13` = gRPC `INTERNAL` code → `upstream_error = true` → no commit
  per the `P3` release-and-pass-through contract.

## 6. Replay harness cross-check

The [`tests/replay_test.rs`](tests/replay_test.rs) suite is the
canonical cross-check: every claim in §5 above has a corresponding
test that loads the fixture, replays it, and asserts the documented
shape. If any test fails after a codec change, this document is the
source of truth for what the wire should look like and
[`src/replay.rs`](src/replay.rs)'s reader is the one that needs to
match.

The expected-report manifests under
[`fixtures/synthetic/*.cursor-rpc.manifest.json`](fixtures/synthetic/)
encode the same evidence in machine-readable form so a reviewer can
diff manifest-vs-replay-report without re-reading hex.

## 7. Real-capture evidence (NOT shipped here)

Per the legal posture in
[`SOW.md`](SOW.md) and
[`design.md`](../../docs/specs/coverage/D17_cursor_mitm/design.md)
§1, real Cursor capture evidence lives in SOW-customer-side artifacts.
The capture script under `tools/capture/` (planned, not yet wired)
generates a sidecar `<name>.cursor-rpc.manifest.json` next to each
recorded fixture documenting:

* Cursor client version (semver from User-Agent or app menu).
* Capture date (UNIX epoch ms).
* Field-by-field hex excerpts mapped to the proto field names.
* Operator who ran the capture (customer-side).

That manifest is referenced by manifest name in any SLICE 8 PR that
adds a recorded fixture to `fixtures/recorded/`. The recorded fixture
+ manifest pair is what a future codec break investigation reads to
trace which Cursor version drifted on which field.

## 8. Standing "do not" list (`R3`/`R5`)

* This document MUST NOT paste a vendor `.proto` verbatim.
* This document MUST NOT carry a Cursor copyright header.
* This document MUST NOT reference a CI-invoked capture path.
* This document MUST NOT advertise live `api.cursor.sh` traffic as
  part of the codec's tested surface — only the recorded fixtures
  are tested; live traffic is the SOW customer's deployment.
