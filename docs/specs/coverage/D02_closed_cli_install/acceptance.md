# D02 — Acceptance Gates

Per build plan §3, every gate listed here must be **100% feasible** at slice-spec time: runnable in the current repo state, no third-party action required, reproducible by the `superpowers:code-reviewer` skill.

## 1. Repository-state gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A1.1` | `services/cli` exists as a workspace member | `cargo metadata --format-version 1 \| jq -e '.packages[] \| select(.name == "spendguard-cli")'` |
| `A1.2` | `services/cli/data/tools.toml` exists and parses | `cargo run -p spendguard-cli -- _internal validate-tools-toml` (sub-command landed in slice `COV_09`) |
| `A1.3` | `services/cli/data/tools.toml` covers every tool id from `design.md` §5 | unit test `tools_toml_covers_strategy_memo_list` green |
| `A1.4` | `services/egress_proxy/src/routing.rs` contains a `ProviderKind::Gemini` arm | `cargo test -p spendguard-egress-proxy routes_gemini_api_key_generate_content` green |
| `A1.5` | `docs/site-v2/src/content/docs/integrations/closed-cli-install.md` exists | `test -f docs/site-v2/src/content/docs/integrations/closed-cli-install.md` |
| `A1.6` | `docs/site-v2/src/content/docs/integrations/gemini-cli.md` exists with the OAuth legal warning | `grep -q '2026-03-25' docs/site-v2/src/content/docs/integrations/gemini-cli.md` |
| `A1.7` | `README.md` `## Adapter integrations` table includes a "Closed CLI install" row | `grep -q 'Closed CLI install' README.md` |

## 2. Build gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A2.1` | Workspace builds | `cargo build --workspace --locked` exits 0 |
| `A2.2` | CLI binary builds | `cargo build -p spendguard-cli --release --locked` exits 0 |
| `A2.3` | CLI binary is reachable | `target/release/spendguard --version` prints a SemVer |
| `A2.4` | No new MSRV warnings | `cargo build --workspace -- -D warnings` exits 0 |
| `A2.5` | Clippy clean for the new crate | `cargo clippy -p spendguard-cli --all-targets -- -D warnings` exits 0 |
| `A2.6` | `cargo deny check` passes (no new disallowed licences from `rcgen`, `keychain-services`, `windows`) | `cargo deny check` exits 0 |

## 3. Unit-test gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A3.1` | All `services/cli` unit tests green | `cargo test -p spendguard-cli --lib` exits 0 |
| `A3.2` | Routing addition tests green | `cargo test -p spendguard-egress-proxy --lib routing::tests::routes_gemini` exits 0 |
| `A3.3` | `tools_toml_covers_strategy_memo_list` green | `cargo test -p spendguard-cli tools_toml_covers_strategy_memo_list` exits 0 |
| `A3.4` | Gemini gate refusal unit tests green | `cargo test -p spendguard-cli gemini_gate::tests` exits 0 |

## 4. OS-conditional integration-test gates

`A4.x` gates run on the matching CI runner; the reviewer can re-run them via the GH Actions `re-run jobs` button. The matrix is the seven jobs from `tests.md` §8.

| ID | Gate | Verification command (runner-local) |
|----|------|---------------------------------------|
| `A4.1` | macOS trust-store integration green | `cargo test -p spendguard-cli --test trust_macos -- --include-ignored` |
| `A4.2` | Linux (Debian path) trust-store integration green | `cargo test -p spendguard-cli --test trust_linux -- --include-ignored` on `ubuntu-24.04` |
| `A4.3` | Linux (RHEL path) trust-store integration green | the same test command, inside `fedora:42` container |
| `A4.4` | Linux (Arch path) trust-store integration green | the same test command, inside `archlinux:base` container |
| `A4.5` | Windows trust-store integration green | `cargo test -p spendguard-cli --test trust_windows -- --include-ignored` on `windows-2025` |
| `A4.6` | Round-trip test green on macOS | `cargo test -p spendguard-cli --features round-trip --test round_trip` |
| `A4.7` | Round-trip test green on Linux | the same command, ubuntu-24.04 |
| `A4.8` | Round-trip test green on Windows | the same command, windows-2025 |

## 5. Per-CLI smoke gate

