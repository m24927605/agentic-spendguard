# COV_05 — D02 Closed CLI install: CA root + leaf issuance

> **Deliverable**: D02 Closed CLI install script + CA bootstrap
> **Slice**: 1 of 8 (M)
> **Spec set**: [`docs/specs/coverage/D02_closed_cli_install/`](../../specs/coverage/D02_closed_cli_install/)

## Scope

Lay down the new `services/cli/` Rust workspace crate and ship the CA-generation + leaf-cert issuance core (the substrate every subsequent slice builds on). Trust-store install per-OS is OUT of this slice — SLICE 2/3/4 cover macOS / Linux / Windows respectively.

Concretely:
- `services/cli/Cargo.toml` — bin + lib, deps on `rcgen`, `time`, `uuid`, `clap` (CLI scaffolding), `serde`, `serde_json`, `tracing`, `anyhow`.
- `services/cli/src/main.rs` — `clap`-driven CLI with two top-level subcommands stubbed: `install` (this slice fills the CA gen path; trust-store install is TODO sniped to SLICE 2-4) and `uninstall` (full impl deferred to SLICE 7).
- `services/cli/src/lib.rs` — public lib surface: `install(opts)`, `uninstall(opts)`, `doctor()` traits + `InstallReport` / `UninstallReport` / `DoctorReport` types (per design §8).
- `services/cli/src/ca.rs` — CA gen module:
  - `generate_root_ca()` — rcgen-backed, CN=`SpendGuard Local Root CA`, validity 825 days (locked §9.1), serial = UUIDv7 → BigUint (locked §9.2), CertSign + CRLSign KeyUsage.
  - `issue_leaf_cert(root_ca, &["localhost", "127.0.0.1", "::1"])` — SAN constrained to localhost only (per design §3 closed-loop locality).
  - Both functions return `(cert_pem, key_pem)`. PEM-only, no PKCS#12 in this slice.
- `services/cli/src/paths.rs` — XDG-friendly path resolver: `ca_root_dir()` returns `~/.local/share/spendguard/ca/` on Linux, `~/Library/Application Support/SpendGuard/ca/` on macOS, `%APPDATA%\SpendGuard\ca\` on Windows. Idempotent `mkdir -p`.
- `Cargo.toml` workspace exclude entry for the new service.

This slice produces a CLI that:
- `spendguard install --ca-out /tmp/test-ca/` writes `root_ca.pem` + `root_ca.key.pem` + `leaf.pem` + `leaf.key.pem` to the path, prints the SHA-256 fingerprint of the root CA, then exits clean.
- The CA + leaf are valid: `openssl verify -CAfile root_ca.pem leaf.pem` succeeds.

## Files touched

| File | Why |
|------|-----|
| `services/cli/Cargo.toml` | New service manifest |
| `services/cli/src/main.rs` | clap CLI |
| `services/cli/src/lib.rs` | Public lib surface |
| `services/cli/src/ca.rs` | CA + leaf gen |
| `services/cli/src/paths.rs` | XDG path resolver |
| `Cargo.toml` (workspace) | exclude entry for `services/cli` |

## Test/verification plan

1. `cargo build --manifest-path services/cli/Cargo.toml` succeeds.
2. `cargo test --manifest-path services/cli/Cargo.toml ca::tests` passes — at minimum:
   - `generates_root_ca_with_uuidv7_serial`
   - `root_ca_validity_is_825_days`
   - `leaf_san_only_localhost`
   - `leaf_verifies_against_root` (uses openssl-like verifier via rcgen or rustls)
3. `cargo build --workspace` doesn't regress (no other crate breaks).
4. Smoke: `cargo run -p spendguard-cli -- install --ca-out /tmp/sgcli-test/` produces 4 PEM files; `openssl verify` (if installed) confirms validity.
5. `cargo fmt --check` clean.

## Anti-scope

- No OS trust-store mutation — SLICE 2/3/4.
- No shell rc emission — SLICE 5.
- No Gemini OAuth refusal — SLICE 6.
- No `doctor` impl beyond signature — SLICE 7.
- No per-CLI smoke — SLICE 8.

## Backlinks

- Spec set: [`design.md`](../../specs/coverage/D02_closed_cli_install/design.md) §7 slice 1 row, §9 locked decisions
- Build plan: [`framework-coverage-build-plan-2026-06.md`](../../strategy/framework-coverage-build-plan-2026-06.md) §1.5
- Review standards: [`review-standards.md`](../../specs/coverage/D02_closed_cli_install/review-standards.md)
