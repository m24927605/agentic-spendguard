# D17 — Acceptance Gates

Per build plan §3, every gate listed here must be **100% feasible** at slice-spec time: runnable in the current repo state, no live Cursor traffic, no third-party action required, reproducible by the `superpowers:code-reviewer` skill.

> **EXPERIMENTAL — SOW only.** D17 is shipped behind `--features cursor-mitm-experimental`. Default workspace builds MUST NOT compile the codec. Acceptance verifies BOTH paths: experimental enabled AND experimental disabled.

## 1. Repository-state gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A1.1` | `services/cursor_codec` exists as a workspace member | `cargo metadata --format-version 1 \| jq -e '.packages[] \| select(.name == "spendguard-cursor-codec")'` |
| `A1.2` | `services/cursor_codec/Cargo.toml` carries the experimental marker | `cargo metadata --format-version 1 \| jq -e '.packages[] \| select(.name == "spendguard-cursor-codec") \| .metadata.experimental'` non-null |
| `A1.3` | `services/cursor_codec/PROTOCOL.md` exists and documents capture date + Cursor version range | `grep -qE 'Cursor client version range: >=[0-9]+\.[0-9]+\.[0-9]+,<[0-9]+\.[0-9]+\.[0-9]+' services/cursor_codec/PROTOCOL.md` |
| `A1.4` | `services/cursor_codec/proto/cursor_observed.proto` starts with the SpendGuard attribution header | `head -5 services/cursor_codec/proto/cursor_observed.proto \| grep -q 'SpendGuard-authored observation'` |
| `A1.5` | Three golden fixtures exist and are < 64 KiB each | `find services/cursor_codec/fixtures -name '*.cursor-rpc' -size -64k \| wc -l` ≥ 3 |
| `A1.6` | `docs/customer/sow-cursor-mitm.md` exists | `test -f docs/customer/sow-cursor-mitm.md` |
| `A1.7` | SOW doc marked experimental above the fold | `head -20 docs/customer/sow-cursor-mitm.md \| grep -qE '^Status:[[:space:]]*EXPERIMENTAL'` |
| `A1.8` | SOW doc carries the Break-Window SLA template section | `grep -q '^## Break-Window SLA' docs/customer/sow-cursor-mitm.md` |
| `A1.9` | SOW doc has `noindex: true` front-matter | `grep -q '^noindex: true' docs/customer/sow-cursor-mitm.md` |
| `A1.10` | README adapter table marks Cursor as EXPERIMENTAL (or absent) — must not list it as GA | `! grep -E 'Cursor.*GA' README.md` |

## 2. Build gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A2.1` | Default workspace builds WITHOUT the experimental crate compiling | `cargo build --workspace --locked` exits 0 AND `cargo tree -p spendguard-egress-proxy \| grep -v spendguard-cursor-codec` (no rdep) |
| `A2.2` | Experimental workspace builds | `cargo build --workspace --features cursor-mitm-experimental --locked` exits 0 |
| `A2.3` | No new MSRV warnings | `cargo build --workspace --features cursor-mitm-experimental -- -D warnings` exits 0 |
| `A2.4` | Clippy clean for the new crate | `cargo clippy -p spendguard-cursor-codec --all-targets -- -D warnings` exits 0 |
| `A2.5` | `cargo deny check` passes (no new disallowed licences from `prost`, `prost-types`, `tokio-util`) | `cargo deny check` exits 0 |
| `A2.6` | `spendguard install --include cursor` is unavailable in default build | `cargo run -p spendguard-cli -- install --include cursor` exits non-zero with `error: feature 'cursor-mitm-experimental' required` |

## 3. Unit-test gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A3.1` | All `spendguard-cursor-codec` unit tests green | `cargo test -p spendguard-cursor-codec --lib --features cursor-mitm-experimental` exits 0 |
| `A3.2` | Framing roundtrip green | `cargo test -p spendguard-cursor-codec --test framing_roundtrip --features cursor-mitm-experimental` exits 0 |
| `A3.3` | Envelope decode preserves unknown fields | `cargo test -p spendguard-cursor-codec envelope_preserves_unknown_fields --features cursor-mitm-experimental` exits 0 |
| `A3.4` | Translator MODEL_MAP round-trips | `cargo test -p spendguard-cursor-codec translator_maps_known_models --features cursor-mitm-experimental` exits 0 |

## 4. Integration-test gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A4.1` | Pipeline fixture replay (unary) green | `cargo test -p spendguard-cursor-codec --test pipeline_fixture_replay unary_fixture_drives_reserve_and_commit --features cursor-mitm-experimental` exits 0 |
| `A4.2` | Pipeline fixture replay (streaming) green | `cargo test -p spendguard-cursor-codec --test pipeline_fixture_replay streaming_fixture_commits_on_terminal_chunk --features cursor-mitm-experimental` exits 0 |
| `A4.3` | Pipeline fixture replay (truncation → release) green | `cargo test -p spendguard-cursor-codec --test pipeline_fixture_replay truncated_stream_releases_reservation --features cursor-mitm-experimental` exits 0 |
| `A4.4` | Byte-for-byte preservation across all three fixtures green | `cargo test -p spendguard-cursor-codec --test byte_for_byte_preserve --features cursor-mitm-experimental` exits 0 |
| `A4.5` | Egress-proxy hook test green | `cargo test -p spendguard-egress-proxy --test cursor_mitm --features cursor-mitm-experimental` exits 0 |
| `A4.6` | Version-gate accept/reject tests green | `cargo test -p spendguard-cursor-codec --test version_gate --features cursor-mitm-experimental` exits 0 |

