# COV_08 — D02 Closed CLI install: Windows trust

> **Deliverable**: D02 Closed CLI install script + CA bootstrap
> **Slice**: 4 of 8 (S)
> **Spec set**: [`docs/specs/coverage/D02_closed_cli_install/`](../../specs/coverage/D02_closed_cli_install/)

## Scope

Add Windows trust-store impl alongside SLICE 2's macOS keychain + SLICE 3's Linux multi-distro support. Follows the LOCKED-at-SLICE-3-R2 module-declaration pattern: `pub mod windows;` ungated (FakeRunner unit tests run on macOS dev host), but MUST avoid `windows`/`windows-sys` crate types in trait-bound public signatures — inject via a `WinCertStore` trait inside the module instead, mirroring how `linux.rs` keeps `CommandRunner` at the boundary.

Concretely:
- `services/cli/src/trust/windows.rs` — NEW:
  - `pub struct WindowsTrustStore { runner: Arc<dyn CommandRunner> }` — same CommandRunner injection pattern as Linux/macOS
  - `add_root(...)`: dispatch on TrustScope:
    - User scope → `certutil -user -addstore -f Root <pem_path>`
    - System scope → `certutil -addstore -f Root <pem_path>` (requires elevated shell)
  - `remove_root(...)`: symmetric `certutil -delstore Root <fingerprint>` with `-user` when scope=User
  - `verify_installed(...)`: `certutil -store Root` + fingerprint match (or `certutil -verifystore`)
  - All shell-outs route through CommandRunner with positional `args[]` so T8 (no shell injection) holds
- `services/cli/src/trust/mod.rs` — extend `dispatch()` with `#[cfg(target_os = "windows")]` arm → `WindowsTrustStore::new()`
- `pub mod windows;` declared on every host per the LOCKED SLICE 3 R2 module-decl pattern
- Windows unit tests (≥10) covering each TrustScope's argv shape via FakeRunner — OS-independent, run on macOS dev host
- Integration tests in `services/cli/tests/trust_windows.rs`:
  - `#![cfg(target_os = "windows")]`-gated
  - 2 always-on smoke tests (constructor + happy-path argv shape)
  - 3 `#[ignore]`-gated mutating tests (User scope add/remove/verify cycle; System scope; fresh-host bogus fingerprint cleanup with RAII guard)

## Files touched

| File | Why |
|------|-----|
| `services/cli/src/trust/windows.rs` | Windows certutil impl |
| `services/cli/src/trust/mod.rs` | dispatch() Windows arm |
| `services/cli/tests/trust_windows.rs` | Integration tests (#[ignore]-gated for CI matrix) |

## Test/verification plan

1. `cargo build --target x86_64-pc-windows-gnu --manifest-path services/cli/Cargo.toml` clean (cross-build via zigbuild — same pattern SLICE 3 R1 deviation #1 established for Linux).
2. `cargo test --manifest-path services/cli/Cargo.toml --lib` — 53 SLICE 3 baseline + ~12 new Windows unit tests = 65+ passing on macOS host.
3. Linux + macOS regression: all SLICE 2 + SLICE 3 tests intact.
4. `cargo clippy --target x86_64-pc-windows-gnu -D warnings` clean (cross-target).
5. `cargo fmt --check` clean.

## Anti-scope

- No shell rc emission — SLICE 5
- No Gemini OAuth refusal — SLICE 6
- No `doctor` impl beyond signature — SLICE 7
- No `windows-rs` crate dep — inject via WinCertStore trait + CommandRunner

## Backlinks

- Spec set: [`design.md`](../../specs/coverage/D02_closed_cli_install/design.md) §4 (Windows backend table line 45), §7 slice 4 row, §4 module-decl LOCKED note
- SLICE 2: [`COV_06_d02_trust_macos.md`](COV_06_d02_trust_macos.md)
- SLICE 3: [`COV_07_d02_trust_linux.md`](COV_07_d02_trust_linux.md)
