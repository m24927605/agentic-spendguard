# D02 — Review Standards

Slice-specific checklist for the `superpowers:code-reviewer` skill across `COV_05` … `COV_12`. Each slice review consults this file plus [`acceptance.md`](acceptance.md) plus the repo-wide coding standards.

## 1. Threat-model assertions

The CLI handles a root CA private key and modifies OS trust stores. Any diff that touches `services/cli/src/ca.rs`, `services/cli/src/trust/**`, or `services/cli/src/paths.rs` MUST be reviewed against these assertions; reviewer flags as a Blocker if any fails.

| ID | Assertion |
|----|-----------|
| `T1` | The CA private key never crosses a process boundary in plaintext other than (a) writing to its on-disk file at the user-resolved path or (b) loading from that file. No logging of key bytes. No stdout/stderr dump. No env var carrying the key. |
| `T2` | The on-disk private key file is created with mode `0o600` on POSIX, with DPAPI sealing on Windows. Reviewer greps for `set_mode\|OpenOptions::new().*mode` in the diff to confirm. |
| `T3` | The CA leaf is constrained: BasicConstraints `cA:FALSE`, KeyUsage = `digitalSignature + keyEncipherment`, ExtKeyUsage = `serverAuth`, SAN restricted to `localhost` + `127.0.0.1` + `::1`. The leaf MUST NOT carry a wildcard SAN. |
| `T4` | The root cert lifetime is bounded at 825 days (Apple max for Safari trust). Reviewer rejects bumps without an ADR. |
| `T5` | UUIDv7 serial: high 48 bits time-derived, low 80 bits cryptographically random. Reviewer flags if the diff uses `uuid::Uuid::new_v4` or a counter instead. |
| `T6` | Trust-store mutations are scoped: the diff calls `security add-trusted-cert` / `update-ca-certificates` / `certutil` exactly once per install, with the user-resolved `scope` argument. No silent escalation from `user` to `system`. |
| `T7` | Process privilege drops: if the installer was invoked via `sudo`, post-install file mode operations are performed as the original user (`SUDO_UID` / `SUDO_GID`), not root. |
| `T8` | No shell injection through `tools.toml` `detect_binaries`: every value passes through `which::which`, never `Command::new("sh").arg("-c", …)`. |
| `T9` | Uninstall is a true inverse — verified by `cargo test … uninstall_is_complete`. Reviewer rejects "delete on best-effort" without an explicit `EX_TEMPFAIL` exit path. |
| `T10` | The Gemini OAuth gate is enforced **before** any trust-store mutation. If `gemini_gate.rs` returns `Refusal::OauthFreeMode` and `--include gemini` is set, the installer exits before issuing or installing any cert. |

## 2. Cross-platform correctness assertions

Slices `COV_06` / `COV_07` / `COV_08` each modify exactly one OS backend. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `X1` | The diff's OS-conditional code is gated by `#[cfg(target_os = "…")]`, not by runtime `std::env::consts::OS` checks in production paths. Runtime checks are only allowed inside `trust/linux.rs` to choose the distro family. |
| `X2` | `trust/linux.rs` family detection reads `/etc/os-release`, not `lsb_release` (the latter is not present on minimal containers). |
| `X3` | Each backend's `is_trusted(fingerprint)` returns `Ok(false)` when the cert is absent — it must NOT error. Idempotent install depends on this. |
| `X4` | macOS keychain interactions use `kSecAttrAccessibleWhenUnlockedThisDeviceOnly` for stored secrets — no `kSecAttrAccessibleAlways`. |
| `X5` | Windows DPAPI calls use `CRYPTPROTECT_LOCAL_MACHINE` flag `0` (per-user binding), unless `--scope system` is set. |
| `X6` | The Linux Arch fallback path uses `p11-kit trust anchor`, not `trust extract` — the latter requires a writable system trust store. |

## 3. Per-tool override matrix correctness

