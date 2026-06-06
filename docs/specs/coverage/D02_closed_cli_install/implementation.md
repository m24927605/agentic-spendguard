# D02 — Implementation

Companion to [`design.md`](design.md). Lays out crate boundaries, module layout, key types, and the routing-table addition.

## 1. Crate layout

A new workspace member: `services/cli` (binary `spendguard`).

```
services/cli/
├── Cargo.toml
├── data/
│   └── tools.toml           # per-tool override matrix (data, not code)
├── src/
│   ├── main.rs              # clap entry point
│   ├── lib.rs               # public API: install / uninstall / doctor
│   ├── ca.rs                # rcgen-backed CA + leaf issuance + serial DB
│   ├── trust/
│   │   ├── mod.rs           # pub trait TrustStore + dispatch
│   │   ├── macos.rs
│   │   ├── linux.rs         # detects debian/rhel/arch family
│   │   └── windows.rs
│   ├── shell/
│   │   ├── mod.rs           # pub trait ShellRc + dispatch
│   │   ├── posix.rs         # bash + zsh shared writer
│   │   ├── fish.rs
│   │   └── pwsh.rs
│   ├── tools.rs             # parse tools.toml + tool detection
│   ├── overrides.rs         # per-tool env var emitter
│   ├── gemini_gate.rs       # OAuth detection + refusal
│   ├── doctor.rs            # health checks
│   └── paths.rs             # XDG / Library / AppData resolution
└── tests/
    ├── ca.rs
    ├── trust_macos.rs       # #[cfg(target_os = "macos")]
    ├── trust_linux.rs       # #[cfg(target_os = "linux")]
    ├── trust_windows.rs     # #[cfg(target_os = "windows")]
    ├── overrides.rs
    ├── gemini_gate.rs
    ├── round_trip.rs        # spawn proxy + curl through it
    └── per_cli_smoke.rs     # gated behind --features smoke-cli
```

Workspace `Cargo.toml` adds:

```toml
[workspace]
members = [..., "services/cli"]
```

## 2. Key types

```rust
// services/cli/src/lib.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, clap::Args)]
pub struct InstallOpts {
    /// `user` (no admin / sudo) or `system` (admin required).
    #[arg(long, default_value = "user")]
    pub scope: TrustScope,

    /// Tool ids to include. Empty = auto-detect via `which`.
    #[arg(long, value_delimiter = ',')]
    pub include: Vec<String>,

    /// Tool ids to exclude (e.g. `gemini` if customer is OAuth-only).
    #[arg(long, value_delimiter = ',')]
    pub exclude: Vec<String>,

    /// Override where the public PEM is written (default: `$SPENDGUARD_HOME/ca.pem`).
    #[arg(long)]
    pub ca_out: Option<PathBuf>,

    /// Override shell detection.
    #[arg(long)]
    pub shell: Option<ShellKind>,

    /// Local listen address the proxy will be reachable on.
    #[arg(long, default_value = "127.0.0.1:8443")]
    pub proxy_listen: SocketAddr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
pub enum TrustScope { User, System }

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ShellKind { Bash, Zsh, Fish, Pwsh }

#[derive(Debug, Serialize)]
pub struct InstallReport {
    pub ca_fingerprint_sha256: String,
    pub ca_pem_path: PathBuf,
    pub leaf_pem_path: PathBuf,
    pub trust_store_locations: Vec<PathBuf>,
    pub shell_rc_paths: Vec<PathBuf>,
    pub tools_configured: Vec<ToolReport>,
    pub gemini_oauth_refusals: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ToolReport {
    pub id: String,
    pub detected_path: Option<PathBuf>,
    pub env_vars_set: Vec<(String, String)>,
    pub config_files_written: Vec<PathBuf>,
}
```

## 3. CA / leaf module

`rcgen 0.13` (already pulled in by `services/control_plane` for SVID minting). Keys:

