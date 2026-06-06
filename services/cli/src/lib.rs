//! Public library surface for `spendguard-cli`.
//!
//! Per design §8, the CLI exposes three top-level operations:
//!
//! - [`install`] — CA issuance, leaf issuance, trust-store install, per-tool
//!   env emitter. SLICE 1 (COV_05) implemented CA + leaf gen + on-disk PEM
//!   persistence. **SLICE 2 (COV_06, this slice)** wires the macOS trust
//!   store (`MacosTrustStore`) into `install` / `uninstall` / `doctor` and
//!   populates `InstallReport.trust_store_locations`. Linux trust install
//!   lands in SLICE 3 (COV_07), Windows in SLICE 4 (COV_08).
//! - [`uninstall`] — symmetric removal of trust-store entries +
//!   on-disk PEMs. This slice implements the trust-store half on macOS;
//!   the shell-rc / per-tool overrides half lands in SLICE 5 / SLICE 7.
//! - [`doctor`] — CA-in-store check. This slice implements the trust-store
//!   query on macOS; HTTPS_PROXY reachability + TLS handshake land in
//!   SLICE 7.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub mod ca;
pub mod paths;
pub mod trust;

/// Per-user vs admin trust scope. Parsed in this slice; honoured by SLICE
/// 2/3/4 trust-store backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrustScope {
    #[default]
    User,
    System,
}

/// Shell rc backend selector. Parsed here; honoured by SLICE 5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShellKind {
    Bash,
    Zsh,
    Fish,
    Pwsh,
}

/// `spendguard install` options. Field set is the v1 lock per design §2;
/// later slices populate more sub-reports but the option struct does not
/// grow.
#[derive(Debug, Clone, clap::Args)]
pub struct InstallOpts {
    /// `user` (no sudo) or `system` (admin). Trust-store install honours
    /// this in SLICE 2/3/4.
    #[arg(long, value_enum, default_value_t = TrustScope::User)]
    pub scope: TrustScope,

    /// Directory the four PEM files are written to. If omitted, resolved
    /// from `paths::ca_root_dir()` (OS-specific XDG path).
    #[arg(long, value_name = "DIR")]
    pub ca_out: Option<PathBuf>,

    /// Override shell detection (parsed; rc emission in SLICE 5).
    #[arg(long, value_enum)]
    pub shell: Option<ShellKind>,
}

/// `spendguard uninstall` options.
///
/// SLICE 2 (COV_06) honours `--scope` and `--ca-fingerprint` to drive the
/// macOS trust-store removal. SLICE 7 (COV_11) adds on-disk PEM cleanup
/// (and resolves the fingerprint from the PEM when `--ca-fingerprint` is
/// not supplied), shell-rc strip, and per-tool config cleanup.
#[derive(Debug, Clone, clap::Args)]
pub struct UninstallOpts {
    /// Match the `--scope` used at install time so the symmetric removal
    /// targets the same trust store.
    #[arg(long, value_enum, default_value_t = TrustScope::User)]
    pub scope: TrustScope,

    /// Lower-case SHA-256 hex of the root CA to remove. Required in
    /// SLICE 2 (COV_06); SLICE 7 will derive it from the on-disk PEM
    /// when omitted.
    #[arg(long, value_name = "HEX")]
    pub ca_fingerprint: Option<String>,
}

/// `spendguard doctor` options.
///
/// SLICE 2 (COV_06) reads the supplied CA fingerprint and reports whether
/// it's trusted in the configured keychain scope. SLICE 7 expands this with
/// HTTPS_PROXY reachability + TLS handshake checks.
#[derive(Debug, Clone, clap::Args)]
pub struct DoctorOpts {
    /// Trust-store scope to probe.
    #[arg(long, value_enum, default_value_t = TrustScope::User)]
    pub scope: TrustScope,

    /// CA fingerprint to look for in the trust store. Optional — when
    /// omitted, the report flags `ca_present_in_store = false` and adds a
    /// warning. SLICE 7 will derive from the on-disk PEM.
    #[arg(long, value_name = "HEX")]
    pub ca_fingerprint: Option<String>,
}