`COV_09` modifies `tools.toml` + `overrides.rs`. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `M1` | Every entry in `design.md` §5 is present in `tools.toml`. Reviewer cross-references both. |
| `M2` | Node-based tools (`claude_code`, `continue`, `cline`, `roo`, `copilot_byok`, `augment`) MUST set `NODE_EXTRA_CA_CERTS`, NOT `SSL_CERT_FILE` (Node ignores OS trust, but does honour `NODE_EXTRA_CA_CERTS`). |
| `M3` | `codex` MUST use `CODEX_CA_CERTIFICATE` — the native env var landed in Codex CLI 0.30. Falling back to `NODE_EXTRA_CA_CERTS` is acceptable but only as a tail-merge. |
| `M4` | `aider` MUST set `REQUESTS_CA_BUNDLE`. `SSL_CERT_FILE` alone is insufficient because `requests` ships its own CA bundle. |
| `M5` | `goose` / `amazon_q` / `zed` MUST NOT set extra env vars beyond `HTTPS_PROXY` — they honour OS trust. Reviewer rejects redundant exports. |
| `M6` | `tabnine_enterprise` writes to its config file (`~/.config/TabNine/config.json` or platform equivalent), NOT to a shell rc — env vars do not persist into the VS Code subprocess that hosts Tabnine. |
| `M7` | Every entry has a `display` field used by the doctor report. |

## 4. Routing-table addition assertions

`COV_10` touches `services/egress_proxy/src/routing.rs`. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `R1` | The Gemini API-key row is **additive**. No existing row mutated. |
| `R2` | The path regex is anchored (`^` and `$`) and uses a non-greedy single-segment match for the model id. |
| `R3` | `ProviderKind::Gemini` is appended to the enum, not inserted in the middle (downstream `match` arms remain exhaustive). |
| `R4` | `ProviderKind::Gemini.as_str() == "gemini"` (CloudEvent payload schema lock). |
| `R5` | The new row precedes the existing Vertex row textually; the regex is restrictive enough that the order does not matter, but textual ordering eases code review. |
| `R6` | A test asserts that the Vertex path does NOT collide with the new Gemini route. |
| `R7` | `providers::gemini::extract_usage` returns identical numeric fields as `providers::vertex::extract_usage` for the same response shape (the wire formats are nearly identical; reviewer requires a doc comment justifying any divergence). |

## 5. Demo / Makefile assertions

`COV_12` adds the demo target. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `D1` | The new Makefile target follows the existing `demo-verify-*` naming and uses the same `make demo-up` pre-condition. |
| `D2` | The verifier SQL uses the same `audit_outbox` schema columns the existing `verify_step_litellm_*.sql` targets use — no schema drift. |
| `D3` | The demo target exits cleanly even when the installer leaves a CA in the test container's trust store (the container is the isolation boundary). |
| `D4` | `make -C deploy/demo demo-clean` removes any artifacts the new target placed under `/tmp/spendguard-demo-cli/`. |

## 6. R1-R5 escalation criteria

| Round | Blocker count | Action |
|-------|--------------|--------|
| R1 | 0 → MERGE | none |
| R1 | ≥ 1 → dispatch same implementer with findings | typical 1-3 findings on first review |
| R2-R4 | drop to 0 → MERGE | follow normal cadence |
| R5 | still > 0 → 5-person Staff+ panel arbitration | per build plan §1.3 |

R5 panel summarizer for D02 defaults to **Security Engineer**, not Software Architect, because the dominant risk surface (root CA trust mutation) is security. This overrides the build plan's default per-deliverable customisation right (build plan §1.3 last bullet).

## 7. Specific Blocker exemplars

Reviewer SHOULD raise these as Blockers; they are seen-before-in-this-codebase footguns:

1. Issuing the CA via `rcgen` defaults — `rcgen` defaults a serial of `42`, which collides across installs. UUIDv7 serial is required.
2. Writing rc lines via `>> ~/.zshrc` shell out — fails on read-only home directories and on profile files behind `chezmoi` symlinks. The writer must operate on the file's resolved real path with a temp-file + atomic rename.
3. Using `which::which` to detect tools but then invoking them via `Command::new("…").arg("--version")` — Tabnine's startup makes a network call; `which` is sufficient.
4. Skipping the round-trip TLS handshake test in `doctor` because "the cert was issued" — issuance does not prove trust-store install succeeded.
5. Leaving the Linux `--scope user` path silently inoperative (it cannot mutate `/etc/ssl/certs/`) without printing the `CURL_CA_BUNDLE` recommendation.

## 8. Out-of-review scope

Reviewer does NOT review:

- The strategy memo `docs/strategy/framework-coverage-2026-06.md` — it is the input.
- D01 / D03 / D04 / D13 cross-cutting concerns — separate deliverables.
- The actual Anthropic / OpenAI / Google API behaviour — fixtures abstract that surface.
- ASP Draft-02 spec drafting — see [`docs/specs/agent-spend-protocol/`](../../agent-spend-protocol/).