```rust
// services/cli/src/ca.rs
pub struct RootCa {
    pub cert_pem: String,
    pub key_pem_pkcs8: Zeroizing<String>,   // never written to disk plaintext if scope == User
    pub fingerprint_sha256: [u8; 32],
    pub subject_cn: String,                   // "SpendGuard Local Root R1"
    pub not_before: OffsetDateTime,
    pub not_after: OffsetDateTime,            // not_before + 825 days
    pub serial: BigUint,                      // UUIDv7-derived
}

impl RootCa {
    pub fn ensure(opts: &InstallOpts) -> Result<RootCa>;     // load-or-issue
    fn issue() -> Result<RootCa>;
    fn load(pem: &Path, key: &Path) -> Result<RootCa>;
    pub fn issue_leaf(&self, listen: SocketAddr) -> Result<LeafCert>;
}
```

Storage layout (resolved via `paths.rs`):

| OS | CA PEM | CA key | Leaf PEM | Leaf key |
|----|--------|--------|----------|----------|
| macOS | `~/Library/Application Support/spendguard/ca.pem` | macOS keychain (`Internet password` class, `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`) | `…/leaf.pem` | macOS keychain |
| Linux | `$XDG_DATA_HOME/spendguard/ca.pem` (default `~/.local/share/spendguard/`) | `$XDG_DATA_HOME/spendguard/ca.key.pem`, mode `0600` | `…/leaf.pem` | `…/leaf.key.pem`, `0600` |
| Windows | `%LOCALAPPDATA%\spendguard\ca.pem` | DPAPI (`CryptProtectData`, `CRYPTPROTECT_LOCAL_MACHINE` flag clear) | `…\leaf.pem` | DPAPI |

The key never round-trips through stdout. `RootCa::Drop` zeroizes the PKCS8 buffer.

## 4. Trust-store backends

```rust
// services/cli/src/trust/mod.rs
pub trait TrustStore {
    fn install(&self, ca: &RootCa, scope: TrustScope) -> Result<Vec<PathBuf>>;
    fn uninstall(&self, ca_fingerprint: &[u8; 32]) -> Result<Vec<PathBuf>>;
    fn is_trusted(&self, ca_fingerprint: &[u8; 32]) -> Result<bool>;
}

pub fn dispatch() -> Box<dyn TrustStore> {
    #[cfg(target_os = "macos")] { Box::new(macos::Keychain::default()) }
    #[cfg(target_os = "linux")] { Box::new(linux::detect()) }
    #[cfg(target_os = "windows")] { Box::new(windows::CertStore::default()) }
}
```

**macOS** (`trust/macos.rs`):
- `scope == User` → `security add-trusted-cert -d -r trustRoot -k $HOME/Library/Keychains/login.keychain-db <pem>`
- `scope == System` → `sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain <pem>` (interactive sudo).
- Uninstall by SHA-1 fingerprint via `security delete-certificate -Z <hex> -t`.

**Linux** (`trust/linux.rs`):
- Detect family by probing `/etc/os-release` `ID_LIKE`.
- Debian/Ubuntu → copy to `/usr/local/share/ca-certificates/spendguard.crt`, run `update-ca-certificates`.
- RHEL/Fedora → `/etc/pki/ca-trust/source/anchors/spendguard.crt`, run `update-ca-trust extract`.
- Arch / fallback → `trust anchor --store /etc/ca-certificates/trust-source/<pem>`.
- `scope == User` falls back to `~/.local/share/ca-certificates/` with the strong recommendation to use `CURL_CA_BUNDLE` (printed in the doctor report) since system trust requires root.

**Windows** (`trust/windows.rs`):
- `scope == User` → `certutil -addstore -user -f Root <pem>`.
- `scope == System` → admin-elevated `certutil -addstore -f Root <pem>` (UAC consent prompt).
- Uninstall by SHA-1 hash: `certutil -delstore -user Root <thumbprint>`.

## 5. Shell rc writer

`shell/posix.rs` shared between bash and zsh — they accept identical syntax for `export`. Markers:

```sh
# >>> spendguard (managed by spendguard install) >>>
export HTTPS_PROXY="http://127.0.0.1:8443"
export NODE_EXTRA_CA_CERTS="$HOME/Library/Application Support/spendguard/ca.pem"
export REQUESTS_CA_BUNDLE="$HOME/Library/Application Support/spendguard/ca.pem"
export CODEX_CA_CERTIFICATE="$HOME/Library/Application Support/spendguard/ca.pem"
# <<< spendguard <<<
```

