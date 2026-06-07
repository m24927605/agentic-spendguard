# Windsurf MITM Fixture Format

> **EXPERIMENTAL.** Fixtures under this directory drive the offline parser /
> decoder tests for `services/windsurf_codec`. Live Windsurf / Codeium
> traffic is NOT used in CI per D18 design.md §3 decision 7. SLICE 80
> ships **synthetic** fixtures only; real captures land in a sibling
> `recorded/` directory under SOW-customer-side workflows.

## 1. File extension and on-disk layout

Each fixture file has the extension `.windsurf-rpc` and the following on-disk
layout (little-endian unless otherwise noted):

```
+----------------------+-------------------+--------------------+---------------------+
| magic = b"SGWRPC\0\0" | version (u16 LE) | frame count (u32 LE) |  reserved (u16 LE)  |
|       8 bytes         |     2 bytes      |       4 bytes        |       2 bytes       |
+----------------------+-------------------+--------------------+---------------------+

Per-frame record (repeated frame_count times):

+--------------------+----------------------+----------------------+---------------------+---------------------+
| timestamp_ms (u64) | direction (u8)       | rpc_flag (u8)        | length (u32 BE)     | payload (length B)  |
|      8 bytes       |  0=client  1=server  |  gRPC-Web flag       |   gRPC-Web bytes    |                     |
+--------------------+----------------------+----------------------+---------------------+---------------------+
```

Key properties of this layout:

* The `length` and `rpc_flag` bytes are stored **exactly as gRPC-Web writes
  them on the wire** — big-endian length, `0x00`/`0x02`/`0x80` flag byte. The
  parser implementation in `src/framing.rs` can therefore consume the bytes
  `[rpc_flag][length BE][payload]` from each record without any per-record
  byte-order conversion.
* The outer envelope (`magic` / `version` / `frame count` / `timestamp_ms`) is
  stored little-endian so the dev harness can produce + read it on x86/aarch64
  hosts without endian-flipping.
* `version` starts at `1` and is bumped whenever the envelope layout changes.
  The reader rejects any version it does not recognise — there is no implicit
  forward compatibility.
* `direction` distinguishes a client-to-server frame (the
  `CascadeRequest`) from a server-to-client frame (a
  `CascadeResponseDelta` or the EOS trailers frame).
* `timestamp_ms` is a UNIX epoch milliseconds value at capture time. For
  synthetic fixtures it is the wall clock at fixture generation; for recorded
  fixtures it is the wall clock at MITM capture.

## 2. Synthetic vs recorded

`synthetic/` holds 6 fixtures crafted from the **public gRPC-Web framing
spec** plus a minimal placeholder Cascade protobuf envelope. They are
deterministic, small (each <2 KiB, well under the 64 KiB cap), and explicitly
labelled in their filenames as `cascade_chat_*`.

`recorded/` is **reserved for SOW-customer-side workflows**. The codec's
public CI does NOT exercise live `server.codeium.com` traffic. Recorded
fixtures live on customer infrastructure with a sidecar
`<name>.windsurf-rpc.manifest.json` documenting Windsurf client version,
capture date, and the redaction-script SHA-256 per `PROVENANCE.md`.

## 3. Redaction policy

Each `.windsurf-rpc` fixture committed under `synthetic/` is built from
SpendGuard-authored synthetic content. The literal sentinels
`FAKE_SOW_USER_TURN`, `FAKE_SOW_SYSTEM`, `FAKE_WORKSPACE_*`,
`FAKE_REDACTED_SCHEMA` document the redaction shape that real captures
would carry.

The fixture test suite gates against any of the following leaking into
fixture payloads — reviewer rejects PRs that introduce real Codeium
credentials by accident:

* `sk-codeium-` (legacy Codeium API key prefix)
* `wsf_` (Windsurf session-token prefix observed in public Codeium
  docs)
* `codeium_pat_` (Codeium personal access token prefix)
* `cdm_` (Codeium internal session prefix observed in capture
  research)

See `tests/integration_test.rs::no_secret_leakage_in_fixtures` for the
literal regression guard.

## 4. Reader contract

`GrpcWebReader::read_frame` consumes one `[rpc_flag][length BE][payload]`
tuple per call. The outer envelope (`magic` / `version` / `frame count`) is
parsed by the higher-level fixture reader in `src/replay.rs::read_fixture_bytes`.

## 5. Size budget

No individual fixture file may exceed 64 KiB. The synthetic fixtures in
this slice are <1 KiB each because the placeholder Cascade envelope is
small and there are at most a dozen frames per fixture. SLICE 80
fixtures stay well within budget.

## 6. Generating new fixtures

`examples/regenerate_fixtures.rs` reproduces every byte of every fixture
deterministically:

```sh
cargo run --manifest-path services/windsurf_codec/Cargo.toml \
    --example regenerate_fixtures
```

Synthetic fixtures committed to this directory are produced exclusively
by that binary. The `--release` flag is optional; the synthesis is
hand-auditable byte-for-byte against `src/replay.rs::write_fixture_bytes`.
