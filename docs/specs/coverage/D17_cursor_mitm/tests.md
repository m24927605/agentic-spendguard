# D17 — Tests

Companion to [`design.md`](design.md) and [`implementation.md`](implementation.md). All tests are **fixture-driven and offline**. No live Cursor traffic in CI; per design §4 there is no path by which CI hits `api.cursor.sh`.

> **EXPERIMENTAL build only.** Every test below requires `--features cursor-mitm-experimental`. A non-experimental `cargo test --workspace` MUST NOT compile or run the codec crate.

## 1. Test taxonomy

| Layer | Tooling | Lives in |
|-------|---------|----------|
| Unit | `cargo test -p spendguard-cursor-codec --lib` | `services/cursor_codec/src/**` `#[cfg(test)]` |
| Integration (offline) | `cargo test -p spendguard-cursor-codec --tests` | `services/cursor_codec/tests/` |
| Egress-proxy integration | `cargo test -p spendguard-egress-proxy --features cursor-mitm-experimental` | `services/egress_proxy/tests/cursor_mitm.rs` |
| Demo regression | `make demo-cursor-mitm-fixture` | `Makefile` + `services/egress_proxy/src/demo.rs` |
| Build / lint gates | `cargo build --workspace` MUST succeed with NO experimental feature; `cargo build --workspace --features cursor-mitm-experimental` MUST succeed | CI matrix |

## 2. Fixture corpus

`services/cursor_codec/fixtures/` ships exactly three golden recordings tagged with Cursor client version range:

| Fixture | Shape | Tagged Cursor version range | Purpose |
|---------|-------|-----------------------------|---------|
| `unary_chat_v1.cursor-rpc` | unary RPC, single response | `>=0.42.0,<0.45.0` | Happy-path reserve + commit |
| `streaming_chat_v1.cursor-rpc` | server-streaming, 12 chunks | `>=0.42.0,<0.45.0` | Streaming reserve + commit on terminal chunk |
| `partial_truncation_v1.cursor-rpc` | server-streaming, truncated at chunk 7 | `>=0.42.0,<0.45.0` | Reserve + release on upstream failure |

Each fixture is a SpendGuard-authored capture, stored verbatim including the original Connect-RPC framing bytes. Fixtures are committed under LFS-free git (size budget ≤ 64 KiB each — capture script enforces).

## 3. Unit tests (Slice S17_02 / S17_03 / S17_05)

### 3.1 Framing parser

| Test | File | Asserts |
|------|------|---------|
| `framing_reads_unary_frame` | `framing/reader.rs#tests` | 5-byte prefix + payload extracted; trailing bytes leftover handled |
| `framing_reads_streaming_chunks` | `framing/reader.rs#tests` | All 12 chunks emitted in order; trailing metadata flag honoured |
| `framing_rejects_truncated_prefix` | `framing/reader.rs#tests` | `CodecError::TruncatedFrame` raised; no panic |
| `framing_rejects_unknown_compression` | `framing/reader.rs#tests` | `flags & 0x80` set → `UnsupportedCompression`; no fallback |
| `framing_writer_roundtrip` | `framing/writer.rs#tests` | `reader → writer` byte-identical for all three fixtures |

### 3.2 Envelope decode

| Test | Asserts |
|------|---------|
| `envelope_decodes_unary_request` | All known fields extracted from `unary_chat_v1.cursor-rpc` |
| `envelope_preserves_unknown_fields` | Synthetic fixture with an unknown proto field tag survives decode → encode |
| `envelope_response_streaming_terminal_chunk` | Last chunk in `streaming_chat_v1` carries terminal flag |

### 3.3 Translator

| Test | Asserts |
|------|---------|
| `translator_maps_known_models` | Every entry in `MODEL_MAP` round-trips |
| `translator_unknown_model_routes_to_passthrough` | Returns `TranslatorError::UnknownModel`; pipeline must release reservation (verified in `pipeline_fixture_replay`) |
| `translator_preserves_extension_bag` | Unknown vendor fields survive to canonical `extensions` |

## 4. Integration tests (Slice S17_06 / S17_07 / S17_08)

### 4.1 Pipeline fixture replay — THE golden test

`tests/pipeline_fixture_replay.rs`:

```rust
#[tokio::test]
async fn unary_fixture_drives_reserve_and_commit() {
    let ledger = InMemoryLedger::new();
    let pipeline = CodecPipeline::for_test(&ledger);
    let fixture = load_fixture("unary_chat_v1.cursor-rpc");

    let resp = pipeline.process(fixture.as_request()).await.unwrap();

    let chain = ledger.audit_chain();
    assert_eq!(chain.len(), 2);                          // reserve + commit
    assert_matches!(chain[0], Event::Reserve { .. });
    assert_matches!(chain[1], Event::Commit  { .. });
    assert_eq!(resp.body_bytes(), fixture.expected_response_bytes());  // byte-for-byte
}

#[tokio::test]
async fn streaming_fixture_commits_on_terminal_chunk() { /* mirror with 12 chunks */ }

#[tokio::test]
async fn truncated_stream_releases_reservation() {
    // partial_truncation_v1.cursor-rpc: upstream drops at chunk 7
    // Assert: ledger.audit_chain() ends with Event::Release { reason: UpstreamTruncated }
}
```