Uninstall strips everything between the markers, inclusive, idempotently. Writer also touches `~/.config/fish/conf.d/spendguard.fish` and `$PROFILE` on PowerShell when those shells are present.

## 6. Per-tool override emitter

`tools.toml` keyed by tool id. Schema (lock for v1):

```toml
[tools.claude_code]
id = "claude_code"
display = "Claude Code CLI"
detect_binaries = ["claude", "claude-code"]
env_vars = ["NODE_EXTRA_CA_CERTS"]

[tools.codex]
id = "codex"
display = "Codex CLI"
detect_binaries = ["codex"]
env_vars = ["CODEX_CA_CERTIFICATE"]

[tools.gemini]
id = "gemini"
display = "Gemini CLI"
detect_binaries = ["gemini"]
env_vars = ["NODE_EXTRA_CA_CERTS"]
legal_gate = "gemini_oauth_refusal"

# ... 11 more
```

The full matrix is the §5 table from `design.md`. `overrides.rs` reads this at runtime and emits only the env vars whose detected binary is on `PATH` (or whose id is in `--include`).

## 7. Routing-table addition

`services/egress_proxy/src/routing.rs` adds an entry for the Gemini API-key path. The Vertex `EncoderKind::Gemini` is already wired (line 254). New entry inserted alphabetically before Vertex:

```rust
// ─── Gemini API key (public generativelanguage.googleapis.com) ───
ProviderConfig {
    kind: ProviderKind::Gemini,          // NEW variant
    inbound_path_pattern: Regex::new(
        r"^/v1beta/models/([^:]+):generateContent$",
    )
    .expect("gemini api key path regex"),
    upstream_url_template:
        "https://generativelanguage.googleapis.com/v1beta/models/{0}:generateContent",
    request_shape: RequestShape::GeminiGenerateContent, // NEW
    tokenizer_kind: EncoderKind::Gemini,
    usage_extractor: providers::gemini::extract_usage,  // NEW thin module
},
```

`ProviderKind::Gemini`, `RequestShape::GeminiGenerateContent`, and `providers::gemini` are additive — no edits to existing rows. `as_str()` returns `"gemini"`.

## 8. Doctor

```rust
pub struct DoctorReport {
    pub ca_present_in_store: bool,
    pub ca_fingerprint_matches: bool,
    pub https_proxy_set: Option<String>,
    pub round_trip_ok: bool,                    // HTTP CONNECT through 127.0.0.1:8443
    pub round_trip_latency_ms: Option<u32>,
    pub per_tool_overrides_present: Vec<(String, bool)>,
    pub gemini_oauth_refusal: Option<String>,
    pub warnings: Vec<String>,
}
```

`doctor` runs a TLS handshake against `127.0.0.1:8443` and asserts the cert chains to the installed CA fingerprint — proves end-to-end trust without depending on any specific CLI being installed.

## 9. Uninstall guarantees

Uninstall is the inverse of install, in reverse order:

1. Remove rc markers.
2. Clear per-tool config-file overrides (Tabnine `caBundle`, Cody `SRC_HTTPS_PROXY`).
3. Remove CA from trust store(s) by fingerprint.
4. Delete on-disk CA / leaf PEM and key blobs (`std::fs::remove_file` + on Linux best-effort `shred -u` when available).

Exit code 0 on full cleanup; exit code 75 (`EX_TEMPFAIL`) when partial cleanup succeeds, with the `InstallReport` listing remaining artifacts.

## 10. Dependencies (versions locked at spec time)

| Crate | Version | Use |
|-------|---------|-----|
| `clap` | `4.5` | CLI |
| `rcgen` | `0.13` | CA + leaf issuance |
| `rustls-pemfile` | `2` | PEM I/O |
| `x509-parser` | `0.16` | fingerprint + store-presence verification |
| `keychain-services` | `0.1` | macOS keychain key storage |
| `windows` | `0.58` (`Win32_Security_Cryptography`) | DPAPI + certutil interop |
| `which` | `7` | binary detection |
| `serde`, `toml`, `time`, `zeroize`, `uuid v1.10` | pinned to workspace versions | misc |

No new MSRV bump: confirmed against the current workspace rust-toolchain pin.
