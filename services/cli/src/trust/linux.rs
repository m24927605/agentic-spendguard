//! Linux multi-distro trust-store backend (SLICE 3 / COV_07).
//!
//! Linux does not have a single trust-store interface. Each major distro
//! family ships its own tool:
//!
//! | Distro family | Anchor dir                                  | Update tool                       |
//! |---------------|---------------------------------------------|-----------------------------------|
//! | Debian/Ubuntu | `/usr/local/share/ca-certificates/`         | `update-ca-certificates --verbose`|
//! | RHEL/Fedora   | `/etc/pki/ca-trust/source/anchors/`         | `update-ca-trust extract`         |
//! | Arch          | (p11-kit-managed; no anchor dir to write)   | `trust anchor <pem>`              |
//! | Alpine        | `/usr/local/share/ca-certificates/`         | `update-ca-certificates --verbose`|
//!
//! We pick a family by reading `/etc/os-release` `ID` + `ID_LIKE` per design
//! `§4` and review-standards `X2` (the strategy memo banned `lsb_release`
//! because minimal containers don't ship it). The dispatcher in
//! [`crate::trust::dispatch`] runs detection at construction time; the
//! detected family is captured into [`LinuxTrustStore::distro`] so every
//! operation gets the same answer.
//!
//! ## `Unknown` distro: fail-closed
//!
//! When detection yields [`LinuxDistro::Unknown`] (no `/etc/os-release`, or
//! an `ID` we don't recognise, with no matching `ID_LIKE`) every operation
//! errors with a "no Linux trust backend matches" message. This is the
//! review-standards `X1` fail-closed contract — we never silently skip a
//! trust-store mutation just because we couldn't identify the host.
//!
//! ## Why a `CommandRunner` indirection (T8 / hermetic tests)
//!
//! Same pattern as `macos.rs`: every shell-out routes through
//! [`crate::trust::CommandRunner`] so the unit tests below — which are
//! deliberately OS-independent — exercise the production argv shape with a
//! [`FakeRunner`] without touching `/usr/local/share/ca-certificates/` or
//! invoking `sudo update-ca-certificates`. Real CA install/uninstall is
//! exercised in the `#[ignore]`-gated `tests/trust_linux.rs` integration
//! tests, which the per-distro CI matrix (`tests.md` §8) opts in to.
//!
//! ## T6 (no silent escalation) and the user/system scope split
//!
//! `update-ca-certificates`, `update-ca-trust`, and `trust anchor` all
//! mutate system trust state under `/etc/` or `/usr/local/`. They require
//! root. Per design `§4 last bullet` we never `sudo` from the binary
//! itself: if `--scope system` is requested and we're not root, the
//! `update-*` shell-out returns an `EACCES`-style failure that we surface
//! verbatim, exactly as the macOS backend does for non-`sudo`
//! `--scope system`.
//!
//! **R2 fix (Blocker B1):** `--scope user` on Linux now **fails closed**
//! at the `TrustStore` boundary. The previous implementation staged the
//! PEM at `~/.local/share/ca-certificates/spendguard.crt` then shelled
//! out to `update-ca-certificates --verbose`, but that tool reads ONLY
//! `/usr/local/share/ca-certificates/`, so the staged file is invisible
//! to every consumer. Worse, when invoked as root the same shell-out
//! silently rebuilds the SYSTEM bundle — a privilege escalation from a
//! `--scope user` request (T6 violation). The new contract: `add_root`
//! / `remove_root` return an error containing the `CURL_CA_BUNDLE`
//! token; SLICE 7 (COV_11) doctor greps for that token and prints the
//! actionable env-var redirect (`export CURL_CA_BUNDLE=… ;
//! export SSL_CERT_FILE=…`). `verify_installed` still inspects the
//! per-user anchor path (read-only, no shell-out) so a doctor probe can
//! report "absent" rather than erroring.
//!
//! ## Idempotence
//!
//! - `add_root` overwrites the anchor file in place, then runs the
//!   distro's update tool. Both legs are idempotent on the distro side.
//! - `remove_root` removes the anchor file if present (no-op otherwise),
//!   then re-runs the update tool to flush the bundle (review-standards
//!   `X3` / acceptance `A8.2`).
//! - `verify_installed` greps `/etc/ssl/certs/ca-certificates.crt`
//!   (Debian/Alpine), `/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem`
//!   (RHEL), or `trust list --filter=ca-anchors` (Arch) for the SHA-256
//!   fingerprint and returns `Ok(false)` when the cert is simply absent.

use crate::trust::{CommandOutput, CommandRunner, StdCommandRunner, TrustStore};
use crate::TrustScope;
use anyhow::{anyhow, Context, Result};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Linux family classification — drives the distro-specific argv shape.
///
/// The variants are intentionally narrow: we ship exactly the four
/// families listed in `design.md §4`, plus an explicit [`Self::Unknown`]
/// that fails closed. Adding e.g. Gentoo or NixOS later means a new
/// variant — review-standards `X1` requires that every per-OS branch is
/// `#[cfg(target_os = …)]`-gated AND distro-typed, not runtime-detected
/// from `env::consts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxDistro {
    Debian,
    Rhel,
    Arch,
    Alpine,
    Unknown,
}

impl LinuxDistro {
    /// Human-readable identifier used in error messages so the operator
    /// can tell which family we detected. The strings are stable so
    /// downstream tooling (the doctor report, the install log) can match
    /// on them.
    pub fn as_str(self) -> &'static str {
        match self {
            LinuxDistro::Debian => "debian",
            LinuxDistro::Rhel => "rhel",
            LinuxDistro::Arch => "arch",
            LinuxDistro::Alpine => "alpine",
            LinuxDistro::Unknown => "unknown",
        }
    }
}

/// Default path to `os-release`. Resolved at call time rather than baked
/// into a `const` so the tests below can point it at a tempfile.
const ETC_OS_RELEASE: &str = "/etc/os-release";