/// What `install` produced. Public so SLICE 5 can extend it additively
/// (tools, shell rc paths) without breaking JSON consumers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallReport {
    pub ca_fingerprint_sha256: String,
    pub ca_pem_path: PathBuf,
    pub ca_key_path: PathBuf,
    pub leaf_pem_path: PathBuf,
    pub leaf_key_path: PathBuf,
    /// Empty until SLICE 2/3/4 wires the OS backend; lets `doctor` reason
    /// about "issued but not trusted" in the interim.
    pub trust_store_locations: Vec<PathBuf>,
    /// Empty until SLICE 5.
    pub shell_rc_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UninstallReport {
    pub removed_files: Vec<PathBuf>,
    pub trust_store_locations_cleared: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorReport {
    pub ca_present_in_store: bool,
    pub https_proxy_set: Option<String>,
    pub round_trip_ok: bool,
    pub warnings: Vec<String>,
}

/// `spendguard install` entry point.
///
/// SLICE 2 (COV_06) wires the macOS trust store on top of SLICE 1's CA
/// material:
///
/// 1. Issue a fresh root CA + localhost leaf (SLICE 1).
/// 2. Write all four PEM files via the atomic temp-file + 0o600 rename
///    pipeline (SLICE 1, R2 fix B1).
/// 3. **NEW**: Hand the CA PEM to the OS trust-store backend (macOS only
///    in this slice). Populates `InstallReport.trust_store_locations` so
///    the operator + `doctor` know exactly which keychain was touched.
///
/// Non-macOS callers fall through with an empty `trust_store_locations`
/// for now — SLICE 3 / SLICE 4 fill those in without an API break.
///
/// Internally this is a thin shim over [`install_with_trust_store`], which
/// the unit tests use with a no-op trust backend to avoid mutating the
/// developer's real keychain.
pub fn install(opts: &InstallOpts) -> Result<InstallReport> {
    let backend = trust_backend()?;
    install_with_trust_store(opts, backend.as_ref())
}

/// `spendguard install` with an injected trust backend. Public so the
/// integration test (`tests/trust_macos.rs`) and the eventual SLICE 7
/// driver can compose this without re-implementing the four-PEM emit.
pub fn install_with_trust_store(
    opts: &InstallOpts,
    trust_backend: &dyn trust::TrustStore,
) -> Result<InstallReport> {
    let out_dir = match &opts.ca_out {
        Some(dir) => {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("create --ca-out dir {}", dir.display()))?;
            dir.clone()
        }
        None => paths::ca_root_dir().context("resolve default CA root dir")?,
    };

    let root = ca::generate_root_ca().context("generate root CA")?;
    let leaf = ca::issue_leaf_cert(&root, &["localhost", "127.0.0.1", "::1"])
        .context("issue localhost leaf")?;

    let ca_pem_path = out_dir.join("root_ca.pem");
    let ca_key_path = out_dir.join("root_ca.key.pem");
    let leaf_pem_path = out_dir.join("leaf.pem");
    let leaf_key_path = out_dir.join("leaf.key.pem");

    write_secret(&ca_pem_path, root.cert_pem.as_bytes(), false)?;
    write_secret(&ca_key_path, root.key_pem.as_bytes(), true)?;
    write_secret(&leaf_pem_path, leaf.cert_pem.as_bytes(), false)?;
    write_secret(&leaf_key_path, leaf.key_pem.as_bytes(), true)?;

    let ca_fingerprint_sha256 = ca::fingerprint_hex(&root.fingerprint_sha256);

    // SLICE 2: install the CA into the OS trust store. macOS only in this
    // slice; Linux/Windows fall through with empty trust_store_locations
    // (review-standards X1: cfg-gated, not runtime branched).
    let trust_store_locations = trust_backend
        .add_root(&ca_pem_path, opts.scope)
        .context("install CA into OS trust store")?;

    Ok(InstallReport {
        ca_fingerprint_sha256,
        ca_pem_path,
        ca_key_path,
        leaf_pem_path,
        leaf_key_path,
        trust_store_locations,
        shell_rc_paths: Vec::new(),
    })
}

