# D02 — Closed CLI Pattern 3 Install Script + CA Bootstrap

**Status:** Spec — Tier 1, build plan §2.1.
**Parent strategy:** [`docs/strategy/framework-coverage-2026-06.md`](../../../strategy/framework-coverage-2026-06.md), Pattern 3 / Closed CLI deep dive.
**Owner sub-agent:** DevOps Automator.

## 1. Problem

Pattern 3 — forward HTTPS proxy with a customer-installed root CA — is the only path that gates closed-binary CLIs that do not honour an OpenAI-compatible `*_BASE_URL`. ~14 BYOK tools collapse to "install one root CA + set `HTTPS_PROXY`": Claude Code, Codex, Gemini (API key / Vertex), Aider, Continue, Cline / Roo, OpenHands, Goose, Amazon Q v1.8+, Copilot CLI BYOK, Tabnine Enterprise, Cody self-hosted, Augment BYOK, Zed.

`services/egress_proxy/src/routing.rs` covers OpenAI / Anthropic / Bedrock / Vertex / Azure OpenAI. No install tooling, no CA bootstrap, no per-tool env helper, and the public Gemini `generativelanguage.googleapis.com` API-key path is missing — Vertex covers only the GCP-authenticated form.

## 2. Goals

1. `spendguard install` issues a root CA, trusts it on macOS / Linux / Windows, issues a leaf cert for `localhost:8443`, writes a per-shell rc snippet exporting `HTTPS_PROXY` plus per-tool overrides.
2. Add `generativelanguage.googleapis.com` inbound route (API-key, NOT OAuth — §6).
3. Per-tool override matrix programmatically picks env vars per CLI.
4. Symmetric `spendguard uninstall` removes every artifact.
5. Per-CLI smoke drives one call through a stub upstream + asserts an audit row.

## 3. Non-goals

- PKI cross-sign / intermediate chains: Tier 3.
- Subscription-tier metering: D13.
- Cursor / Windsurf MITM codec: D17 / D18.
- Gemini OAuth free tier: legal red line (§6).

## 4. Architecture

```
spendguard CLI (Rust binary, services/cli)
  install  : ca::ensure_root → ca::issue_leaf → trust::install → shell::write_rc → tools::write_overrides
  uninstall: tools::clear_overrides → shell::strip_rc → trust::uninstall
  doctor   : CA fingerprint in store + HTTPS_PROXY reachable + TLS handshake round-trip
```

Trust-store backends:

| OS | Backend |
|----|---------|
| macOS | `security add-trusted-cert -r trustRoot -k {login,System}.keychain` |
| Linux Debian/Ubuntu | `/usr/local/share/ca-certificates/` + `update-ca-certificates` |
| Linux RHEL/Fedora | `/etc/pki/ca-trust/source/anchors/` + `update-ca-trust extract` |
| Linux Arch / fallback | `p11-kit trust anchor` |
| Windows | `certutil -addstore {-user,} -f Root` |

Installer prefers per-user trust where the OS supports it; escalates to system trust only with explicit `--scope system`. **R2 Linux clarification (SLICE 3 / COV_07):** Linux's `update-ca-certificates`, `update-ca-trust`, and `trust anchor` all read from system paths only — there is no per-user analogue. `--scope user` on Linux therefore fails closed at the `TrustStore` boundary; the SLICE 7 doctor surfaces the `CURL_CA_BUNDLE` / `SSL_CERT_FILE` env-var redirect instead. macOS and Windows keep their per-user keychain / cert store paths.

**Module-declaration pattern (LOCKED at SLICE 3 R2):** `pub mod macos;` is `#[cfg(target_os = "macos")]`-gated because its primary code path uses `/usr/bin/security` shellouts whose argv shape is macOS-only; the cross-platform compile is not needed. `pub mod linux;` is NOT cfg-gated because its FakeRunner-driven unit tests run on every workspace member's `cargo test` invocation (including macOS dev hosts), and the production paths only touch the live filesystem through test-injectable overrides, so the macOS-host compile is hermetic. SLICE 4 `pub mod windows;` SHOULD follow the linux pattern (ungated) for the same FakeRunner-on-macOS-dev reason, but MUST avoid `windows` / `windows-sys` crate types in trait-bound public signatures (inject via a `WinCertStore` trait inside the module instead, mirroring how `linux.rs` keeps `CommandRunner` as the boundary).

