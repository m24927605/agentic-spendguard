# COV_11 — D02 Closed CLI install: doctor

> **Deliverable**: D02 Closed CLI install script + CA bootstrap
> **Slice**: 7 of 8 (M)
> **Spec set**: [`docs/specs/coverage/D02_closed_cli_install/`](../../specs/coverage/D02_closed_cli_install/)

## Scope

Land `spendguard doctor` — runtime diagnostic per design §3 line 34: CA fingerprint in store + HTTPS_PROXY reachable + TLS handshake round-trip. Also surfaces the Linux user-scope CURL_CA_BUNDLE / SSL_CERT_FILE redirect per the SLICE 3 R2 clarification, and detects Gemini OAuth state from SLICE 6.

Concretely:
- `services/cli/src/doctor/mod.rs` — NEW DoctorReport struct + run() entry:
  - `pub fn run(env: &BaseEnv, opts: DoctorOpts) -> DoctorReport`
  - Aggregates checks; never crashes; degrades gracefully
- `services/cli/src/doctor/ca_fingerprint.rs` — NEW:
  - Reads $XDG_DATA_HOME/spendguard/root_ca.pem
  - Computes fingerprint
  - Cross-checks against installed trust store (dispatch to trust::macos/linux/windows::verify_installed)
- `services/cli/src/doctor/proxy_reachable.rs` — NEW:
  - TCP connect to localhost:8443 (or proxy from config)
  - Optional TLS handshake against the CA
  - Times out cleanly (5s default)
- `services/cli/src/doctor/shell_rc.rs` — NEW:
  - Inspects user's shell rc for the SpendGuard marker block (from SLICE 5)
  - Reports presence + content (does NOT mutate)
- `services/cli/src/doctor/linux_user_scope.rs` — NEW:
  - If Linux user-scope was attempted (SLICE 3 R2 fail-closed message): surface the CURL_CA_BUNDLE / SSL_CERT_FILE redirect with command examples
- `services/cli/src/doctor/gemini_check.rs` — NEW:
  - Reads gemini::detect() (preflight from SLICE 6)
  - Reports state (NotInstalled / ApiKeyMode / ServiceAccountMode / OauthFreetierRefused)
- `services/cli/src/main.rs` — wire `doctor` subcommand
- Tests (≥15 unit + 3 lib-level integration):
  - DoctorReport struct field assertions
  - Each check module independently mockable via BaseEnv + CommandRunner injection
  - Color/no-color rendering (terminal-color-friendly check report)

## Files touched

| File | Why |
|------|-----|
| `services/cli/src/doctor/mod.rs` | NEW DoctorReport + run() |
| `services/cli/src/doctor/ca_fingerprint.rs` | CA check |
| `services/cli/src/doctor/proxy_reachable.rs` | TCP+TLS check |
| `services/cli/src/doctor/shell_rc.rs` | shell rc inspection |
| `services/cli/src/doctor/linux_user_scope.rs` | Linux user-scope redirect surface |
| `services/cli/src/doctor/gemini_check.rs` | Gemini state surface |
| `services/cli/src/main.rs` | doctor subcommand wiring |
| `services/cli/src/cli.rs` (or where args live) | `doctor` clap subcommand |

## Test/verification plan

1. `cargo build` clean
2. `cargo test --lib` — 132 SLICE 6 baseline + ~18 new = 150+ passing
3. `cargo fmt --check` + `cargo clippy -D warnings` clean
4. CLI: `spendguard doctor` runs end-to-end without panicking on fresh install OR no-install state
5. macOS regression: trust + shell_integration tests unchanged

## Anti-scope

- No symmetric uninstall workflow beyond signature — SLICE 8
- No mutating recovery actions (doctor is read-only)
- No remote-host reachability checks (localhost-only)

## Backlinks

- Spec set: [`design.md`](../../specs/coverage/D02_closed_cli_install/design.md) §3 line 34 (doctor flow), §3 line 47 (SLICE 7 doctor surfaces CURL_CA_BUNDLE redirect), §7 slice 7 row
- SLICE 6: [`COV_10_d02_gemini_oauth_refusal.md`](COV_10_d02_gemini_oauth_refusal.md)
- SLICE 3 R2 Linux user-scope clarification
