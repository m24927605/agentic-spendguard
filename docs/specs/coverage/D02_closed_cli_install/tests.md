# D02 — Tests

Companion to [`design.md`](design.md) and [`implementation.md`](implementation.md). Defines unit coverage, OS-conditional integration coverage, demo-mode regression coverage, and the per-CLI smoke matrix.

## 1. Unit tests (cross-platform, `cargo test -p spendguard-cli`)

### 1.1 `ca.rs`

| Test | Asserts |
|------|---------|
| `ca_issue_root_has_basic_constraints_ca_true` | `BasicConstraints.cA == true`, `pathLenConstraint == 0`. |
| `ca_issue_root_has_key_usage_certsign_crlsign` | KeyUsage bits limited to `keyCertSign + cRLSign`. |
| `ca_issue_root_serial_is_uuidv7_derived` | High 48 bits of serial match the UUIDv7 timestamp field within ±2s of test clock. |
| `ca_issue_root_validity_825_days` | `not_after - not_before == 825 days`. |
| `ca_issue_leaf_has_san_localhost_and_ip` | Leaf SAN contains `DNSName: localhost` AND `IPAddress: 127.0.0.1`. |
| `ca_load_then_reissue_is_idempotent` | `RootCa::ensure` is a no-op if a valid root already exists. |
| `ca_load_rejects_expired_root` | Synthesized root with `not_after < now` triggers re-issue. |
| `ca_root_fingerprint_stable` | SHA-256 fingerprint is deterministic given the same DER bytes. |
| `ca_drop_zeroizes_key` | After `drop(root)`, the underlying buffer is zero (verified via raw-pointer borrow in a `MaybeUninit` harness). |

### 1.2 `overrides.rs` / `tools.rs`

| Test | Asserts |
|------|---------|
| `tools_toml_parses_without_panic` | `data/tools.toml` loads, every entry has at least one `detect_binaries`. |
| `tools_toml_covers_strategy_memo_list` | Every tool id from `design.md` §5 is present. |
| `overrides_node_extra_ca_certs_set_for_node_tools` | For `claude_code`, `continue`, `cline`, `copilot_byok`, `augment`, the rc snippet contains `NODE_EXTRA_CA_CERTS=…`. |
| `overrides_codex_uses_native_var` | For `codex` the rc snippet contains `CODEX_CA_CERTIFICATE=…` and NOT `NODE_EXTRA_CA_CERTS`. |
| `overrides_aider_uses_requests_ca_bundle` | For `aider` the snippet contains `REQUESTS_CA_BUNDLE=…`. |
| `overrides_openhands_uses_ssl_cert_file` | `SSL_CERT_FILE=…`. |
| `overrides_zero_overrides_when_no_tools_detected` | `--include` empty + nothing on PATH → only `HTTPS_PROXY` is set. |
| `overrides_explicit_include_overrides_detection` | `--include codex` produces the codex line even when `which codex` returns nothing. |
| `overrides_exclude_takes_precedence_over_include` | `--include codex --exclude codex` → no codex line. |

### 1.3 `shell/posix.rs` + friends

| Test | Asserts |
|------|---------|
| `posix_writer_creates_block_with_markers` | Output starts with `# >>> spendguard …` and ends with `# <<< spendguard <<<`. |
| `posix_writer_is_idempotent` | Calling writer twice on the same rc file yields a file with exactly one block. |
| `posix_stripper_removes_block_only` | Pre-existing user content outside markers is preserved byte-identical. |
| `posix_stripper_handles_missing_block_gracefully` | Strip on a file without the block is a no-op, exit 0. |
| `fish_writer_uses_set_x` | Fish output uses `set -gx`, not `export`. |
| `pwsh_writer_uses_env_setter` | PowerShell output uses `$env:HTTPS_PROXY = …`. |

### 1.4 `gemini_gate.rs`

| Test | Asserts |
|------|---------|
| `gemini_gate_detects_oauth_creds_file` | Presence of `~/.gemini/oauth_creds.json` AND no env keys → `RefusalReason::OauthFreeMode`. |
| `gemini_gate_allows_api_key_mode` | `GEMINI_API_KEY=…` set → returns `Decision::Allow`. |
| `gemini_gate_allows_vertex_sa_mode` | `GOOGLE_APPLICATION_CREDENTIALS=/path/sa.json` set → `Decision::Allow`. |
| `gemini_gate_refusal_exit_code` | `install --include gemini` in OAuth mode exits with code `78` (`EX_CONFIG`). |
| `gemini_gate_emits_legal_warning_text` | Stderr contains `2026-03-25` and a Vertex / API-key steer URL. |