## 5. Per-tool override matrix

| Tool | Extra env required |
|------|--------------------|
| Claude Code (Node) | `NODE_EXTRA_CA_CERTS` |
| Codex CLI (Node) | `CODEX_CA_CERTIFICATE` (native) |
| Gemini CLI (Node, API-key) | `NODE_EXTRA_CA_CERTS` |
| Aider (Python) | `REQUESTS_CA_BUNDLE` |
| Continue (Node, VS Code) | `NODE_EXTRA_CA_CERTS` |
| Cline / Roo (Node, VS Code BYOK) | `NODE_EXTRA_CA_CERTS` |
| OpenHands (Python) | `SSL_CERT_FILE` |
| Goose (Rust) | none beyond `HTTPS_PROXY` |
| Amazon Q v1.8+ (Rust) | none |
| GitHub Copilot CLI BYOK (Node) | `NODE_EXTRA_CA_CERTS` |
| Tabnine Enterprise | `tabnine.caBundle` config key |
| Cody self-hosted | `SRC_HTTPS_PROXY` |
| Augment BYOK (Node) | `NODE_EXTRA_CA_CERTS` |
| Zed (Rust) | none |

Node-based tools ignore the OS trust store, so `NODE_EXTRA_CA_CERTS` is mandatory. Installer reads `services/cli/data/tools.toml` and emits only the rc lines required by tools detected via `which`; `--include` / `--exclude` override.

## 6. Legal red line — Gemini OAuth

Google banned OAuth-token proxying for Gemini Code Assist on 2026-02 with enforcement from 2026-03-25 (strategy memo §"Archetype V"). The installer:

- Routes only `generativelanguage.googleapis.com` API-key and Vertex GCP-token paths.
- Refuses `HTTPS_PROXY` setup when Gemini is in OAuth free-tier mode. Detection: `~/.gemini/oauth_creds.json` present AND `GEMINI_API_KEY` unset AND `GOOGLE_APPLICATION_CREDENTIALS` unset.
- On `install --include gemini` in banned mode: emits `legal_gate_gemini_oauth`, exits non-zero, steers to `gemini auth use-api-key` or `GOOGLE_APPLICATION_CREDENTIALS`.
- `docs/site-v2/src/content/docs/integrations/gemini-cli.md` carries the same warning above the fold.

## 7. Slice plan

| Slice | Title | Size |
|-------|-------|------|
| `COV_05_d02_ca_root_and_leaf` | CA gen + leaf issuance (`rcgen`-backed) | M |
| `COV_06_d02_trust_macos` | macOS keychain install/uninstall | S |
| `COV_07_d02_trust_linux` | Linux multi-distro install/uninstall | M |
| `COV_08_d02_trust_windows` | Windows `certutil` install/uninstall | S |
| `COV_09_d02_per_tool_overrides` | Env var emitter + shell rc writer | M |
| `COV_10_d02_gemini_routing` | API-key route + OAuth refusal | S |
| `COV_11_d02_doctor_and_uninstall` | `spendguard doctor` + symmetric uninstall | M |
| `COV_12_d02_smoke_each_cli` | Per-CLI smoke harness + CI matrix | M |

## 8. Interfaces

```rust
// services/cli/src/lib.rs (full skeleton in implementation.md)
pub fn install(opts: InstallOpts) -> Result<InstallReport>;
pub fn uninstall(opts: UninstallOpts) -> Result<UninstallReport>;
pub fn doctor() -> Result<DoctorReport>;
```

CLI: `spendguard install [--scope user|system] [--include …] [--exclude …] [--ca-out …] [--shell bash|zsh|fish|pwsh]`; symmetric `uninstall`; `doctor`.

## 9. Locked decisions

1. CA validity 825 days (Apple-Safari max).
2. CA serial: UUIDv7 → `BigUint`, never reused.
3. rc placement: between `# >>> spendguard >>>` / `# <<< spendguard <<<` markers.
4. Per-user trust by default; `--scope system` opts in.
5. R5 panel summarizer: Security Engineer (root CA mutation is dominant risk surface; overrides build-plan default).