/// `spendguard uninstall` — SLICE 2 lands the trust-store half (macOS only).
///
/// Per design §9 (Uninstall guarantees) the full inverse is:
///   1. Remove rc markers (SLICE 5 / SLICE 7).
///   2. Clear per-tool config-file overrides (SLICE 5 / SLICE 7).
///   3. **NEW (this slice)**: Remove CA from trust store(s) by fingerprint
///      — macOS only.
///   4. Delete on-disk CA / leaf PEM and key blobs (SLICE 7).
///
/// In this slice we accept the SHA-256 fingerprint via `opts.ca_fingerprint`
/// — the caller is the operator with their install-report JSON to hand. The
/// SLICE 7 implementation will resolve the fingerprint from the on-disk PEM
/// when the operator doesn't have one handy.
pub fn uninstall(opts: &UninstallOpts) -> Result<UninstallReport> {
    let backend = trust_backend()?;
    uninstall_with_trust_store(opts, backend.as_ref())
}

/// `spendguard uninstall` with injected trust backend — see
/// [`install_with_trust_store`] for the rationale.
pub fn uninstall_with_trust_store(
    opts: &UninstallOpts,
    trust_backend: &dyn trust::TrustStore,
) -> Result<UninstallReport> {
    let fingerprint = match &opts.ca_fingerprint {
        Some(fp) => fp.clone(),
        None => {
            // SLICE 7 will read the PEM from `paths::ca_root_dir()` and
            // re-derive. For now require the operator to supply it explicitly.
            anyhow::bail!(
                "uninstall: --ca-fingerprint is required in SLICE 2 (COV_06); \
                 SLICE 7 (COV_11) will re-derive from the on-disk PEM"
            );
        }
    };

    let trust_store_locations_cleared = trust_backend
        .remove_root(&fingerprint, opts.scope)
        .context("remove CA from OS trust store")?;

    Ok(UninstallReport {
        removed_files: Vec::new(),
        trust_store_locations_cleared,
    })
}

/// `spendguard doctor` — SLICE 2 reports whether the CA is trusted in the
/// configured keychain.
///
/// SLICE 7 expands this to also probe `HTTPS_PROXY` and to drive a TLS
/// handshake; for now we provide the deterministic, no-network half: was
/// the CA fingerprint successfully landed in the trust store?
pub fn doctor(opts: &DoctorOpts) -> Result<DoctorReport> {
    let backend = trust_backend()?;
    doctor_with_trust_store(opts, backend.as_ref())
}

/// `spendguard doctor` with injected trust backend.
pub fn doctor_with_trust_store(
    opts: &DoctorOpts,
    trust_backend: &dyn trust::TrustStore,
) -> Result<DoctorReport> {
    let mut warnings = Vec::new();
    let ca_present_in_store = match &opts.ca_fingerprint {
        Some(fp) => trust_backend
            .verify_installed(fp, opts.scope)
            .unwrap_or_else(|e| {
                warnings.push(format!("trust-store probe failed: {e:#}"));
                false
            }),
        None => {
            warnings.push(
                "doctor: --ca-fingerprint not supplied; trust-store probe skipped".to_string(),
            );
            false
        }
    };

    Ok(DoctorReport {
        ca_present_in_store,
        https_proxy_set: std::env::var("HTTPS_PROXY").ok(),
        round_trip_ok: false, // SLICE 7
        warnings,
    })
}

/// Resolve the OS trust-store backend for the production entry points.
/// Pulled out so the `#[cfg]` gate lives in one place — see
/// [`trust::dispatch`] for the per-OS routing.
fn trust_backend() -> Result<Box<dyn trust::TrustStore>> {
    trust::dispatch()
}