## 5. Experimental-marker / warning gates

These gates enforce the "loud experimental" requirement from `design.md` §6. Reviewer rejects any slice that lands without at least the three loud markers wired.

| ID | Gate | Verification command |
|----|------|----------------------|
| `A5.1` | Cargo metadata carries `[package.metadata.experimental]` | (same as `A1.2`) |
| `A5.2` | Codec process emits stderr banner on first request | `cargo test -p spendguard-egress-proxy stderr_banner_emitted_once_per_process --features cursor-mitm-experimental` exits 0 |
| `A5.3` | `spendguard install --include cursor` (when feature on) prints the experimental banner before any cert work | `cargo run -p spendguard-cli --features cursor-mitm-experimental -- install --include cursor --dry-run 2>&1 \| head -3 \| grep -q '\[EXPERIMENTAL\] cursor-mitm'` |
| `A5.4` | SOW doc warning text present | `grep -qF 'DO NOT SHIP AS A GA FEATURE' docs/customer/sow-cursor-mitm.md` |
| `A5.5` | No "Cursor" entry in any GA-tier docs landing page | `! grep -Rl 'Cursor.*GA\|Cursor.*generally available' docs/site-v2/src/content/docs/integrations/` |

## 6. Demo-mode gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A6.1` | Demo target exists in `Makefile` | `grep -q '^demo-cursor-mitm-fixture' Makefile` |
| `A6.2` | Demo runs against the streaming fixture and exits 0 | `make demo-cursor-mitm-fixture` exits 0 |
| `A6.3` | Demo emits experimental banner to stderr | `make demo-cursor-mitm-fixture 2>&1 1>/dev/null \| grep -q '\[EXPERIMENTAL\]'` |
| `A6.4` | Demo produces reserve + commit in stdout audit chain | `make demo-cursor-mitm-fixture 2>/dev/null \| grep -cE '^(reserve\|commit) ' \| grep -q '^2$'` |
| `A6.5` | Demo wired into demo-mode regression suite | `cargo test -p spendguard-egress-proxy --test demo_modes cursor_mitm_fixture --features cursor-mitm-experimental` exits 0 |

## 7. Negative gates (must NOT compile / must NOT exist)

| ID | Gate | Verification command |
|----|------|----------------------|
| `A7.1` | No vendor `.proto` committed verbatim | `! grep -RIl '^//.*Cursor\..*All rights reserved' services/cursor_codec/proto/` |
| `A7.2` | No live `api.cursor.sh` calls in CI | `! grep -RIl '"https://api.cursor.sh\|http_client.*cursor\.sh' services/ \| grep -v '/fixtures/\|/tests/\|PROTOCOL.md\|sites.toml'` |
| `A7.3` | No GA Cargo feature alias for the codec | `! cargo metadata --format-version 1 \| jq -e '.packages[].features \| keys[] \| select(. == "cursor-mitm" or . == "cursor")'` |
| `A7.4` | Default `sites.toml` does NOT include `api.cursor.sh` | `! grep -q 'api\.cursor\.sh' services/egress_proxy/config/sites.toml` |

## 8. Slice ↔ gate matrix

| Slice | Gates that must pass to merge the slice |
|-------|-----------------------------------------|
| `COV_S17_01_d17_recon` | `A1.3`, `A1.4`, `A1.5`, `A7.1` |
| `COV_S17_02_d17_framing_parser` | `A3.2`, plus `A2.4` |
| `COV_S17_03_d17_envelope_extract` | `A3.3`, `A1.4`, `A7.1` |
| `COV_S17_04_d17_codec_skeleton` | `A1.1`, `A1.2`, `A2.1`, `A2.2`, `A2.6`, `A5.1`, `A7.3` |
| `COV_S17_05_d17_translator` | `A3.4` |
| `COV_S17_06_d17_reserve_commit` | `A4.1`, `A4.2`, `A4.3` |
| `COV_S17_07_d17_response_reencode` | `A4.4`, `A4.5` |
| `COV_S17_08_d17_fixtures_and_tests` | `A1.5`, `A4.1`-`A4.6`, `A5.2` |
| `COV_S17_09_d17_docs_sow_demo` | `A1.6`-`A1.10`, `A5.3`-`A5.5`, `A6.1`-`A6.5`, `A7.2`, `A7.4` |

## 9. Definition of done

D17 is "shipped" when:

- All 9 slices merged into main.
- All `A1.x` – `A7.x` gates run green on `cargo build --workspace --features cursor-mitm-experimental --locked` and on the default-build counterparts.
- `docs/customer/sow-cursor-mitm.md` reviewed by product/legal (manual gate, recorded as PR comment).
- An entry has been added to `README.md` `## 🔌 Adapter integrations` table marked **`Experimental — SOW only`** (NOT "GA").
- A memory entry `project_coverage_D17_shipped.md` is written per build-plan §8.
