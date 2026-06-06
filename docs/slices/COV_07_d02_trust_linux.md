# COV_07 — D02 Closed CLI install: Linux multi-distro trust

> **Deliverable**: D02 Closed CLI install script + CA bootstrap
> **Slice**: 3 of 8 (M)
> **Spec set**: [`docs/specs/coverage/D02_closed_cli_install/`](../specs/coverage/D02_closed_cli_install/)

## Scope

Add Linux trust-store impl alongside SLICE 2's macOS keychain. Dispatch on `/etc/os-release` ID + ID_LIKE to pick the right OS-trust tool: Debian/Ubuntu → `update-ca-certificates` + `/usr/local/share/ca-certificates/`, RHEL/Fedora → `update-ca-trust` + `/etc/pki/ca-trust/source/anchors/`, Arch → `trust anchor`, Alpine → similar to Debian via `ca-certificates`.

Concretely:
- `services/cli/src/trust/linux.rs` — NEW:
  - `pub struct LinuxTrustStore { runner: Arc<dyn CommandRunner>, distro: LinuxDistro }`
  - `pub enum LinuxDistro { Debian, Rhel, Arch, Alpine, Unknown }`
  - `LinuxTrustStore::detect_distro()` — reads `/etc/os-release` ID + ID_LIKE
  - `add_root`: distro-dispatch to right tool + cert path
  - `remove_root`: symmetric removal
  - `verify_installed`: distro-dispatch to verify cert in store
- `services/cli/src/trust/mod.rs` — extend `dispatch()` with `#[cfg(target_os = "linux")]` arm → `LinuxTrustStore::new()`
- Linux tests: 13+ unit tests covering each distro's argv shape via FakeRunner
- Linux integration tests: `#[ignore]`-gated (CI matrix runs them per design §4.5)
  - Debian: `update-ca-certificates --verbose` + `/usr/local/share/ca-certificates/spendguard.crt` + verify via `awk '/-----BEGIN CERTIFICATE-----/' /etc/ssl/certs/ca-certificates.crt`
  - RHEL: `update-ca-trust extract` + `/etc/pki/ca-trust/source/anchors/spendguard.crt`
  - Arch: `trust anchor --store /tmp/sgcli-test/root_ca.pem`
  - Alpine: same as Debian

## Files touched

| File | Why |
|------|-----|
| `services/cli/src/trust/linux.rs` | Linux multi-distro impl |
| `services/cli/src/trust/mod.rs` | dispatch() Linux arm |
| `services/cli/tests/trust_linux.rs` | Integration tests (#[ignore]-gated) |

## Test/verification plan

1. `cargo build --target x86_64-unknown-linux-gnu --manifest-path services/cli/Cargo.toml` clean (cross-build).
2. `cargo test --manifest-path services/cli/Cargo.toml` — new unit tests pass (FakeRunner-driven, OS-independent).
3. `cargo test -- --include-ignored` on each Linux distro CI matrix (deferred for human run).
4. macOS regression: SLICE 2's 26 + 7 tests still pass.
5. `cargo clippy --target x86_64-unknown-linux-gnu -D warnings` clean.

## Anti-scope

- No Windows — SLICE 4.
- No shell rc emission — SLICE 5.
- No Gemini OAuth refusal — SLICE 6.
- No `doctor` impl beyond signature — SLICE 7.

## Backlinks

- Spec set: [`design.md`](../specs/coverage/D02_closed_cli_install/design.md) §4 (Linux backends), §7 slice 3 row
- SLICE 2: [`COV_06_d02_trust_macos.md`](COV_06_d02_trust_macos.md)
