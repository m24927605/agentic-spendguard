# COV_09 — D02 Closed CLI install: shell rc emission + per-tool overrides

> **Deliverable**: D02 Closed CLI install script + CA bootstrap
> **Slice**: 5 of 8 (M)
> **Spec set**: [`docs/specs/coverage/D02_closed_cli_install/`](../../specs/coverage/D02_closed_cli_install/)

## Scope

Land the env-var emitter + shell rc writer that bridges CA trust (SLICE 2-4) to BYOK tools. For ~14 closed-binary CLIs that don't honor OpenAI-compatible `*_BASE_URL`, the bridge is "trust the CA + set `HTTPS_PROXY` + per-tool env overrides" (e.g., `SRC_HTTPS_PROXY` for Cody, `CLAUDE_PROXY` for Claude Code, etc.).

Concretely:
- `services/cli/src/shell/mod.rs` — NEW shell rc backend dispatch:
  - `ShellKind` enum: Bash / Zsh / Fish / Pwsh / Cmd
  - `Shell::detect()` reads `$SHELL` (or PowerShell ENV on Windows)
  - `write_rc(rc_path, vars)` writes a marked block (`# >>> spendguard >>>` / `# <<< spendguard <<<`) so uninstall can locate + strip
- `services/cli/src/shell/posix.rs` — NEW Bash/Zsh/Fish:
  - bash → `~/.bashrc`
  - zsh → `~/.zshrc`
  - fish → `~/.config/fish/conf.d/spendguard.fish`
  - Export `HTTPS_PROXY=https://localhost:8443` + per-tool overrides
- `services/cli/src/shell/windows.rs` — NEW PowerShell + cmd:
  - pwsh → `$PROFILE` add-content with begin/end markers
  - cmd → `AutoRun` registry hint (NOT actually mutated; recorded in install report; operator runs `setx` themselves)
- `services/cli/src/tools/mod.rs` — NEW per-tool override table:
  - 14 tools mapped to their env-var names (full table in design.md §4)
  - Claude Code: `CLAUDE_PROXY`
  - Codex: `OPENAI_PROXY`
  - Cody self-hosted: `SRC_HTTPS_PROXY`
  - Goose: none (HTTPS_PROXY only)
  - etc.
- `services/cli/src/lib.rs` — extend `install()` to call shell::write_rc + tools::write_overrides AFTER trust::install succeeds; extend `uninstall()` to call shell::strip_rc + tools::clear_overrides BEFORE trust::uninstall
- 12+ FakeRunner unit tests covering each shell's rc output shape + marker idempotency (re-running install doesn't duplicate the block)
- 4 integration tests #[ignore]-gated: bash + zsh + pwsh + fish smoke

## Files touched

| File | Why |
|------|-----|
| `services/cli/src/shell/mod.rs` | NEW — dispatch + ShellKind enum |
| `services/cli/src/shell/posix.rs` | NEW — bash/zsh/fish rc writers |
| `services/cli/src/shell/windows.rs` | NEW — pwsh / cmd handling |
| `services/cli/src/tools/mod.rs` | NEW — 14-tool override table |
| `services/cli/src/lib.rs` | install/uninstall integration |
| `services/cli/tests/shell_integration.rs` | NEW integration tests |

## Test/verification plan

1. `cargo build --manifest-path services/cli/Cargo.toml` clean
2. `cargo test --manifest-path services/cli/Cargo.toml --lib` — 73 SLICE 4 baseline + ~14 new shell/tools unit tests = 87+ passing
3. macOS + Linux + Windows trust regression: all unchanged
4. `cargo clippy -D warnings` + `cargo fmt --check` clean
5. Marker block idempotency: re-running install adds the block once, not N times

## Anti-scope

- No Gemini OAuth refusal — SLICE 6
- No doctor impl — SLICE 7
- No uninstall workflow beyond signature — SLICE 8
- No SSL_CERT_FILE / CURL_CA_BUNDLE on Linux user scope — that's SLICE 7 doctor redirect surface

## Backlinks

- Spec set: [`design.md`](../../specs/coverage/D02_closed_cli_install/design.md) §4 (14-tool table + HTTPS_PROXY pattern), §7 slice 5 row
- SLICE 4: [`COV_08_d02_trust_windows.md`](COV_08_d02_trust_windows.md)
