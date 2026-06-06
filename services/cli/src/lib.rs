//! Public library surface for `spendguard-cli`.
//!
//! Per design §8, the CLI exposes three top-level operations:
//!
//! - [`install`] — CA issuance, leaf issuance, trust-store install, per-tool
//!   env emitter. This slice (COV_05) implements CA + leaf gen + on-disk
//!   PEM persistence; trust-store install is sniped to SLICE 2/3/4 and
//!   per-tool env emission to SLICE 5.
//! - [`uninstall`] — symmetric removal of all install artifacts. Full impl
//!   in SLICE 7.
//! - [`doctor`] — CA-in-store + HTTPS_PROXY + TLS handshake. SLICE 7.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub mod ca;
pub mod paths;

/// Per-user vs admin trust scope. Parsed in this slice; honoured by SLICE
/// 2/3/4 trust-store backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrustScope {
    User,
    System,
}

impl Default for TrustScope {
    fn default() -> Self {
        Self::User
    }
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

/// `spendguard uninstall` options. Field set defined here so the CLI
/// scaffold compiles; behaviour is SLICE 7.
#[derive(Debug, Clone, clap::Args)]
pub struct UninstallOpts {
    /// Match the `--scope` used at install time so the symmetric removal
    /// targets the same trust store.
    #[arg(long, value_enum, default_value_t = TrustScope::User)]
    pub scope: TrustScope,
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

/// `spendguard install` entry point. In SLICE 1 the only honoured option is
/// `--ca-out`; trust-store and shell-rc fields are populated as empty Vecs
/// so the downstream slices can replace them without an API break.
pub fn install(opts: &InstallOpts) -> Result<InstallReport> {
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

    Ok(InstallReport {
        ca_fingerprint_sha256: ca::fingerprint_hex(&root.fingerprint_sha256),
        ca_pem_path,
        ca_key_path,
        leaf_pem_path,
        leaf_key_path,
        trust_store_locations: Vec::new(),
        shell_rc_paths: Vec::new(),
    })
}

/// `spendguard uninstall` — stub for SLICE 7.
pub fn uninstall(_opts: &UninstallOpts) -> Result<UninstallReport> {
    // Full impl is the inverse of install in reverse order (design §9).
    // SLICE 1 wires the scaffold so the CLI surface is stable.
    anyhow::bail!("spendguard uninstall is implemented in SLICE 7 (COV_11)")
}

/// `spendguard doctor` — stub for SLICE 7.
pub fn doctor() -> Result<DoctorReport> {
    anyhow::bail!("spendguard doctor is implemented in SLICE 7 (COV_11)")
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

    /// End-to-end: `install --ca-out <tmp>` writes 4 PEM files, all parseable.
    #[test]
    fn install_writes_four_pem_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let opts = InstallOpts {
            scope: TrustScope::User,
            ca_out: Some(tmp.path().to_path_buf()),
            shell: None,
        };
        let report = install(&opts).expect("install");

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
        assert!(report.trust_store_locations.is_empty());
        assert!(report.shell_rc_paths.is_empty());
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
        let report = install(&opts).expect("install");

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
        let report = install(&opts).expect("install over pre-existing key files");

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