/// Detect the distro family from the textual contents of `/etc/os-release`.
///
/// The file is shell-style `KEY=VALUE` with optional quoting. We read `ID`
/// (the canonical id) first; if that doesn't match a known family we
/// search `ID_LIKE` (space-separated list) for a match. Both keys are
/// case-insensitive at the value level — `ID="Debian"` parses the same as
/// `ID=debian`. This matches `os-release(5)` and the way `systemd`'s own
/// detection works.
///
/// Returns [`LinuxDistro::Unknown`] when neither `ID` nor any `ID_LIKE`
/// token matches a known family. Callers should treat `Unknown` as
/// fail-closed.
pub fn detect_distro_from(contents: &str) -> LinuxDistro {
    let mut id: Option<String> = None;
    let mut id_like: Option<String> = None;
    for raw in contents.lines() {
        // Skip blank lines and shell-style comments.
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("ID=") {
            id = Some(strip_shell_quotes(rest).to_ascii_lowercase());
        } else if let Some(rest) = line.strip_prefix("ID_LIKE=") {
            id_like = Some(strip_shell_quotes(rest).to_ascii_lowercase());
        }
    }

    if let Some(id) = &id {
        if let Some(family) = family_for_token(id) {
            return family;
        }
    }
    if let Some(like) = &id_like {
        for token in like.split_whitespace() {
            if let Some(family) = family_for_token(token) {
                return family;
            }
        }
    }
    LinuxDistro::Unknown
}

/// Strip one wrapping pair of single or double quotes — `os-release(5)` allows
/// both forms. No escape-sequence handling because the IDs we care about are
/// all ASCII alphanumerics.
fn strip_shell_quotes(value: &str) -> String {
    let trimmed = value.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

/// Map a single canonical token (lowercase) onto its distro family. Returns
/// `None` for tokens we don't recognise so the caller can fall back to
/// `ID_LIKE`. The lookup is intentionally exhaustive on the "ship-and-test"
/// list; new families need an explicit variant + token.
fn family_for_token(token: &str) -> Option<LinuxDistro> {
    match token {
        // Debian family: debian itself, plus its downstream tokens.
        "debian" | "ubuntu" | "linuxmint" | "pop" | "raspbian" | "kali" => {
            Some(LinuxDistro::Debian)
        }
        // RHEL family: rhel, fedora, plus the downstream rebuilds.
        "rhel" | "fedora" | "centos" | "rocky" | "almalinux" | "ol" | "amzn" => {
            Some(LinuxDistro::Rhel)
        }
        "arch" | "manjaro" | "endeavouros" => Some(LinuxDistro::Arch),
        "alpine" => Some(LinuxDistro::Alpine),
        _ => None,
    }
}

/// Read `/etc/os-release` (or a test-supplied path) and dispatch to
/// [`detect_distro_from`]. Returns [`LinuxDistro::Unknown`] if the file
/// can't be read — same fail-closed treatment as an unrecognised `ID`.
fn detect_distro_at(path: &Path) -> LinuxDistro {
    match std::fs::read_to_string(path) {
        Ok(contents) => detect_distro_from(&contents),
        Err(e) => {
            tracing::debug!(
                path = %path.display(),
                error = %e,
                "os-release read failed; treating as Unknown"
            );
            LinuxDistro::Unknown
        }
    }
}

/// Path the anchor file is written to for a given distro + scope.
///
/// For `--scope system` we use the canonical system-trust dir per design
/// §4. For `--scope user` we fall back to `~/.local/share/ca-certificates/`
/// — note the operator must still feed this to client libraries (typically
/// via `CURL_CA_BUNDLE`); the warning lives in the doctor report (SLICE 7).
fn anchor_path_for(distro: LinuxDistro, scope: TrustScope) -> Result<PathBuf> {
    match scope {
        TrustScope::System => Ok(match distro {
            LinuxDistro::Debian | LinuxDistro::Alpine => {
                PathBuf::from("/usr/local/share/ca-certificates/spendguard.crt")
            }
            LinuxDistro::Rhel => PathBuf::from("/etc/pki/ca-trust/source/anchors/spendguard.crt"),
            // Arch / p11-kit ingests the cert via `trust anchor` directly;
            // there's no dropped-in anchor file. We still return a path so
            // `add_root` has something to write to before the `trust
            // anchor` invocation (the tool reads the PEM by path).
            LinuxDistro::Arch => {
                PathBuf::from("/usr/share/ca-certificates/trust-source/spendguard.p11-kit")
            }
            LinuxDistro::Unknown => {
                return Err(anyhow!(
                    "no Linux trust backend matches this host's /etc/os-release; \
                     refusing to mutate system trust (T6 fail-closed)"
                ))
            }
        }),
        TrustScope::User => {
            let home = std::env::var_os("HOME").ok_or_else(|| {
                anyhow!("HOME not set; cannot resolve ~/.local/share/ca-certificates/")
            })?;
            // `~/.local/share/ca-certificates/` is the per-user mirror of
            // `/usr/local/share/ca-certificates/`. None of the system
            // update tools read from it; the operator wires it through
            // `CURL_CA_BUNDLE` / `SSL_CERT_FILE` (per design §4 last
            // bullet + the SLICE 5 shell-rc emitter).
            Ok(PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("ca-certificates")
                .join("spendguard.crt"))
        }
    }
}

/// Where to look for the cert in the post-update merged bundle (system
/// scope) so [`LinuxTrustStore::verify_installed`] can grep for the
/// fingerprint without re-reading the anchor file.
fn merged_bundle_path_for(distro: LinuxDistro) -> Option<PathBuf> {
    match distro {
        LinuxDistro::Debian | LinuxDistro::Alpine => {
            Some(PathBuf::from("/etc/ssl/certs/ca-certificates.crt"))
        }
        LinuxDistro::Rhel => Some(PathBuf::from(
            "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem",
        )),
        // Arch uses p11-kit; verify via `trust list`, not a merged file.
        LinuxDistro::Arch => None,
        LinuxDistro::Unknown => None,
    }
}

/// Linux trust-store backend. The detected [`LinuxDistro`] is captured at
/// construction time so every operation routes consistently — a
/// late-arriving `/etc/os-release` change between `add_root` and
/// `verify_installed` cannot accidentally drive us across families.
pub struct LinuxTrustStore {
    runner: Box<dyn CommandRunner>,
    distro: LinuxDistro,
    /// Optional override for the anchor file path. Tests inject a
    /// tempdir-rooted path here so they can drive `add_root` / `remove_root`
    /// without writing to `/usr/local/share/ca-certificates/`. `None` at
    /// runtime → defaults from [`anchor_path_for`].
    anchor_override: Option<PathBuf>,
    /// Optional override for the merged-bundle grep target. Same
    /// motivation as `anchor_override`: tests point at a tempfile.
    merged_bundle_override: Option<PathBuf>,
}

impl LinuxTrustStore {
    /// Production constructor: reads `/etc/os-release` at call time and
    /// invokes the real system tools through [`StdCommandRunner`].
    pub fn new() -> Self {
        Self::with_distro(detect_distro_at(Path::new(ETC_OS_RELEASE)))
    }

    /// Construct with an already-detected distro. Exposed for tests and
    /// for `trust::dispatch()` which wants to surface the detected family
    /// in error messages.
    pub fn with_distro(distro: LinuxDistro) -> Self {
        Self {
            runner: Box::new(StdCommandRunner),
            distro,
            anchor_override: None,
            merged_bundle_override: None,
        }
    }

    /// Test-only constructor — same shape as `MacosTrustStore::with_runner`.
    /// Hidden from rustdoc; visible enough that `tests/trust_linux.rs` can
    /// build a store targeting a per-test tempdir.
    #[doc(hidden)]
    pub fn with_runner<R: CommandRunner + 'static>(
        runner: R,
        distro: LinuxDistro,
        anchor_override: Option<PathBuf>,
        merged_bundle_override: Option<PathBuf>,
    ) -> Self {
        Self {
            runner: Box::new(runner),
            distro,
            anchor_override,
            merged_bundle_override,
        }
    }

    /// The detected family — exposed read-only so the dispatcher /
    /// doctor report can surface it.
    pub fn distro(&self) -> LinuxDistro {
        self.distro
    }

    fn anchor_path(&self, scope: TrustScope) -> Result<PathBuf> {
        if let Some(p) = &self.anchor_override {
            return Ok(p.clone());
        }
        anchor_path_for(self.distro, scope)
    }

    fn merged_bundle(&self) -> Option<PathBuf> {
        if let Some(p) = &self.merged_bundle_override {
            return Some(p.clone());
        }
        merged_bundle_path_for(self.distro)
    }

    fn run(&self, program: &str, args: Vec<OsString>) -> Result<CommandOutput> {
        self.runner.run(program, &args)
    }
}