/// Write a PEM blob to disk. On POSIX, `secret=true` clamps mode to `0o600`
/// (review-standards.md `T2`); on Windows the OS handles ACLs and DPAPI
/// sealing is added in SLICE 4.
///
/// R2 fix per security review (B1): on POSIX the write is staged through a
/// `.tmp-<uuid>` sibling created with the target mode and *then* atomically
/// renamed over the destination. This:
///   1. Hard-clamps mode at create-time AND verifies it via
///      `set_permissions` immediately before rename, so even if the
///      pre-existing destination file is `0o644` / `0o666` from a prior
///      botched install it is replaced (inode-swap) with one at `0o600`.
///   2. Closes the partial-write window — a crash mid-`write_all` leaves the
///      old file intact, never a half-written secret at the final path.
fn write_secret(path: &std::path::Path, bytes: &[u8], secret: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let mode = if secret { 0o600 } else { 0o644 };
        let tmp_path = tmp_sibling_for(path);

        let mut opts = std::fs::OpenOptions::new();
        opts.create_new(true).write(true).mode(mode);
        let mut f = opts
            .open(&tmp_path)
            .with_context(|| format!("open temp {} for atomic write", tmp_path.display()))?;
        f.write_all(bytes)
            .with_context(|| format!("write temp {}", tmp_path.display()))?;
        f.sync_all()
            .with_context(|| format!("sync temp {}", tmp_path.display()))?;
        // Hard-clamp again: defends against umask, ACL inheritance, or a
        // hostile pre-existing file (B1). `set_permissions` is the only call
        // documented to apply mode unconditionally on the inode.
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(mode))
            .with_context(|| format!("clamp mode on {}", tmp_path.display()))?;
        // Atomic rename: replaces dest inode, dropping any prior mode bits.
        if let Err(e) = std::fs::rename(&tmp_path, path) {
            // Best-effort cleanup so we don't leak the temp file on rename
            // failure. The original `Err` is what the caller sees.
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e)
                .with_context(|| format!("rename {} -> {}", tmp_path.display(), path.display()));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = secret;
        std::fs::write(path, bytes).with_context(|| format!("write {}", path.display()))?;
        Ok(())
    }
}