| ID | Gate | Verification command |
|----|------|----------------------|
| `A5.1` | Smoke matrix container images build | `docker buildx bake -f deploy/demo/cli-smoke/bake.hcl --print` exits 0 |
| `A5.2` | At least the four "core" containers (Claude Code, Codex, Aider, Goose) pass | `cargo test -p spendguard-cli --features smoke-cli --test per_cli_smoke -- --skip experimental` exits 0 |
| `A5.3` | Each remaining container exists with a passing OR `experimental` annotation | `cargo test … per_cli_smoke -- --list` enumerates all 15 entries; failing ones must be `#[ignore]` with the `experimental` reason |

`A5.2` is the merge-blocking gate; `A5.3` only enforces that nothing is silently missing.

## 6. Demo-mode regression gate

| ID | Gate | Verification command |
|----|------|----------------------|
| `A6.1` | `make -C deploy/demo demo-verify-closed-cli-install` exits 0 | runs `install → drive a curl → verify SQL → uninstall → verify uninstall` |
| `A6.2` | The verifier SQL is committed | `test -f deploy/demo/verify_step_closed_cli_install.sql` |
| `A6.3` | The existing `make -C deploy/demo demo-verify-step9` still passes (no regression in the baseline forward proxy) | `make -C deploy/demo demo-verify-step9` exits 0 |

## 7. Legal-gate evidence

| ID | Gate | Verification command |
|----|------|----------------------|
| `A7.1` | Installer refuses to enable Gemini in OAuth-only mode | `cargo test -p spendguard-cli gemini_gate_refusal_exit_code` exits 0 |
| `A7.2` | Doc page mentions the 2026-03-25 Google enforcement date | `grep -qE '2026-03-25' docs/site-v2/src/content/docs/integrations/gemini-cli.md` |
| `A7.3` | Doc page steers to API key / Vertex paths | `grep -qE 'gemini auth use-api-key\|GOOGLE_APPLICATION_CREDENTIALS' docs/site-v2/src/content/docs/integrations/gemini-cli.md` |

## 8. Uninstall symmetry gate

| ID | Gate | Verification command |
|----|------|----------------------|
| `A8.1` | After `install` then `uninstall`, no artifact remains | `cargo test -p spendguard-cli --test round_trip uninstall_is_complete` exits 0 |
| `A8.2` | Uninstall after partial install is graceful | `cargo test -p spendguard-cli uninstall_with_no_prior_install` exits 0 |
| `A8.3` | Rc files are stripped exactly, preserving user content outside the markers | unit test `posix_stripper_removes_block_only` green |

## 9. Documentation gate

| ID | Gate | Verification command |
|----|------|----------------------|
| `A9.1` | `docs/site-v2/src/content/docs/integrations/closed-cli-install.md` lists every tool in `tools.toml` | unit test `docs_integrations_closed_cli_install_md_lists_every_tool` green |
| `A9.2` | The doc has install / uninstall / doctor sections | `grep -qE '^## (Install\|Uninstall\|Doctor)' docs/site-v2/src/content/docs/integrations/closed-cli-install.md` (three matches) |
| `A9.3` | CHANGELOG entry exists for the D02 release line | `grep -q 'D02' CHANGELOG.md` |

## 10. Memory write-back gate

| ID | Gate | Verification command |
|----|------|----------------------|
| `A10.1` | `~/.claude/projects/-Users-michael-chen-products-agentic-spendguard/memory/project_coverage_d02_shipped.md` exists | `test -f` |
| `A10.2` | The memory entry follows the GA / HARDEN / POST_GA single-paragraph convention | manual review by Software Architect during R5 panel if invoked |

## 11. "Reviewer can re-run everything" gate

| ID | Gate | Verification command |
|----|------|----------------------|
| `A11.1` | `scripts/coverage/d02_acceptance.sh` invokes every command above and exits 0 | `bash scripts/coverage/d02_acceptance.sh` |
| `A11.2` | The script needs no privileged access beyond what the per-runner CI job already grants | code review verifies no `aws`, `gh login`, or unscoped `sudo` calls outside the OS trust-store-install path |

`A11.1` is the single integration point: the `superpowers:code-reviewer` skill runs this one script per slice review and inspects the per-step output.