impl Default for LinuxTrustStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TrustStore for LinuxTrustStore {
    fn add_root(&self, ca_pem_path: &Path, scope: TrustScope) -> Result<Vec<PathBuf>> {
        if self.distro == LinuxDistro::Unknown {
            return Err(anyhow!(
                "no Linux trust backend matches this host's /etc/os-release; \
                 refusing to install root CA (T6 fail-closed)"
            ));
        }

        // R2 Blocker B1: `--scope user` on Linux has no in-tree write path
        // that any consumer actually reads. The system update tools
        // (`update-ca-certificates`, `update-ca-trust`, `trust anchor`) ALL
        // refuse to read from `~/.local/share/ca-certificates/`, and worse:
        // when invoked as root they silently rebuild the SYSTEM bundle —
        // a privilege escalation from a `--scope user` request. Fail closed
        // here and steer the operator at the env-var redirect the SLICE 7
        // doctor will surface. The "CURL_CA_BUNDLE" token in the message is
        // what the SLICE 7 doctor greps for to print the actionable hint.
        if scope == TrustScope::User {
            return Err(anyhow!(
                "Linux user-scope trust requires CURL_CA_BUNDLE / SSL_CERT_FILE; \
                 see SLICE 7 doctor (T6 fail-closed: refusing to stage an anchor \
                 no system tool reads, and refusing to escalate to system trust)"
            ));
        }

        let anchor = self.anchor_path(scope)?;

        // Distro-dispatch the update step. Each family runs a single,
        // positional-argv shell-out (T8). The exact argv shape matches
        // design §4 and is what the integration tests below assert
        // verbatim.
        //
        // R2 minor m1: `copy_pem_into` only fires on the distros whose
        // update tool actually reads from `anchor`. Arch / p11-kit reads
        // the PEM directly via the `--store <ca_pem_path>` argv (see the
        // `Arch` arm below) so staging a copy in the p11-kit anchor dir
        // would be dead I/O — a phantom file `trust anchor` ignores.
        match self.distro {
            LinuxDistro::Debian | LinuxDistro::Alpine => {
                copy_pem_into(ca_pem_path, &anchor)
                    .with_context(|| format!("stage CA PEM at {}", anchor.display()))?;
                let out = self
                    .run("update-ca-certificates", vec![OsString::from("--verbose")])
                    .context("invoke update-ca-certificates")?;
                if !out.success() {
                    return Err(anyhow!(
                        "update-ca-certificates exited {:?}: stderr={}",
                        out.status,
                        out.stderr_str().trim()
                    ));
                }
            }
            LinuxDistro::Rhel => {
                copy_pem_into(ca_pem_path, &anchor)
                    .with_context(|| format!("stage CA PEM at {}", anchor.display()))?;
                let out = self
                    .run("update-ca-trust", vec![OsString::from("extract")])
                    .context("invoke update-ca-trust extract")?;
                if !out.success() {
                    return Err(anyhow!(
                        "update-ca-trust exited {:?}: stderr={}",
                        out.status,
                        out.stderr_str().trim()
                    ));
                }
            }
            LinuxDistro::Arch => {
                // `trust anchor --store <ca_pem_path>` (p11-kit) reads the
                // PEM by the path we pass on argv — NOT from the anchor
                // dir under `/usr/share/ca-certificates/trust-source/`.
                // Skip `copy_pem_into` here: it would create a phantom
                // file that no consumer reads.
                let out = self
                    .run(
                        "trust",
                        vec![
                            OsString::from("anchor"),
                            OsString::from("--store"),
                            ca_pem_path.as_os_str().to_owned(),
                        ],
                    )
                    .context("invoke trust anchor")?;
                if !out.success() {
                    return Err(anyhow!(
                        "trust anchor exited {:?}: stderr={}",
                        out.status,
                        out.stderr_str().trim()
                    ));
                }
            }
            LinuxDistro::Unknown => unreachable!("already returned above"),
        }

        Ok(vec![anchor])
    }