/// Build a unique sibling temp path for atomic rename. Lives next to the
/// target so the rename is guaranteed to be on the same filesystem
/// (`std::fs::rename` is only atomic intra-filesystem).
#[cfg(unix)]
fn tmp_sibling_for(path: &std::path::Path) -> std::path::PathBuf {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "secret".to_string());
    // Process id + uuid keeps the name unique across concurrent installers
    // sharing a directory; uuid alone would be sufficient but the pid aids
    // debugging.
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let tmp_name = format!(".{file_name}.tmp-{}-{suffix}", std::process::id());
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    parent.join(tmp_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Mutex;

    /// Inert trust store used by lib-tests that exercise the install path
    /// without touching the developer's real keychain. SLICE 2 (COV_06) added
    /// this for the `install_with_trust_store` shim so unit tests can stay
    /// hermetic on macOS hosts — the real keychain integration test lives in
    /// `services/cli/tests/trust_macos.rs` and is `#[ignore]`-gated.
    #[derive(Debug, Default)]
    struct NoopTrustStore {
        added: Mutex<Vec<PathBuf>>,
        removed: Mutex<Vec<String>>,
    }

    impl trust::TrustStore for NoopTrustStore {
        fn add_root(&self, ca_pem_path: &Path, _scope: TrustScope) -> Result<Vec<PathBuf>> {
            self.added.lock().unwrap().push(ca_pem_path.to_path_buf());
            Ok(vec![PathBuf::from("/tmp/noop-trust-store")])
        }
        fn remove_root(
            &self,
            fingerprint_sha256_hex: &str,
            _scope: TrustScope,
        ) -> Result<Vec<PathBuf>> {
            self.removed
                .lock()
                .unwrap()
                .push(fingerprint_sha256_hex.to_string());
            Ok(vec![PathBuf::from("/tmp/noop-trust-store")])
        }
        fn verify_installed(
            &self,
            _fingerprint_sha256_hex: &str,
            _scope: TrustScope,
        ) -> Result<bool> {
            Ok(true)
        }
    }

    /// End-to-end: `install_with_trust_store --ca-out <tmp>` writes 4 PEM
    /// files, all parseable. Uses NoopTrustStore so the developer's real
    /// keychain stays untouched.
    #[test]
    fn install_writes_four_pem_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let opts = InstallOpts {
            scope: TrustScope::User,
            ca_out: Some(tmp.path().to_path_buf()),
            shell: None,
        };
        let backend = NoopTrustStore::default();
        let report = install_with_trust_store(&opts, &backend).expect("install");

        for p in [
            &report.ca_pem_path,
            &report.ca_key_path,
            &report.leaf_pem_path,
            &report.leaf_key_path,
        ] {
            assert!(p.exists(), "{p:?} not written");
            let content = std::fs::read_to_string(p).expect("read");
            assert!(content.contains("BEGIN"), "{p:?} not PEM");
        }

        assert_eq!(
            report.ca_fingerprint_sha256.len(),
            64,
            "sha256 hex must be 64 chars, got {}",
            report.ca_fingerprint_sha256
        );
        // SLICE 2: trust_store_locations is populated by the (noop here)
        // backend's add_root return value.
        assert_eq!(
            report.trust_store_locations,
            vec![PathBuf::from("/tmp/noop-trust-store")],
            "install_with_trust_store should surface the backend's return value"
        );
        assert!(report.shell_rc_paths.is_empty());
    }

    /// SLICE 2: `install_with_trust_store` calls `add_root` exactly once with
    /// the CA PEM path it wrote.
    #[test]
    fn install_invokes_trust_backend_add_root() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let opts = InstallOpts {
            scope: TrustScope::User,
            ca_out: Some(tmp.path().to_path_buf()),
            shell: None,
        };
        let backend = NoopTrustStore::default();
        let report = install_with_trust_store(&opts, &backend).expect("install");

        let added = backend.added.lock().unwrap().clone();
        assert_eq!(added.len(), 1, "add_root called exactly once");
        assert_eq!(added[0], report.ca_pem_path);
    }

    /// SLICE 2: `uninstall_with_trust_store` requires `--ca-fingerprint`.
    #[test]
    fn uninstall_requires_ca_fingerprint_in_slice_2() {
        let backend = NoopTrustStore::default();
        let opts = UninstallOpts {
            scope: TrustScope::User,
            ca_fingerprint: None,
        };
        let err = uninstall_with_trust_store(&opts, &backend)
            .expect_err("must require fingerprint until SLICE 7");
        assert!(format!("{err:#}").contains("ca-fingerprint"));
    }

    /// SLICE 2: `uninstall_with_trust_store` forwards the fingerprint to the
    /// backend and surfaces the cleared locations.
    #[test]
    fn uninstall_invokes_backend_remove_root() {
        let backend = NoopTrustStore::default();
        let opts = UninstallOpts {
            scope: TrustScope::User,
            ca_fingerprint: Some(
                "0000000000000000000000000000000000000000000000000000000000000000".into(),
            ),
        };
        let report = uninstall_with_trust_store(&opts, &backend).expect("uninstall");
        assert_eq!(
            report.trust_store_locations_cleared,
            vec![PathBuf::from("/tmp/noop-trust-store")]
        );
        let removed = backend.removed.lock().unwrap().clone();
        assert_eq!(removed.len(), 1);
        assert_eq!(
            removed[0],
            "0000000000000000000000000000000000000000000000000000000000000000"
        );
    }

    /// SLICE 2: `doctor_with_trust_store` reports the trust-store probe
    /// result and surfaces a warning when no fingerprint is supplied.
    #[test]
    fn doctor_without_fingerprint_warns_and_reports_false() {
        let backend = NoopTrustStore::default();
        let opts = DoctorOpts {
            scope: TrustScope::User,
            ca_fingerprint: None,
        };
        let report = doctor_with_trust_store(&opts, &backend).expect("doctor");
        assert!(!report.ca_present_in_store);
        assert!(report
            .warnings
            .iter()
            .any(|w| w.contains("ca-fingerprint not supplied")));
    }

    #[test]
    fn doctor_with_fingerprint_returns_backend_probe() {
        let backend = NoopTrustStore::default();
        let opts = DoctorOpts {
            scope: TrustScope::User,
            ca_fingerprint: Some(
                "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".into(),
            ),
        };
        let report = doctor_with_trust_store(&opts, &backend).expect("doctor");
        assert!(
            report.ca_present_in_store,
            "NoopTrustStore.verify_installed returns true"
        );
        assert!(report
            .warnings
            .iter()
            .all(|w| !w.contains("ca-fingerprint not supplied")));
    }

    /// `T2` — private key files are mode 0o600.
    #[cfg(unix)]
    #[test]
    fn private_key_files_are_mode_0o600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().expect("tempdir");
        let opts = InstallOpts {
            scope: TrustScope::User,
            ca_out: Some(tmp.path().to_path_buf()),
            shell: None,
        };
        let backend = NoopTrustStore::default();
        let report = install_with_trust_store(&opts, &backend).expect("install");

        for key_path in [&report.ca_key_path, &report.leaf_key_path] {
            let meta = std::fs::metadata(key_path).expect("metadata");
            let mode = meta.permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{key_path:?} mode is {mode:o}, expected 0600");
        }
    }

    /// R2 regression for B1 — pre-existing key files at world-readable modes
    /// MUST be replaced with 0o600 across a re-install. The original failure
    /// mode was `OpenOptions::mode()` only applying at create-time, so a
    /// truncate-and-rewrite kept the old 0o644 / 0o666 mode bits.
    #[cfg(unix)]
    #[test]
    fn reinstall_clamps_world_readable_key_files_to_0o600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().expect("tempdir");
        let ca_key = tmp.path().join("root_ca.key.pem");
        let leaf_key = tmp.path().join("leaf.key.pem");

        // Pre-seed both key paths at world-readable modes — the bug under
        // test is that the rewrite preserved these bits.
        std::fs::write(&ca_key, b"stale-root-key-from-prior-botched-install")
            .expect("seed stale ca key");
        std::fs::write(&leaf_key, b"stale-leaf-key-from-prior-botched-install")
            .expect("seed stale leaf key");
        std::fs::set_permissions(&ca_key, std::fs::Permissions::from_mode(0o644))
            .expect("seed ca key at 0o644");
        std::fs::set_permissions(&leaf_key, std::fs::Permissions::from_mode(0o666))
            .expect("seed leaf key at 0o666");

        let opts = InstallOpts {
            scope: TrustScope::User,
            ca_out: Some(tmp.path().to_path_buf()),
            shell: None,
        };
        let backend = NoopTrustStore::default();
        let report =
            install_with_trust_store(&opts, &backend).expect("install over pre-existing key files");

        for key_path in [&report.ca_key_path, &report.leaf_key_path] {
            let meta = std::fs::metadata(key_path).expect("metadata");
            let mode = meta.permissions().mode() & 0o777;
            assert_eq!(
                mode, 0o600,
                "{key_path:?} re-install left mode {mode:o}, expected 0600"
            );
            // Sanity: content was replaced, not just permissioned.
            let content = std::fs::read_to_string(key_path).expect("read");
            assert!(
                content.contains("BEGIN") && content.contains("PRIVATE KEY"),
                "{key_path:?} content should be the freshly issued PEM"
            );
        }

        // The temp-sibling pattern leaves no residue on the happy path.
        let residue: Vec<_> = std::fs::read_dir(tmp.path())
            .expect("readdir")
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name();
                let s = name.to_string_lossy();
                s.starts_with('.') && s.contains(".tmp-")
            })
            .collect();
        assert!(
            residue.is_empty(),
            "atomic-rename temp files leaked: {residue:?}"
        );
    }
}