### 4.2 Byte-for-byte preservation

`tests/byte_for_byte_preserve.rs`: replay each fixture, capture the codec's outbound response body, assert `==` against the original recorded bytes wherever SpendGuard did not intervene. SpendGuard MAY mutate audit/usage fields only; other field bytes must round-trip.

### 4.3 Egress-proxy integration

`services/egress_proxy/tests/cursor_mitm.rs`:

| Test | Asserts |
|------|---------|
| `proxy_routes_cursor_host_to_codec_when_feature_on` | Compiled with feature → `RouteTarget::Codec` |
| `proxy_falls_through_when_feature_off` | Build without feature → arm absent; `api.cursor.sh` returns the default 502 (no SOW config) |
| `stderr_banner_emitted_once_per_process` | First handled request emits banner; second request does not duplicate |

## 5. Version-gate tests

`tests/version_gate.rs`:

| Test | Asserts |
|------|---------|
| `version_in_range_accepts` | Cursor UA `0.43.2` → `Supported` |
| `version_above_range_rejects_loudly` | Cursor UA `0.45.0` → `VersionOutOfRange`; pipeline returns 503 with `X-SpendGuard-Codec-Break: version-out-of-range` header |
| `version_below_range_rejects_loudly` | Cursor UA `0.41.9` → same |
| `version_missing_ua_rejects` | No UA header → reject; never silently translate |

## 6. Demo regression

`make demo-cursor-mitm-fixture` runs against `streaming_chat_v1.cursor-rpc`. The demo script asserts:

1. Stderr banner appears once.
2. In-memory ledger ends with reserve + commit.
3. Canonical audit chain printed to stdout contains `reservation_id`, `commit_actual_tokens`, and `tenant_id` matching the demo config.
4. Exit code 0.

The demo is wired into `services/egress_proxy/tests/demo_modes.rs` so it runs in CI alongside the other demo-mode regressions, gated by `--features cursor-mitm-experimental`.

## 7. Negative / red-team tests

| Test | Asserts |
|------|---------|
| `experimental_marker_present_in_cargo_metadata` | `cargo metadata --features cursor-mitm-experimental | jq '.packages[]|select(.name=="spendguard-cursor-codec").metadata.experimental'` non-null |
| `non_experimental_build_excludes_crate` | `cargo metadata` (no features) → `spendguard-cursor-codec` IS in workspace but its rdeps from `egress_proxy` are absent; verified by parsing `cargo tree` output |
| `cli_install_include_cursor_refused_without_feature` | `spendguard install --include cursor` on a non-experimental build exits non-zero with `error: feature 'cursor-mitm-experimental' required` |
| `sow_doc_exists_and_marks_experimental` | `grep -E '^Status:[[:space:]]*EXPERIMENTAL' docs/customer/sow-cursor-mitm.md` matches |
| `sow_doc_contains_break_window_sla_template` | `grep -q 'Break-Window SLA' docs/customer/sow-cursor-mitm.md` |
| `no_vendor_proto_committed_verbatim` | Repo scan: `services/cursor_codec/proto/*.proto` files start with the SpendGuard attribution header sentinel; reviewer rejects diffs that import a vendor `.proto` |

## 8. CI matrix

| Job | OS | Features | Command |
|-----|----|----|---------|
| `default-build` | ubuntu-24.04 | none | `cargo build --workspace --locked` |
| `default-test` | ubuntu-24.04 | none | `cargo test --workspace --locked` |
| `experimental-build` | ubuntu-24.04 | `cursor-mitm-experimental` | `cargo build --workspace --features cursor-mitm-experimental --locked` |
| `experimental-test` | ubuntu-24.04 | `cursor-mitm-experimental` | `cargo test --workspace --features cursor-mitm-experimental --locked` |
| `demo-fixture` | ubuntu-24.04 | `cursor-mitm-experimental` | `make demo-cursor-mitm-fixture` |
| `negative-no-feature` | ubuntu-24.04 | none | tests in §7 that assert absence |

No macOS / Windows job — the codec runs only inside the SpendGuard egress-proxy sidecar, which is Linux-only.

## 9. What is NOT tested in CI

Explicitly out of scope, documented for the reviewer:

- Live `api.cursor.sh` connectivity.
- Real Cursor IDE binary interop.
- Vendor protocol updates after the fixture-capture date.

These are SOW operational responsibilities, not CI gates. The SOW addendum documents the break-window SLA the customer accepts.