    fn remove_root(&self, fingerprint_sha256_hex: &str, scope: TrustScope) -> Result<Vec<PathBuf>> {
        validate_hex_fingerprint(fingerprint_sha256_hex)?;
        if self.distro == LinuxDistro::Unknown {
            return Err(anyhow!(
                "no Linux trust backend matches this host's /etc/os-release; \
                 refusing to mutate trust store"
            ));
        }

        // R2 Blocker B1 (symmetric with `add_root`): user-scope on Linux
        // has no in-tree write path that any system tool reads, so there
        // is nothing for `remove_root` to do here — and shelling out to
        // `update-ca-certificates --fresh` as root would silently rebuild
        // the SYSTEM bundle (privilege escalation user→system). Fail
        // closed for symmetry: the SLICE 7 doctor steers the operator at
        // CURL_CA_BUNDLE / SSL_CERT_FILE.
        if scope == TrustScope::User {
            return Err(anyhow!(
                "Linux user-scope trust requires CURL_CA_BUNDLE / SSL_CERT_FILE; \
                 see SLICE 7 doctor (T6 fail-closed: refusing to escalate to system trust)"
            ));
        }

        let anchor = self.anchor_path(scope)?;
        // Idempotent: if the anchor file is absent, we still want to flush
        // the update tool in case a previous partial install left bundle
        // residue. But to satisfy review-standards X3 / A8.2, when neither
        // the anchor file NOR the merged bundle contains the fingerprint we
        // short-circuit cleanly.
        let anchor_present = anchor.exists();
        let already_absent =
            !anchor_present && !self.fingerprint_in_merged_bundle(fingerprint_sha256_hex)?;
        if already_absent {
            return Ok(vec![]);
        }

        if anchor_present {
            std::fs::remove_file(&anchor)
                .with_context(|| format!("remove anchor file {}", anchor.display()))?;
        }

        match self.distro {
            LinuxDistro::Debian | LinuxDistro::Alpine => {
                // `update-ca-certificates --fresh` rebuilds the bundle from
                // scratch, dropping certs whose anchor file is gone. Without
                // `--fresh`, the cert lingers in `/etc/ssl/certs/ca-certificates.crt`
                // (verified against debian:12 + ubuntu:24.04).
                let out = self
                    .run(
                        "update-ca-certificates",
                        vec![OsString::from("--fresh"), OsString::from("--verbose")],
                    )
                    .context("invoke update-ca-certificates --fresh")?;
                if !out.success() {
                    return Err(anyhow!(
                        "update-ca-certificates --fresh exited {:?}: stderr={}",
                        out.status,
                        out.stderr_str().trim()
                    ));
                }
            }
            LinuxDistro::Rhel => {
                let out = self
                    .run("update-ca-trust", vec![OsString::from("extract")])
                    .context("invoke update-ca-trust extract (uninstall)")?;
                if !out.success() {
                    return Err(anyhow!(
                        "update-ca-trust exited {:?}: stderr={}",
                        out.status,
                        out.stderr_str().trim()
                    ));
                }
            }
            LinuxDistro::Arch => {
                // p11-kit has no "remove by fingerprint" sub-command; the
                // documented uninstall path is `trust anchor --remove
                // <pem>`. We feed it the staged anchor PEM iff present;
                // when absent the cert is already gone and we no-op.
                if anchor_present {
                    let out = self
                        .run(
                            "trust",
                            vec![
                                OsString::from("anchor"),
                                OsString::from("--remove"),
                                anchor.clone().into_os_string(),
                            ],
                        )
                        .context("invoke trust anchor --remove")?;
                    if !out.success() {
                        return Err(anyhow!(
                            "trust anchor --remove exited {:?}: stderr={}",
                            out.status,
                            out.stderr_str().trim()
                        ));
                    }
                }
            }
            LinuxDistro::Unknown => unreachable!("already returned above"),
        }

        Ok(vec![anchor])
    }

    fn verify_installed(&self, fingerprint_sha256_hex: &str, scope: TrustScope) -> Result<bool> {
        validate_hex_fingerprint(fingerprint_sha256_hex)?;
        if self.distro == LinuxDistro::Unknown {
            // Unknown distro is fail-closed for mutations; for verify we
            // return Ok(false) so the doctor report can surface "not in
            // store" without erroring (review-standards X3 says verify
            // never errors on absence; the Unknown case is just "absent
            // because we couldn't look").
            return Ok(false);
        }

        // First check the anchor file path: we own it, and if it's still
        // there the cert is at least staged for install.
        if let Ok(anchor) = self.anchor_path(scope) {
            if anchor.exists() && file_contains_fingerprint(&anchor, fingerprint_sha256_hex)? {
                return Ok(true);
            }
        }

        // Then check the merged bundle (Debian/RHEL/Alpine) or `trust
        // list` (Arch). Either confirms the cert is in the system trust
        // path after `update-*` has run.
        if self.fingerprint_in_merged_bundle(fingerprint_sha256_hex)? {
            return Ok(true);
        }

        if self.distro == LinuxDistro::Arch {
            // `trust list --filter=ca-anchors` enumerates p11-kit anchors;
            // each entry includes a `pubkey-sha256` hex. We grep for the
            // CA fingerprint instead of pubkey because we'd need to
            // re-parse the PEM to get the pubkey hash, and the `trust`
            // CLI does not print the cert SHA-256 directly. Belt-and-
            // braces: also accept a substring hit anywhere in the output.
            let out = self
                .run(
                    "trust",
                    vec![
                        OsString::from("list"),
                        OsString::from("--filter=ca-anchors"),
                    ],
                )
                .context("invoke trust list")?;
            if out.success()
                && haystack_contains_fingerprint(&out.stdout_str(), fingerprint_sha256_hex)
            {
                return Ok(true);
            }
        }

        Ok(false)
    }
}

impl LinuxTrustStore {
    /// Grep the distro-specific merged bundle for the fingerprint. Returns
    /// `Ok(false)` when the bundle file doesn't exist (cert is absent) and
    /// only errors on actual I/O failures.
    fn fingerprint_in_merged_bundle(&self, fingerprint_sha256_hex: &str) -> Result<bool> {
        let bundle = match self.merged_bundle() {
            Some(p) => p,
            None => return Ok(false),
        };
        if !bundle.exists() {
            return Ok(false);
        }
        file_contains_fingerprint(&bundle, fingerprint_sha256_hex)
    }
}

/// Copy the staged CA PEM into the distro's anchor path, creating the
/// parent dir if missing. We deliberately use `std::fs::copy` rather than
/// `std::fs::rename` because the source might be on a different mount
/// (e.g. `$XDG_DATA_HOME` on tmpfs vs `/usr/local/share/...` on the root fs)
/// and `rename` across mounts is not portable.
fn copy_pem_into(src: &Path, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create anchor parent dir {}", parent.display()))?;
    }
    std::fs::copy(src, dest)
        .with_context(|| format!("copy {} -> {}", src.display(), dest.display()))?;
    Ok(())
}

/// True when the file at `path` contains `fingerprint_hex` as a case-
/// insensitive substring on any line. Used both for anchor-file probing
/// and merged-bundle probing.
fn file_contains_fingerprint(path: &Path, fingerprint_hex: &str) -> Result<bool> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("read trust-bundle {}", path.display()))?;
    Ok(haystack_contains_fingerprint(&contents, fingerprint_hex))
}

