# D17 — Review Standards

Slice-specific checklist for the `superpowers:code-reviewer` skill across `COV_S17_01` … `COV_S17_09`. Each slice review consults this file plus [`acceptance.md`](acceptance.md) plus the repo-wide coding standards.

> **R5 panel summarizer override: Security Engineer.** Per `design.md` §8 decision 5: reverse-engineered codec + MITM of a closed binary is the dominant risk surface. On R5 arbitration the Security Engineer's memo is the tiebreaker, NOT the build-plan default Software Architect.

## 1. Experimental-marker assertions (loud, repeated, three places)

Any diff touching `services/cursor_codec/**`, `services/egress_proxy/src/routing.rs`, or `services/cli/src/overrides.rs` MUST satisfy every assertion below. Reviewer flags any failure as a **Blocker**.

| ID | Assertion |
|----|-----------|
| `E1` | `services/cursor_codec/Cargo.toml` has `[package.metadata.experimental]` with `reason = "Reverse-engineered Cursor wire protocol. Breaks on vendor release."` exactly. Reviewer greps for the literal `reason` string. |
| `E2` | Every public entry point in `spendguard-cursor-codec` calls `assert_experimental_banner_emitted()` on first use per process. Reviewer greps `lib.rs` + `pipeline.rs` for the call. |
| `E3` | `spendguard install --include cursor` MUST print the experimental banner to stderr BEFORE any cert work. Reviewer rejects diffs where banner emits after `ca::ensure_root` or `trust::install`. |
| `E4` | `docs/customer/sow-cursor-mitm.md` carries `Status: EXPERIMENTAL — SOW only` above the fold AND `DO NOT SHIP AS A GA FEATURE` as a top-level callout. |
| `E5` | `docs/customer/sow-cursor-mitm.md` front-matter sets `noindex: true`. SOW addendum content must not be publicly crawlable. |
| `E6` | No GA-tier documentation page lists Cursor as a generally available integration. README table row is marked `Experimental — SOW only`. |
| `E7` | Feature flag name is exactly `cursor-mitm-experimental`. Aliases such as `cursor`, `cursor-mitm`, or `cursor-codec` are rejected — the suffix `-experimental` must be present so anyone enabling the feature reads "experimental" inline. |

## 2. Reverse-engineering / IP assertions

The codec ships SpendGuard's own description of an observed wire format. It MUST NOT ship vendor source. Any diff touching `services/cursor_codec/proto/**` or `PROTOCOL.md`:

| ID | Assertion |
|----|-----------|
| `R1` | `proto/cursor_observed.proto` starts with a SpendGuard attribution header: `// SpendGuard-authored observation of the Cursor api.cursor.sh wire protocol. Reconstructed from black-box capture. NOT vendor source.` |
| `R2` | `PROTOCOL.md` documents capture date, Cursor client version range, and field-by-field observation evidence (hex excerpts). Reviewer rejects "TODO" placeholders here. |
| `R3` | No file under `services/cursor_codec/` contains a Cursor copyright header or vendor-attributed license text. Reviewer rejects diffs that paste vendor `.proto` content verbatim. |
| `R4` | Fixture `.cursor-rpc` files carry tagged version range in their filename or sidecar manifest. A fixture without a version tag is rejected. |
| `R5` | Capture script under `tools/capture/` is documented but NOT wired into CI. Reviewer rejects diffs that add a CI job calling `tools/capture/`. |

## 3. Feature-flag isolation assertions

The codec MUST be invisible to default builds.

| ID | Assertion |
|----|-----------|
| `F1` | `services/egress_proxy/Cargo.toml` lists `spendguard-cursor-codec` as `optional = true` AND as a dep of feature `cursor-mitm-experimental` only. |
| `F2` | `services/egress_proxy/src/routing.rs` cursor arm is gated by `#[cfg(feature = "cursor-mitm-experimental")]`. Reviewer rejects runtime-flag-driven dispatch (`if cfg.cursor_enabled { ... }`) because it would compile the codec into default builds. |
| `F3` | `services/cli/src/overrides.rs` cursor handler is `cfg`-gated; default build emits `error: feature 'cursor-mitm-experimental' required` at runtime if `--include cursor` is passed. |
| `F4` | Default `services/egress_proxy/config/sites.toml` does NOT list `api.cursor.sh`. SOW customer adds it post-install. |
| `F5` | Default `cargo build --workspace --locked` MUST succeed without the codec crate compiling. Reviewer runs `cargo tree -p spendguard-egress-proxy` and confirms `spendguard-cursor-codec` is absent in the default tree. |

## 4. Wire-protocol correctness assertions

Slices `COV_S17_02` / `COV_S17_03` / `COV_S17_05` / `COV_S17_07` touch wire bytes. Any diff:

| ID | Assertion |
|----|-----------|
| `W1` | Connect-RPC framing is parsed as 5-byte prefix: `flags:u8 + length:u32 big-endian + payload`. Reviewer rejects any other layout. |
| `W2` | `flags & 0x01` (compressed) is acted on, NOT silently ignored. Unknown compression algorithms short-circuit to `CodecError::UnsupportedCompression`. |
| `W3` | `flags & 0x02` (trailing metadata) is honoured on server-streaming responses; terminal chunk MUST be detected. |
| `W4` | `prost`-derived structs include `unknown_fields: prost_types::UnknownFields` for vendor field additions to survive. Reviewer rejects bare structs without an unknown-fields bag. |
| `W5` | Byte-for-byte preservation: response re-encoding round-trips identical bytes for any field SpendGuard did not modify. Reviewer requires `tests/byte_for_byte_preserve.rs` to cover all three fixtures. |
| `W6` | Unknown Cursor model strings fall through to release-and-pass-through (best-effort gating per `design.md` §2 disclaimer), NOT fail-closed. Reviewer rejects code that 5xx's on unknown model. |
| `W7` | Version gate enforced BEFORE decode. Cursor UA outside the tested range returns 503 with `X-SpendGuard-Codec-Break` header; the gate MUST NOT optimistically attempt decode. |

## 5. Pipeline / ledger correctness assertions

Slice `COV_S17_06`:

| ID | Assertion |
|----|-----------|
| `P1` | Pipeline calls `ledger.reserve` BEFORE forwarding to `api.cursor.sh`. Reviewer rejects diffs that reserve after upstream response. |
| `P2` | On terminal chunk OR unary response, pipeline calls `ledger.commit` with actual token count derived from the canonical-translated response. |
| `P3` | On upstream truncation / error / version-out-of-range, pipeline calls `ledger.release` with a reason code matching `services/ledger`'s enum. No "leaked" reservations. |
| `P4` | `fixture_replay_mode = true` short-circuits the upstream dial and reads response bytes from the fixture. Production mode (`false`) MUST dial `api.cursor.sh`. Reviewer rejects diffs that conflate the two. |
| `P5` | No `api.cursor.sh` URL is hardcoded outside `sites.toml`. Reviewer greps `services/` for `cursor\.sh` and expects matches only in `sites.toml`, `PROTOCOL.md`, fixture filenames, and test code. |

## 6. CI / test-isolation assertions

| ID | Assertion |
|----|-----------|
| `C1` | No CI job hits live `api.cursor.sh`. Reviewer greps `.github/workflows/**` and `Makefile` for `cursor\.sh` and expects matches only in `--demo cursor_mitm_fixture` lines, which read from local fixtures. |
| `C2` | The negative-no-feature CI job (acceptance `A2.6`, `A7.3`) is present and green. |
| `C3` | Demo target `demo-cursor-mitm-fixture` exists in `Makefile` and runs in the demo-mode regression suite. |
| `C4` | Fixtures are < 64 KiB each. Reviewer rejects bloated captures — large captures suggest unnecessary session state leaked into the file. |
| `C5` | Stderr-banner-once test (`stderr_banner_emitted_once_per_process`) is present and green. Reviewer rejects diffs that spam the banner per-request. |

## 7. SOW addendum assertions

Slice `COV_S17_09` lands `docs/customer/sow-cursor-mitm.md`. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `S1` | Doc has `Status: EXPERIMENTAL — SOW only` and `noindex: true` in front-matter. |
| `S2` | Doc has `## Break-Window SLA` section with placeholder fields for: customer name, SOW number, codec-break detection window, codec-fix turnaround target, customer escalation contact. |
| `S3` | Doc carries the literal warning `DO NOT SHIP AS A GA FEATURE` as a top-level callout. |
| `S4` | Doc explains the customer maintenance burden: vendor protocol updates can break the codec at any time; customer accepts this risk by signing. |
| `S5` | Doc explains the legal posture: this is reverse-engineered interop, not vendor-endorsed. Customer is responsible for confirming their Cursor terms permit MITM. |
| `S6` | Doc is generated from a template (`docs/customer/sow-cursor-mitm.md.tmpl`); the template path is referenced inline so future updates flow through one source. |

## 8. R5 panel arbitration material (per `staff-panel-arbitration-process.md` §2)

If a slice exhausts R5, the arbitration package MUST include:

1. The slice's `acceptance.md` gate matrix with pass/fail per gate.
2. Every reviewer finding from R1-R5 with the implementer's response.
3. A pointer to `design.md` §1 (loud warning) and `design.md` §6 (experimental marker locations).
4. A pointer to `services/cursor_codec/PROTOCOL.md` so the Security Engineer summarizer can audit the reverse-engineering posture.
5. The fixture corpus manifest with version tags so the AI Engineer can confirm the codec's tested window.

Default summarizer override per `design.md` §8 decision 5: **Security Engineer** writes the final ruling.

## 9. Standing "do not" list

Reviewer rejects any diff that does any of the following, regardless of whether other gates pass:

- Promotes the codec to default features.
- Removes any of the three loud markers (Cargo metadata, stderr banner, doc warning).
- Adds Cursor to a GA-tier integrations page.
- Commits vendor `.proto` content verbatim.
- Wires the capture script into CI.
- Adds live `api.cursor.sh` traffic to any test path.
- Implements fail-closed gating on unknown Cursor models or out-of-range client versions in a way that breaks customer Cursor sessions silently — the codec is best-effort gating per `design.md` §2; failures MUST be loud (503 + header) or release-and-pass-through.
