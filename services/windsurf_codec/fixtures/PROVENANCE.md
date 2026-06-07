# Windsurf MITM Fixture Provenance

> **EXPERIMENTAL — SOW only.** This document is the field-by-field
> provenance ledger backing the 6 synthetic fixtures under
> [`synthetic/`](synthetic/). It mirrors the D17
> `services/cursor_codec/fixtures/README.md` + `PROTOCOL.md` pattern.
>
> Per D18 design.md §3 decision 7: live Windsurf / Codeium captures
> NEVER appear in CI. Real-capture evidence lives in SOW-customer-side
> artifacts under the customer's own legal sign-off. SLICE 80 ships
> the synthetic corpus only.

last_verified_capture: 2026-06-07 (synthetic — SLICE 80)
redaction_sha256: synthetic-no-redaction-needed
sow_id: synthetic
windsurf_min_version: synthetic
windsurf_max_version: synthetic

## 1. Document scope

This is the D18 SLICE 80 deliverable that review-standards demand:

> `PROVENANCE.md` documents capture date, Windsurf client version
> range, redaction SHA-256, and SOW ID per fixture. Reviewer rejects
> "TODO" placeholders here.

The SLICE 80 charter is "fixtures + replay harness without live
capture" because the legal posture in
[`SOW.md`](../SOW.md) §5 forbids running the Windsurf binary
through MITM in CI. Real-capture hex evidence lives in SOW-customer-
side artifacts on the customer's infrastructure and is referenced
(NOT pasted) here by manifest name.

## 2. Synthetic fixture inventory

| Fixture | Frames | Bytes | Wire version | Notes |
|---------|-------:|------:|--------------|-------|
| `cascade_chat_simple.windsurf-rpc` | 5 | 362 | `cascade.v2.0` | happy-path: 1 user turn + 3 streaming deltas + EOS |
| `cascade_chat_with_tools.windsurf-rpc` | 3 | 392 | `cascade.v2.1` | Cascade Agent with 2 tool declarations + `finish_reason=tool_calls` |
| `cascade_chat_streaming.windsurf-rpc` | 11 | 645 | `cascade.v2.0` | 8 streaming deltas + terminal usage |
| `cascade_chat_error.windsurf-rpc` | 2 | 195 | `cascade.v2.0` | upstream `grpc-status:13` trailers → no commit |
| `cascade_chat_unknown_wire_version.windsurf-rpc` | 2 | 162 | `cascade.v9.9` (UNKNOWN) | exercises `windsurf_wire_version_unsupported` gate |
| `cascade_chat_truncated.windsurf-rpc` | 2 | 104 | n/a (garbage body) | exercises `decoder_skipped` best-effort fallback |

All 6 fixtures stay well under the 64 KiB cap and can be regenerated
via `cargo run --example regenerate_fixtures`.

## 3. On-disk envelope (file header)

Per [`fixtures/README.md`](README.md) §1, every fixture starts with:

```
+----------------------+-------------------+--------------------+---------------------+
| magic = b"SGWRPC\0\0" | version (u16 LE) | frame count (u32 LE) |  reserved (u16 LE)  |
|       8 bytes         |     2 bytes      |       4 bytes        |       2 bytes       |
+----------------------+-------------------+--------------------+---------------------+
```

* Bytes `00..07`: ASCII `"SGWRPC"` + two `\0` padding bytes — the
  fixed `FIXTURE_MAGIC` (matches the D17 cursor_codec `SGCRPC` pattern
  but spelt for "SpendGuard Windsurf RPC").
* Bytes `08..09`: `0x0001` little-endian = `version=1`.
* Bytes `0a..0d`: `u32 LE` frame count.
* Bytes `0e..0f`: reserved, zero.

The replay harness rejects:

* Wrong magic with `ReplayError::BadMagic`.
* Wrong version with `ReplayError::BadVersion`.
* Frame count mismatch with `ReplayError::FrameCountMismatch`.

## 4. Per-frame record (the on-wire framing)

Per [`fixtures/README.md`](README.md) §1:

```
+--------------------+----------------------+----------------------+---------------------+---------------------+
| timestamp_ms (u64) | direction (u8)       | rpc_flag (u8)        | length (u32 BE)     | payload (length B)  |
|      8 bytes       |  0=client  1=server  |  gRPC-Web flag       |   gRPC-Web bytes    |                     |
+--------------------+----------------------+----------------------+---------------------+---------------------+
```

Crucially, the `[rpc_flag][length BE][payload]` triple is stored
**exactly as a gRPC-Web writer emits them on the wire**. The replay
harness extracts these bytes verbatim and feeds them to
`GrpcWebReader` for the framing-layer contract.

## 5. Field-by-field Cascade protobuf evidence

The synthetic fixtures construct each protobuf field deliberately so
hex-level inspection lines up against
[`src/proto/windsurf.proto`](../src/proto/windsurf.proto). Each
sub-section here picks one fixture that exercises the field.