/// Same defence-in-depth as the macOS backend: any string passed onto the
/// `trust` argv MUST be plain hex. `update-ca-certificates` and
/// `update-ca-trust` do NOT consume the fingerprint, but `trust anchor
/// --remove` etc. could in future revisions, and review-standards `T8`
/// expects a single validator at the trust-store boundary.
fn validate_hex_fingerprint(s: &str) -> Result<()> {
    if s.is_empty() || s.len() > 128 {
        return Err(anyhow!(
            "fingerprint hex length {} is outside the expected [1, 128] range",
            s.len()
        ));
    }
    if !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(anyhow!(
            "fingerprint hex contains a non-hex byte; refusing to pass to Linux trust CLI"
        ));
    }
    Ok(())
}

/// Case-insensitive substring match for `fingerprint_hex` against any line
/// of `haystack`. Lifted from `macos.rs` — same semantics, different
/// haystack (PEM bundle vs `security` stdout).
fn haystack_contains_fingerprint(haystack: &str, fingerprint_hex: &str) -> bool {
    let needle = fingerprint_hex.to_ascii_lowercase();
    haystack
        .lines()
        .any(|line| line.to_ascii_lowercase().contains(&needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    // ──────────────────────────────────────────────────────────────────
    //  Distro detection — pure-function tests, no fakes needed.
    // ──────────────────────────────────────────────────────────────────

    /// `ID=debian` is recognised regardless of quoting.
    #[test]
    fn detect_distro_debian_via_id() {
        let bare = "ID=debian\nVERSION_ID=12\n";
        let dquoted = "ID=\"debian\"\nVERSION_ID=\"12\"\n";
        let squoted = "ID='debian'\n";
        for s in [bare, dquoted, squoted] {
            assert_eq!(
                detect_distro_from(s),
                LinuxDistro::Debian,
                "should detect debian from: {s:?}"
            );
        }
    }

    /// `ID=ubuntu` (with the typical noble release fields) → Debian family.
    #[test]
    fn detect_distro_ubuntu_via_id_maps_to_debian() {
        let s = r#"
PRETTY_NAME="Ubuntu 24.04 LTS"
NAME="Ubuntu"
VERSION_ID="24.04"
VERSION="24.04 LTS (Noble Numbat)"
ID=ubuntu
ID_LIKE=debian
"#;
        assert_eq!(detect_distro_from(s), LinuxDistro::Debian);
    }

    /// `ID=fedora` and `ID=rhel` map to the RHEL family.
    #[test]
    fn detect_distro_fedora_and_rhel_map_to_rhel_family() {
        let fedora = "ID=fedora\nVERSION_ID=42\n";
        let rhel = "ID=\"rhel\"\nVERSION_ID=\"9.4\"\nID_LIKE=\"fedora\"\n";
        let rocky = "ID=\"rocky\"\nVERSION_ID=\"9.4\"\nID_LIKE=\"rhel centos fedora\"\n";
        assert_eq!(detect_distro_from(fedora), LinuxDistro::Rhel);
        assert_eq!(detect_distro_from(rhel), LinuxDistro::Rhel);
        assert_eq!(detect_distro_from(rocky), LinuxDistro::Rhel);
    }

    /// `ID=arch` is recognised, as are Arch derivatives.
    #[test]
    fn detect_distro_arch_and_derivatives() {
        let arch = "ID=arch\n";
        let manjaro = "ID=manjaro\nID_LIKE=arch\n";
        assert_eq!(detect_distro_from(arch), LinuxDistro::Arch);
        assert_eq!(detect_distro_from(manjaro), LinuxDistro::Arch);
    }

    /// `ID=alpine` is recognised.
    #[test]
    fn detect_distro_alpine() {
        let s = "NAME=\"Alpine Linux\"\nID=alpine\nVERSION_ID=3.20\n";
        assert_eq!(detect_distro_from(s), LinuxDistro::Alpine);
    }

    /// Unknown `ID` with no `ID_LIKE` → fail-closed.
    #[test]
    fn detect_distro_unknown_id_no_id_like_returns_unknown() {
        let s = "ID=plan9\nVERSION_ID=4th\n";
        assert_eq!(detect_distro_from(s), LinuxDistro::Unknown);
    }

    /// `ID_LIKE` is consulted when `ID` is unknown. An off-brand distro
    /// claiming "ID=foobar ID_LIKE=debian" still routes to Debian.
    #[test]
    fn detect_distro_falls_back_to_id_like_when_id_unknown() {
        let s = "ID=foobar\nID_LIKE=\"debian ubuntu\"\n";
        assert_eq!(detect_distro_from(s), LinuxDistro::Debian);
    }

    /// Blank file → Unknown (defensive — minimal containers might ship an
    /// empty os-release on accident).
    #[test]
    fn detect_distro_blank_input_returns_unknown() {
        assert_eq!(detect_distro_from(""), LinuxDistro::Unknown);
    }

    /// Comments and blank lines are skipped without affecting detection.
    #[test]
    fn detect_distro_skips_comments_and_blanks() {
        let s = "# This is a comment\n\n\nID=fedora\n# trailing comment\n";
        assert_eq!(detect_distro_from(s), LinuxDistro::Rhel);
    }

    // ──────────────────────────────────────────────────────────────────
    //  Fake runner + add_root / remove_root / verify_installed argv-shape tests.
    // ──────────────────────────────────────────────────────────────────

    /// Fake `CommandRunner` mirroring the macOS test pattern.
    #[derive(Debug, Default)]
    struct FakeRunner {
        scripted: Mutex<Vec<CommandOutput>>,
        calls: Mutex<Vec<(String, Vec<OsString>)>>,
    }

    impl FakeRunner {
        fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }
        fn push(&self, out: CommandOutput) {
            self.scripted.lock().unwrap().push(out);
        }
        fn calls(&self) -> Vec<(String, Vec<OsString>)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl CommandRunner for Arc<FakeRunner> {
        fn run(&self, program: &str, args: &[OsString]) -> Result<CommandOutput> {
            self.calls
                .lock()
                .unwrap()
                .push((program.to_string(), args.to_vec()));
            let mut scripted = self.scripted.lock().unwrap();
            if scripted.is_empty() {
                panic!(
                    "FakeRunner exhausted: unexpected call to {program} {:?}",
                    args
                );
            }
            Ok(scripted.remove(0))
        }
    }

    fn ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            status: Some(0),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    fn err(code: i32, stderr: &str) -> CommandOutput {
        CommandOutput {
            status: Some(code),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    /// Build a Debian-family store rooted at a tempdir. The CA PEM source
    /// lives inside the tempdir too; the anchor path is the tempdir-rooted
    /// override; the merged-bundle path is a tempfile we can pre-seed.
    fn tempdir_debian_store(
        runner: Arc<FakeRunner>,
        anchor_override: PathBuf,
        merged_bundle_override: PathBuf,
    ) -> LinuxTrustStore {
        LinuxTrustStore::with_runner(
            runner,
            LinuxDistro::Debian,
            Some(anchor_override),
            Some(merged_bundle_override),
        )
    }

    const FINGERPRINT: &str = "68f37edc0d2063597abb39b2f99b02562210ce9f433c50201de6b7e1296ed371";

    fn make_pem(tmp: &Path) -> PathBuf {
        let pem = tmp.join("ca.pem");
        std::fs::write(
            &pem,
            b"-----BEGIN CERTIFICATE-----\nstub\n-----END CERTIFICATE-----\n",
        )
        .expect("seed pem");
        pem
    }

    /// `add_root` on Debian writes the anchor file then runs
    /// `update-ca-certificates --verbose`. Single positional shell-out (T8).
    #[test]
    fn add_root_debian_writes_anchor_and_runs_update_ca_certificates() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runner = FakeRunner::new();
        runner.push(ok("1 added, 0 removed; done.\n"));

        let pem = make_pem(tmp.path());
        let anchor = tmp.path().join("anchors/spendguard.crt");
        let bundle = tmp.path().join("ca-certificates.crt");
        // Bundle file doesn't need to exist for add_root; create empty.
        std::fs::write(&bundle, b"").expect("seed bundle");

        let store = tempdir_debian_store(runner.clone(), anchor.clone(), bundle);
        let touched = store
            .add_root(&pem, TrustScope::System)
            .expect("add_root debian");

        assert_eq!(touched, vec![anchor.clone()]);
        assert!(anchor.exists(), "anchor file must be staged: {anchor:?}");

        let calls = runner.calls();
        assert_eq!(calls.len(), 1, "exactly one shell-out: {calls:?}");
        let (program, args) = &calls[0];
        assert_eq!(program, "update-ca-certificates");
        assert_eq!(args.as_slice(), &[OsString::from("--verbose")]);
    }

    /// `add_root` on RHEL uses `update-ca-trust extract` and the
    /// `/etc/pki/ca-trust/source/anchors/` path. With the override we
    /// target a tempdir-rooted path, but the argv shape is what matters.
    #[test]
    fn add_root_rhel_runs_update_ca_trust_extract() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runner = FakeRunner::new();
        runner.push(ok(""));

        let pem = make_pem(tmp.path());
        let anchor = tmp.path().join("pki/anchors/spendguard.crt");
        let bundle = tmp.path().join("tls-ca-bundle.pem");
        std::fs::write(&bundle, b"").expect("seed bundle");

        let store = LinuxTrustStore::with_runner(
            runner.clone(),
            LinuxDistro::Rhel,
            Some(anchor.clone()),
            Some(bundle),
        );
        let touched = store
            .add_root(&pem, TrustScope::System)
            .expect("add_root rhel");
        assert_eq!(touched, vec![anchor]);

        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        let (program, args) = &calls[0];
        assert_eq!(program, "update-ca-trust");
        assert_eq!(args.as_slice(), &[OsString::from("extract")]);
    }

    /// `add_root` on Arch uses `trust anchor --store <pem>`. The PEM path
    /// is passed positionally — T8 cannot trigger because we ship a
    /// PathBuf, not user-controlled bytes.
    #[test]
    fn add_root_arch_runs_trust_anchor_with_store() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runner = FakeRunner::new();
        runner.push(ok(""));

        let pem = make_pem(tmp.path());
        let anchor = tmp.path().join("p11/spendguard.p11-kit");

        let store = LinuxTrustStore::with_runner(
            runner.clone(),
            LinuxDistro::Arch,
            Some(anchor.clone()),
            None,
        );
        let touched = store
            .add_root(&pem, TrustScope::System)
            .expect("add_root arch");
        assert_eq!(touched, vec![anchor]);

        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        let (program, args) = &calls[0];
        assert_eq!(program, "trust");
        assert_eq!(args[0], OsString::from("anchor"));
        assert_eq!(args[1], OsString::from("--store"));
        assert_eq!(args[2], pem.as_os_str());
    }

    /// `add_root` on Alpine matches Debian (same anchor dir + tool).
    #[test]
    fn add_root_alpine_matches_debian_shape() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runner = FakeRunner::new();
        runner.push(ok(""));

        let pem = make_pem(tmp.path());
        let anchor = tmp.path().join("alpine/spendguard.crt");
        let bundle = tmp.path().join("ca-certificates.crt");
        std::fs::write(&bundle, b"").expect("seed bundle");

        let store = LinuxTrustStore::with_runner(
            runner.clone(),
            LinuxDistro::Alpine,
            Some(anchor.clone()),
            Some(bundle),
        );
        store
            .add_root(&pem, TrustScope::System)
            .expect("add_root alpine");

        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        let (program, args) = &calls[0];
        assert_eq!(program, "update-ca-certificates");
        assert_eq!(args.as_slice(), &[OsString::from("--verbose")]);
    }

    /// `add_root` surfaces a non-zero exit verbatim with stderr context.
    #[test]
    fn add_root_surfaces_update_ca_certificates_nonzero_exit() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runner = FakeRunner::new();
        runner.push(err(1, "update-ca-certificates: permission denied"));

        let pem = make_pem(tmp.path());
        let anchor = tmp.path().join("anchors/spendguard.crt");
        let bundle = tmp.path().join("ca-certificates.crt");
        std::fs::write(&bundle, b"").expect("seed bundle");

        let store = tempdir_debian_store(runner, anchor, bundle);
        let err = store
            .add_root(&pem, TrustScope::System)
            .expect_err("must error on non-zero exit");
        let msg = format!("{err:#}");
        assert!(msg.contains("permission denied"), "stderr context: {msg}");
        assert!(msg.contains("update-ca-certificates"), "cmd context: {msg}");
    }

    /// `add_root` on Unknown fails closed — T6 belt-and-braces.
    #[test]
    fn add_root_unknown_distro_fails_closed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runner = FakeRunner::new();
        // No scripted output — if add_root tried to shell out, FakeRunner
        // would panic, which would fail the test for a different reason.

        let pem = make_pem(tmp.path());
        let store = LinuxTrustStore::with_runner(runner, LinuxDistro::Unknown, None, None);
        let err = store
            .add_root(&pem, TrustScope::System)
            .expect_err("Unknown must refuse");
        let msg = format!("{err:#}");
        assert!(msg.contains("no Linux trust backend matches"));
        assert!(msg.contains("fail-closed"));
    }

    /// `verify_installed` returns true when the fingerprint is in the
    /// merged bundle file.
    #[test]
    fn verify_installed_finds_fingerprint_in_merged_bundle() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runner = FakeRunner::new();
        // verify_installed → no shell-out for Debian when merged bundle hits.
        let anchor = tmp.path().join("anchors/spendguard.crt");
        let bundle = tmp.path().join("ca-certificates.crt");
        // Anchor absent on purpose so we exercise the merged-bundle path.
        std::fs::write(
            &bundle,
            format!(
                "# SpendGuard\n# SHA-256 Fingerprint: {}\n-----BEGIN CERTIFICATE-----\nfoo\n",
                FINGERPRINT.to_uppercase()
            ),
        )
        .expect("seed bundle");

        let store = tempdir_debian_store(runner.clone(), anchor, bundle);
        let present = store
            .verify_installed(FINGERPRINT, TrustScope::System)
            .expect("verify");
        assert!(present, "fingerprint should be detected case-insensitively");
        assert!(runner.calls().is_empty(), "no shell-out expected");
    }

    /// `verify_installed` returns false (NOT Err) when neither anchor nor
    /// merged bundle contains the fingerprint — review-standards X3 / A8.2.
    #[test]
    fn verify_installed_returns_false_when_absent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runner = FakeRunner::new();
        let anchor = tmp.path().join("anchors/spendguard.crt");
        let bundle = tmp.path().join("ca-certificates.crt");
        std::fs::write(&bundle, b"# empty bundle\n").expect("seed bundle");

        let store = tempdir_debian_store(runner.clone(), anchor, bundle);
        let present = store
            .verify_installed(FINGERPRINT, TrustScope::System)
            .expect("verify must not error");
        assert!(!present);
        assert!(runner.calls().is_empty(), "no shell-out expected");
    }

    /// `verify_installed` on Unknown returns Ok(false), never Err — the
    /// doctor report surfaces "absent because we couldn't look".
    #[test]
    fn verify_installed_unknown_distro_returns_false() {
        let runner = FakeRunner::new();
        let store = LinuxTrustStore::with_runner(runner, LinuxDistro::Unknown, None, None);
        let present = store
            .verify_installed(FINGERPRINT, TrustScope::User)
            .expect("verify must not error on Unknown");
        assert!(!present);
    }

    /// `verify_installed` on Arch shells out to `trust list --filter=ca-anchors`.
    #[test]
    fn verify_installed_arch_shells_out_to_trust_list() {
        let runner = FakeRunner::new();
        runner.push(ok(&format!(
            "label: SpendGuard Local Root R1\n    trust: anchor\n    sha256: {}\n",
            FINGERPRINT.to_uppercase()
        )));

        let store = LinuxTrustStore::with_runner(runner.clone(), LinuxDistro::Arch, None, None);
        // For Arch, anchor_path defaults to a system path we don't write
        // to, so verify must reach the trust-list shell-out.
        let present = store
            .verify_installed(FINGERPRINT, TrustScope::User)
            .expect("verify arch");
        assert!(present);

        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        let (program, args) = &calls[0];
        assert_eq!(program, "trust");
        assert_eq!(args[0], OsString::from("list"));
        assert_eq!(args[1], OsString::from("--filter=ca-anchors"));
    }

    /// `remove_root` short-circuits when the anchor is absent AND the
    /// merged bundle does not contain the fingerprint (no update shell-out).
    #[test]
    fn remove_root_is_noop_when_cert_already_absent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runner = FakeRunner::new();
        // No scripted output — FakeRunner will panic on any shell-out.

        let anchor = tmp.path().join("anchors/spendguard.crt");
        let bundle = tmp.path().join("ca-certificates.crt");
        std::fs::write(&bundle, b"# empty bundle\n").expect("seed bundle");
        // anchor file deliberately not created.

        let store = tempdir_debian_store(runner.clone(), anchor, bundle);
        let touched = store
            .remove_root(FINGERPRINT, TrustScope::System)
            .expect("remove_root");
        assert!(touched.is_empty(), "no-op should return empty Vec");
        assert!(runner.calls().is_empty(), "no shell-out for absent cert");
    }

    /// `remove_root` on Debian removes the anchor file and runs
    /// `update-ca-certificates --fresh --verbose`.
    #[test]
    fn remove_root_debian_runs_update_fresh() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runner = FakeRunner::new();
        runner.push(ok("0 added, 1 removed; done.\n"));

        let anchor = tmp.path().join("anchors/spendguard.crt");
        std::fs::create_dir_all(anchor.parent().unwrap()).expect("mk anchor dir");
        std::fs::write(&anchor, b"stub-pem").expect("seed anchor");
        let bundle = tmp.path().join("ca-certificates.crt");
        std::fs::write(&bundle, b"# empty bundle\n").expect("seed bundle");

        let store = tempdir_debian_store(runner.clone(), anchor.clone(), bundle);
        let touched = store
            .remove_root(FINGERPRINT, TrustScope::System)
            .expect("remove_root");
        assert_eq!(touched, vec![anchor.clone()]);
        assert!(!anchor.exists(), "anchor file must be deleted");

        let calls = runner.calls();
        assert_eq!(calls.len(), 1, "single update shell-out");
        let (program, args) = &calls[0];
        assert_eq!(program, "update-ca-certificates");
        assert_eq!(
            args.as_slice(),
            &[OsString::from("--fresh"), OsString::from("--verbose")]
        );
    }

    /// `validate_hex_fingerprint` rejects shell metacharacters and flag-
    /// shaped strings before they could reach any trust CLI. Mirrors the
    /// macOS belt-and-braces test (T8).
    #[test]
    fn validate_hex_fingerprint_rejects_non_hex() {
        for bad in [
            "",                       // empty
            "abc; rm -rf /",          // shell injection attempt
            "--insert-evil-flag",     // flag injection attempt
            "abc\nSHA-256: DEAD",     // newline injection
            "🦀rust",                 // unicode
            "a".repeat(129).as_str(), // overlong
        ] {
            let err = validate_hex_fingerprint(bad).expect_err("non-hex must be rejected");
            let msg = format!("{err}");
            assert!(
                msg.contains("hex") || msg.contains("length") || msg.contains("range"),
                "expected hex-validation error for {bad:?}, got: {msg}"
            );
        }
    }

    /// `LinuxDistro::as_str` returns stable identifiers used by error
    /// messages / doctor reports.
    #[test]
    fn linux_distro_as_str_is_stable() {
        assert_eq!(LinuxDistro::Debian.as_str(), "debian");
        assert_eq!(LinuxDistro::Rhel.as_str(), "rhel");
        assert_eq!(LinuxDistro::Arch.as_str(), "arch");
        assert_eq!(LinuxDistro::Alpine.as_str(), "alpine");
        assert_eq!(LinuxDistro::Unknown.as_str(), "unknown");
    }

    // ──────────────────────────────────────────────────────────────────
    //  R2 Blocker B1 — Linux `--scope user` fails closed, never shells
    //  out, and the error message includes the CURL_CA_BUNDLE token the
    //  SLICE 7 doctor greps for. Belt-and-braces on T6 (no silent
    //  escalation): if these regressed, a user-scope install on a root
    //  shell would silently rebuild the SYSTEM trust bundle.
    // ──────────────────────────────────────────────────────────────────

    /// `add_root` with `TrustScope::User` returns the user-scope error,
    /// stages no file, and invokes no shell-out — even when the FakeRunner
    /// has no scripted output (which would panic if reached).
    #[test]
    fn add_root_user_scope_fails_closed_with_curl_ca_bundle_hint() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runner = FakeRunner::new();
        // No scripted output — any shell-out below would panic FakeRunner
        // and surface a different test failure than the assertion below.

        let pem = make_pem(tmp.path());
        let anchor = tmp.path().join("anchors/spendguard.crt");
        let bundle = tmp.path().join("ca-certificates.crt");
        std::fs::write(&bundle, b"").expect("seed bundle");

        let store = tempdir_debian_store(runner.clone(), anchor.clone(), bundle);
        let err = store
            .add_root(&pem, TrustScope::User)
            .expect_err("user scope must fail closed on Linux");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("CURL_CA_BUNDLE"),
            "error message must mention CURL_CA_BUNDLE for SLICE 7 doctor grep, got: {msg}"
        );
        assert!(
            msg.contains("user-scope"),
            "error message must name the failing scope, got: {msg}"
        );
        assert!(
            !anchor.exists(),
            "anchor file must not be staged for user scope: {anchor:?}"
        );
        assert!(
            runner.calls().is_empty(),
            "no shell-out should fire for user scope, got: {:?}",
            runner.calls()
        );
    }

    /// `remove_root` mirrors `add_root` for user scope: error variant,
    /// no shell-out, no fs mutation. Same CURL_CA_BUNDLE hint so the
    /// SLICE 7 doctor surface is consistent across install / uninstall.
    #[test]
    fn remove_root_user_scope_fails_closed_with_curl_ca_bundle_hint() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runner = FakeRunner::new();

        let anchor = tmp.path().join("anchors/spendguard.crt");
        let bundle = tmp.path().join("ca-certificates.crt");
        std::fs::write(&bundle, b"").expect("seed bundle");

        let store = tempdir_debian_store(runner.clone(), anchor.clone(), bundle);
        let err = store
            .remove_root(FINGERPRINT, TrustScope::User)
            .expect_err("user scope must fail closed on Linux");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("CURL_CA_BUNDLE"),
            "error message must mention CURL_CA_BUNDLE for SLICE 7 doctor grep, got: {msg}"
        );
        assert!(
            runner.calls().is_empty(),
            "no shell-out should fire for user scope, got: {:?}",
            runner.calls()
        );
    }

    /// Cross-distro coverage of B1: user-scope fails closed on Arch too
    /// (no `trust anchor` invocation, no anchor stage).
    #[test]
    fn add_root_user_scope_fails_closed_on_arch() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runner = FakeRunner::new();
        let pem = make_pem(tmp.path());

        let store = LinuxTrustStore::with_runner(runner.clone(), LinuxDistro::Arch, None, None);
        let err = store
            .add_root(&pem, TrustScope::User)
            .expect_err("Arch user scope must fail closed");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("CURL_CA_BUNDLE"),
            "Arch user-scope error must surface CURL_CA_BUNDLE hint, got: {msg}"
        );
        assert!(runner.calls().is_empty(), "no trust anchor invocation");
    }

    // ──────────────────────────────────────────────────────────────────
    //  R2 minor m1 — Arch `add_root` does NOT stage a phantom anchor file
    //  (the p11-kit canonical anchor dir is `/usr/share/ca-certificates/
    //  trust-source/` and `trust anchor --store` reads the source PEM
    //  by path, not the staged copy).
    // ──────────────────────────────────────────────────────────────────

    /// Arch `add_root` invokes `trust anchor --store <ca_pem_path>` and
    /// MUST NOT write to `anchor_path`. Verifies via direct fs probe — if
    /// `copy_pem_into` regressed back into the unconditional branch the
    /// anchor file would be present at `anchor`.
    #[test]
    fn add_root_arch_does_not_stage_phantom_anchor_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let runner = FakeRunner::new();
        runner.push(ok(""));

        let pem = make_pem(tmp.path());
        let anchor = tmp.path().join("p11/spendguard.p11-kit");

        let store = LinuxTrustStore::with_runner(
            runner.clone(),
            LinuxDistro::Arch,
            Some(anchor.clone()),
            None,
        );
        let touched = store
            .add_root(&pem, TrustScope::System)
            .expect("add_root arch");

        // `add_root` still returns the conceptual anchor path for the
        // install report (operators expect to see it), but no I/O fires
        // to that location.
        assert_eq!(touched, vec![anchor.clone()]);
        assert!(
            !anchor.exists(),
            "Arch add_root must NOT stage a phantom anchor file at {anchor:?}; \
             `trust anchor --store <ca_pem_path>` reads the source PEM directly"
        );
        // Parent dir also must not have been created for the phantom.
        assert!(
            !anchor.parent().unwrap().exists(),
            "Arch add_root must not even create the anchor parent dir"
        );

        let calls = runner.calls();
        assert_eq!(calls.len(), 1, "exactly one shell-out: {calls:?}");
        let (program, _) = &calls[0];
        assert_eq!(program, "trust");
    }
}