### 1.5 Routing-table addition (`services/egress_proxy`)

| Test | Asserts |
|------|---------|
| `routes_gemini_api_key_generate_content` | `route("/v1beta/models/gemini-1.5-pro:generateContent")` returns `ProviderKind::Gemini`, `EncoderKind::Gemini`. |
| `routes_gemini_does_not_collide_with_vertex` | Vertex path (`/v1/projects/.../publishers/google/models/...:generateContent`) still routes to Vertex, not Gemini. |
| `routes_gemini_upstream_url_substitutes_model` | `upstream_url_for(...)` embeds the model id. |
| `routes_gemini_provider_kind_str_stable` | `ProviderKind::Gemini.as_str() == "gemini"` (CloudEvent payload contract). |

## 2. OS-conditional integration tests

Gated behind `#[cfg(target_os = "…")]` and `#[ignore]` so CI can opt-in per runner.

### 2.1 macOS (`tests/trust_macos.rs`)

| Test | Asserts |
|------|---------|
| `macos_install_user_scope_adds_to_login_keychain` | After `install --scope user`, `security find-certificate -c "SpendGuard Local Root R1" $HOME/Library/Keychains/login.keychain-db` returns 0. |
| `macos_install_user_scope_marks_as_trustroot` | `security verify-cert -c <leaf.pem>` succeeds. |
| `macos_uninstall_user_scope_removes_cert` | After `uninstall`, the find-certificate command returns non-zero. |
| `macos_install_is_idempotent` | Two installs in a row leave exactly one matching cert in the keychain. |
| `macos_system_scope_requires_sudo` | `install --scope system` without sudo exits non-zero with a clear error. |

### 2.2 Linux (`tests/trust_linux.rs`)

| Test | Asserts |
|------|---------|
| `linux_debian_install_writes_to_local_share_ca_certs` | After `install --scope system`, `/usr/local/share/ca-certificates/spendguard.crt` exists. |
| `linux_debian_update_ca_certificates_invoked` | The CA appears in `/etc/ssl/certs/ca-certificates.crt`. |
| `linux_rhel_install_writes_to_pki_anchors` | (CI matrix entry under a `fedora:42` container) anchors file exists; `update-ca-trust extract` was invoked. |
| `linux_arch_falls_back_to_p11_kit` | (CI matrix `archlinux:base`) `trust list --filter=ca-anchors` includes the cert. |
| `linux_user_scope_warns_no_system_trust` | `install --scope user` on Linux prints the `CURL_CA_BUNDLE` recommendation and exits 0. |
| `linux_uninstall_runs_update_ca_certificates_again` | Post-uninstall, `/etc/ssl/certs/ca-certificates.crt` no longer contains the cert. |

### 2.3 Windows (`tests/trust_windows.rs`)

| Test | Asserts |
|------|---------|
| `windows_install_user_scope_adds_to_root_store` | `certutil -store -user Root` output contains `SpendGuard Local Root R1`. |
| `windows_uninstall_user_scope_removes_cert` | After uninstall the cert is no longer in `-store -user Root`. |
| `windows_install_idempotent` | Two consecutive installs result in one entry. |

## 3. End-to-end round-trip (`tests/round_trip.rs`)

`#[cfg_attr(not(feature = "round-trip"), ignore)]`. Runs cross-platform once Phase 8 of the slice plan ships:

1. `install --scope user --include none` writes only `HTTPS_PROXY`.
2. Spawn `services/egress_proxy` listening on the configured `proxy_listen` with the installer-issued leaf cert.
3. Spawn a stub upstream (`tests/fixtures/upstream_stub.rs`) that mimics `api.openai.com/v1/chat/completions`.
4. Drive an in-process HTTP client through `HTTPS_PROXY` to `https://api.openai.com/v1/chat/completions`.
5. Assert: HTTP 200; chain validates against the installed CA; one row appears in the audit outbox table with the expected `tokenizer_kind = openai`.
6. `uninstall` is invoked; the same client now fails certificate validation.

## 4. Per-CLI smoke matrix (`tests/per_cli_smoke.rs`)

Gated behind `--features smoke-cli`. Each entry runs in a per-tool container under `deploy/demo/cli-smoke/<tool>/Dockerfile`:

| Tool | Smoke command | Pass condition |
|------|---------------|----------------|
| Claude Code | `claude --print "say ok"` against stub | exit 0, audit row exists |
| Codex | `codex exec --quiet "say ok"` | exit 0, audit row exists |
| Gemini (API key) | `GEMINI_API_KEY=stub gemini -p "say ok"` | exit 0, audit row exists |
| Gemini (OAuth banned) | `gemini -p "say ok"` with `oauth_creds.json` present | install refused before CLI runs |
| Aider | `aider --message "say ok" --no-git --yes` | exit 0, audit row exists |
| Continue | headless config + one completion request | exit 0, audit row exists |
| Cline / Roo | scripted via `cline-cli` | exit 0, audit row exists |
| OpenHands | `openhands run --task "say ok" --headless` | exit 0, audit row exists |
| Goose | `goose run --instructions /tmp/say-ok.txt` | exit 0, audit row exists |
| Amazon Q (v1.8+) | `q chat --no-interactive "say ok"` | exit 0, audit row exists |
| Copilot CLI BYOK | `gh copilot suggest "say ok" --target shell` | exit 0, audit row exists |
| Tabnine Enterprise | startup probe (`tabnine --version` + completion ping) | exit 0, audit row exists |
| Cody self-hosted | `cody chat --message "say ok"` against stub Sourcegraph | exit 0, audit row exists |
| Augment BYOK | scripted completion via Augment CLI | exit 0, audit row exists |
| Zed | scripted `zed --eval` driving the AI panel | exit 0, audit row exists |

A failed smoke for an individual tool fails the matrix but does not block merge of D02 — the per-tool container is marked `experimental` until two consecutive green runs.

## 5. Demo-mode regression (`deploy/demo/`)

Add `demo-verify-closed-cli-install`:

1. `make demo-up` brings the stack up as today (Postgres + ledger + egress proxy + outbox forwarder).
2. The test driver runs `spendguard install --scope user --include claude_code,codex,aider --proxy-listen 127.0.0.1:18443`.
3. Drives one chat-completion through `curl` (the lowest-common-denominator smoke; per-CLI containers are §4).
4. SQL verifier `deploy/demo/verify_step_closed_cli_install.sql` asserts: exactly one `audit_outbox` row with `provider = 'openai'` and `tokenizer_kind = 'openai'`.
5. `spendguard uninstall` runs and a second `curl` call fails (certificate validation), confirming uninstall is real.

Targets added to `deploy/demo/Makefile` alongside the existing `demo-verify-step*` family.

## 6. Negative / refusal coverage

| Test | Asserts |
|------|---------|
| `install_rejects_world_writable_ca_dir` | If `~/.local/share/spendguard` is `0o777`, install exits non-zero. |
| `install_refuses_to_overwrite_user_managed_marker` | If the rc file already contains a `# >>> spendguard >>>` block whose CA fingerprint differs, install warns and refuses unless `--force`. |
| `install_refuses_system_scope_without_admin_on_windows` | UAC denial → exit non-zero with actionable error. |
| `install_records_legal_gate_in_install_report` | `InstallReport.gemini_oauth_refusals` is populated when applicable. |
| `uninstall_partial_failure_returns_75` | Simulated trust-store failure → exit 75 + remaining artifacts listed. |

## 7. Documentation tests

| Test | Asserts |
|------|---------|
| `docs_integrations_gemini_cli_md_includes_legal_warning` | `grep -q "2026-03-25"` against the file. |
| `docs_integrations_closed_cli_install_md_lists_every_tool` | Doc page mentions every tool id in `tools.toml`. |
| `readme_adapter_table_includes_closed_cli_row` | `grep -q "Closed CLI install"` in `README.md`. |

## 8. CI matrix

| Job | OS | Tests |
|-----|----|-------|
| `cli-test-macos` | `macos-15` | unit + `trust_macos.rs` + `round_trip.rs` |
| `cli-test-linux-debian` | `ubuntu-24.04` | unit + `trust_linux.rs` (debian path) + `round_trip.rs` |
| `cli-test-linux-fedora` | container `fedora:42` on `ubuntu-24.04` | unit + `trust_linux.rs` (rhel path) |
| `cli-test-linux-arch` | container `archlinux:base` on `ubuntu-24.04` | unit + `trust_linux.rs` (arch path) |
| `cli-test-windows` | `windows-2025` | unit + `trust_windows.rs` + `round_trip.rs` |
| `cli-smoke-matrix` | `ubuntu-24.04` | per-tool containers from §4 |
| `cli-demo-regression` | `ubuntu-24.04` | §5 demo target |

All seven jobs gate merge of any D02 slice into main.