### 5.1 `messages` (tag 1, repeated `CascadeMessage`)

Wire-type 2 (length-delimited), per-message length-delimited.

In `cascade_chat_streaming.windsurf-rpc`, the request payload encodes
4 `CascadeMessage` entries in proto order:

* `system` → `FAKE_SOW_SYSTEM: be thorough`
* `user`   → `FAKE_SOW_USER: write the longer reply`
* `assistant` → `FAKE_SOW_ASSISTANT_PREVIOUS: prior turn`
* `user`   → `FAKE_SOW_USER_FOLLOWUP: continue`

Tag `0x0a` repeats four times; the translator preserves order
verbatim (no system-prepending heuristic — Cascade's wire shape is
already canonical OpenAI order).

### 5.2 `model_name` (tag 2, string)

Tag `0x12` (wire-type 2). All fixtures advertise a publicly-named
upstream model (`gpt-4o`, `claude-3.5-sonnet`, etc.). The codec
neither rewrites nor classifies the model at this boundary —
classification happens in `services/output_predictor`.

### 5.3 `max_tokens` (tag 3, optional uint32)

Tag `0x18` (wire-type 0, varint). `cascade_chat_streaming.windsurf-rpc`
sets `max_tokens=1024` → varint `0x80 0x08`.

### 5.4 `tool_declarations` (tag 4, repeated `CascadeToolDecl`)

Tag `0x22` (wire-type 2). `cascade_chat_with_tools.windsurf-rpc`
attaches 2 declarations (`read_file`, `list_dir`), each with a
schema string redacted to `FAKE_REDACTED_SCHEMA`.

### 5.5 `workspace_id` (tag 5, optional string)

Tag `0x2a` (wire-type 2). Every fixture stamps a synthetic workspace
id (`FAKE_WORKSPACE_*`) to document the redaction shape. Real
captures replace the value with the literal sentinel per the SOW
redaction policy.

### 5.6 `cascade_wire_version` (tag 99, optional string)

Tag `0x9a 0x06` (high-tag varint for tag 99 wire-type 2). Pinned per
fixture in §2 above. Unknown stamps trigger
`windsurf_wire_version_unsupported`; missing stamps fall back to the
SHA-256 preamble hash check (which must then be registered via
`SPENDGUARD_WINDSURF_PREAMBLE_HASHES`).

### 5.7 `CascadeResponseDelta.usage` (tag 4, optional `CascadeUsage`)

`cascade_chat_simple.windsurf-rpc` (terminal delta) carries
`{input_tokens=11, output_tokens=18}`. The replay harness extracts
`output_tokens` for the SLICE 78 commit lane.

### 5.8 `CascadeResponseDelta.finish_reason` (tag 3, optional string)

* `cascade_chat_simple.windsurf-rpc`: `"stop"` on the terminal delta.
* `cascade_chat_with_tools.windsurf-rpc`: `"tool_calls"` on the
  terminal delta — exercises the non-stop terminal case Cascade
  Agent mode produces.
* `cascade_chat_streaming.windsurf-rpc`: `"stop"` on the terminal
  delta after 8 streaming chunks.

### 5.9 End-of-stream trailers (`flags = 0x02` or `0x80`)

The gRPC-Web trailers blob carries `grpc-status:<n>` and an optional
`grpc-message:<text>`. The replay harness detects the trailers
frame via `Frame::is_end_of_stream()` (which accepts either bit) and
inspects the payload prefix to flag `upstream_error` when the status
is non-zero.

## 6. Real-capture evidence (NOT shipped here)

Per the legal posture in [`SOW.md`](../SOW.md) and
[`design.md`](../../../docs/specs/coverage/D18_windsurf_mitm/design.md)
§1, real Windsurf capture evidence lives in SOW-customer-side
artifacts. The capture script (planned, not yet wired) generates a
sidecar `<name>.windsurf-rpc.manifest.json` next to each recorded
fixture documenting:

* Windsurf client version (semver from User-Agent or app menu).
* Codeium client version range.
* Capture date (UNIX epoch ms).
* Field-by-field hex excerpts mapped to the proto field names.
* Operator who ran the capture (customer-side).

That manifest is referenced by manifest name in any future PR that
adds a recorded fixture to `fixtures/recorded/`. The recorded
fixture + manifest pair is what a future codec break investigation
reads to trace which Windsurf version drifted on which field.

## 7. Standing "do not" list

* This document MUST NOT paste a vendor `.proto` verbatim.
* This document MUST NOT carry a Codeium / Windsurf copyright header.
* This document MUST NOT reference a CI-invoked capture path.
* This document MUST NOT advertise live `server.codeium.com` /
  `windsurf-server.codeium.com` traffic as part of the codec's
  tested surface — only the recorded fixtures are tested; live
  traffic is the SOW customer's deployment.
