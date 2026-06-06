# COV_06 — D02 Closed CLI install: macOS keychain install/uninstall

> **Deliverable**: D02 Closed CLI install script + CA bootstrap
> **Slice**: 2 of 8 (S)
> **Spec set**: [`docs/specs/coverage/D02_closed_cli_install/`](../specs/coverage/D02_closed_cli_install/)

## Scope

Wire the macOS keychain trust path on top of SLICE 1's CA gen. Add `services/cli/src/trust/macos.rs` module + integrate into `install()` / `uninstall()` / `doctor()` library functions. Tested via `security` CLI shellouts (no third-party crate; matches existing repo pattern).

Concretely:
- `services/cli/src/trust/mod.rs` — trait `TrustStore` with `add_root`, `remove_root`, `verify_installed`.
- `services/cli/src/trust/macos.rs`:
  - `MacosTrustStore` impl
  - `add_root(ca_pem_path, scope: TrustScope)` — invokes `security add-trusted-cert -d -r trustRoot -k <keychain> <ca_pem>` where `<keychain>` is `login.keychain-db` for user scope and `System.keychain` for system scope (requires sudo)
  - `remove_root(fingerprint_sha256_hex)` — invokes `security delete-certificate -Z <sha1>` (note: macOS `security` historically uses SHA-1; spec out the SHA-256 → SHA-1 derivation OR pass `-c` by Common Name as fallback)
  - `verify_installed(fingerprint_sha256_hex)` — invokes `security find-certificate -a -Z` and grep for fingerprint
- `services/cli/src/lib.rs` — `install()` calls `trust::macos::MacosTrustStore::add_root()` after writing PEMs; populates `InstallReport.trust_store_locations` with the keychain path.
- `services/cli/src/lib.rs` — `uninstall()` symmetrically removes via fingerprint.
- `services/cli/src/lib.rs` — `doctor()` reports whether CA is trusted in the configured keychain.

## Files touched

| File | Why |
|------|-----|
| `services/cli/src/trust/mod.rs` | TrustStore trait |
| `services/cli/src/trust/macos.rs` | macOS impl via `security` shellout |
| `services/cli/src/lib.rs` | Wire install/uninstall/doctor |

## Test/verification plan

1. `cargo build` clean.
2. `cargo test` — new tests:
   - `macos::tests::add_root_invokes_security_add_trusted_cert` (mocks `security` via test harness or `which` PATH redirect)
   - `macos::tests::verify_installed_returns_true_after_add`
   - `macos::tests::remove_root_removes_via_fingerprint`
3. On macOS host only (`#[cfg(target_os = "macos")]`): integration test that actually adds + removes + verifies the cert in `~/Library/Keychains/login.keychain-db` (skipped on Linux CI).
4. `cargo run -p spendguard-cli -- install` followed by `spendguard doctor` shows the CA as trusted; followed by `spendguard uninstall` shows it removed.
5. `cargo fmt --check` clean.

## Anti-scope

- No Linux trust — SLICE 3.
- No Windows trust — SLICE 4.
- No shell rc emission — SLICE 5.
- No Gemini OAuth refusal — SLICE 6.

## Backlinks

- Spec set: [`design.md`](../specs/coverage/D02_closed_cli_install/design.md) §7 (slice 2 row)
- SLICE 1 backbone: [`COV_05_d02_ca_root_and_leaf.md`](COV_05_d02_ca_root_and_leaf.md)
