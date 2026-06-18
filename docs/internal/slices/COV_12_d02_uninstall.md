# COV_12 — D02 Closed CLI install: symmetric uninstall (D02 closes)

> **Deliverable**: D02 Closed CLI install script + CA bootstrap
> **Slice**: 8 of 8 (M) — FINAL D02 slice
> **Spec set**: [`docs/specs/coverage/D02_closed_cli_install/`](../../specs/coverage/D02_closed_cli_install/)

## Scope

Land `spendguard uninstall` — symmetric inverse of `install`. Per design §3 line 32: `uninstall: tools::clear_overrides → shell::strip_rc → trust::uninstall`. After this slice, D02 deliverable is COMPLETE.

Most uninstall infrastructure already shipped:
- SLICE 5 (shell rc): `strip_rc` + `uninstall_with_backends` seam already exists
- SLICE 6 (preflight): uninstall correctly bypasses preflight (safety)
- SLICE 7 (doctor): `default_ca_pem_path()` already pub for uninstall reuse

SLICE 8 closes the loop: clean integration tests + CLI surface + `doctor::is_healthy()` post-uninstall regression.

Concretely:
- `services/cli/src/lib.rs` — verify uninstall_with_backends end-to-end:
  - Strips shell rc marker block (SLICE 5 wired)
  - Removes per-tool env vars (SLICE 5 tools::env_vars_for_install symmetric)
  - Removes CA from trust store (trust::dispatch().remove_root via SLICE 2-4)
  - Deletes CA PEM file from XDG dir
  - Does NOT touch oauth_creds.json or any user data outside SpendGuard's own files
- `services/cli/src/cli.rs` — `uninstall` clap subcommand with `--keep-shell-rc` + `--keep-ca-files` opt-out flags
- `services/cli/tests/uninstall_smoke.rs` — NEW integration tests:
  - Round-trip: install + uninstall + doctor::is_healthy() returns Healthy (all checks flip to Absent/NotInstalled)
  - Partial install + uninstall: cleans whatever's there
  - --keep-shell-rc: rc untouched after uninstall

## Files touched

| File | Why |
|------|-----|
| `services/cli/src/lib.rs` | verify/complete uninstall_with_backends |
| `services/cli/src/cli.rs` | uninstall subcommand + opt-out flags |
| `services/cli/src/main.rs` | uninstall arm wire |
| `services/cli/tests/uninstall_smoke.rs` | NEW round-trip integration tests |

## Test/verification plan

1. `cargo build` clean
2. `cargo test --lib` — 171 SLICE 7 baseline + ~10 new = 181+ passing
3. cargo fmt + clippy clean
4. CLI: `spendguard uninstall --help` shows opt-out flags
5. Smoke: install + uninstall + doctor → all checks Healthy

## Anti-scope

- No prompts/confirmations (uninstall is destructive but expected)
- No partial-uninstall recovery from broken state

## Backlinks

- Spec set: [`design.md`](../../specs/coverage/D02_closed_cli_install/design.md) §3 line 32 (uninstall flow), §7 slice 8 row
- SLICE 7: [`COV_11_d02_doctor.md`](COV_11_d02_doctor.md)
- D02 deliverable closes at this slice
