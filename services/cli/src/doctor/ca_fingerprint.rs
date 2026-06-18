//! SLICE 7 (COV_11) — CA fingerprint trust-store probe.
//!
//! Read-only — opens the on-disk CA PEM, SHA-256s the file bytes (which
//! are already DER-equivalent under PEM's deterministic base64 wrap so a
//! file-content hash is identity-equivalent to a per-cert hash), and
//! cross-checks the resulting fingerprint against the OS trust store via
//! [`crate::trust::TrustStore::verify_installed`].
//!
//! ## Why hash the PEM bytes, not the parsed cert
//!
//! `ca::fingerprint_hex` returns the SHA-256 of the DER body, which is
//! what every trust-store backend compares against. Re-parsing the PEM
//! via `x509-parser` would require pulling that crate into the doctor
//! module's runtime deps; instead we hash the file bytes and accept the
//! caveat that this fingerprint MUST match what the install slice wrote
//! to `InstallReport.ca_fingerprint_sha256` (which it does, because
//! install uses the same `sha2::Sha256` over the same PEM bytes).
//!
//! Deviation #3: the production fingerprint for `verify_installed`
//! cross-check is the SHA-256 of the cert's DER encoding, not the PEM
//! bytes — the macOS / Linux / Windows backends search the trust store
//! using the certificate-content hash. To compute that hash we DECODE
//! the PEM's base64 body and SHA-256 the resulting DER. This keeps the
//! fingerprint identity-equivalent to the install-time
//! `ca::fingerprint_hex` (which hashes the cert object's DER bytes).
//!
//! ## Anti-scope
//!
//! - We do NOT mutate the trust store, even for "repair-now" prompts.
//!   Doctor is read-only per the slice-doc anti-scope.
//! - We do NOT issue a fresh CA on the user's behalf if absent — that
//!   would race the install slice's atomic-write contract.

use crate::preflight::BaseEnv;
use crate::TrustScope;
use base64::Engine;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Outcome of the CA probe. Each variant carries enough context for the
/// renderer to format a useful one-liner.
#[derive(Debug, Clone)]
pub enum CaCheckResult {
    /// CA PEM exists, fingerprint matches what's in the OS trust store.
    Healthy {
        fingerprint: String,
        /// Backend-reported trust-store paths (informational — surfaced
        /// in the render so the operator can see "yes, it's in the
        /// keychain at <path>").
        trust_store_locations: Vec<PathBuf>,
    },
    /// CA PEM exists, fingerprint computed, but the OS trust store does
    /// NOT have it. Indicates a botched / partial install.
    NotInTrustStore {
        fingerprint: String,
        ca_pem_path: PathBuf,
    },
    /// CA PEM file is absent — `spendguard install` was never run, or
    /// `spendguard uninstall` cleaned it up.
    NotInstalled,
    /// CA PEM file exists but can't be read or parsed.
    FileNotReadable { ca_pem_path: PathBuf, error: String },
}

impl CaCheckResult {
    /// One-line render for the doctor report. `use_color=true` adds
    /// ANSI markers.
    pub fn render(&self, use_color: bool) -> String {
        use crate::doctor::{paint, Color};
        match self {
            Self::Healthy {
                fingerprint,
                trust_store_locations,
            } => {
                let head = paint("OK", Color::Green, use_color);
                let locs = if trust_store_locations.is_empty() {
                    String::new()
                } else {
                    format!(
                        " ({})",
                        trust_store_locations
                            .iter()
                            .map(|p| p.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                let short = &fingerprint[..fingerprint.len().min(16)];
                format!("{head} sha256={short}…{locs}")
            }
            Self::NotInTrustStore {
                fingerprint,
                ca_pem_path,
            } => {
                let head = paint("WARN", Color::Yellow, use_color);
                let short = &fingerprint[..fingerprint.len().min(16)];
                format!(
                    "{head} CA at {} (sha256={short}…) not found in OS trust store \
                     — re-run `spendguard install`",
                    ca_pem_path.display()
                )
            }
            Self::NotInstalled => {
                let head = paint("MISSING", Color::Yellow, use_color);
                format!("{head} no CA PEM on disk (run `spendguard install`)")
            }
            Self::FileNotReadable { ca_pem_path, error } => {
                let head = paint("ERROR", Color::Red, use_color);
                format!(
                    "{head} CA PEM at {} unreadable: {}",
                    ca_pem_path.display(),
                    error
                )
            }
        }
    }

    /// Lower-case hex SHA-256 of the cert content, if computed. Used by
    /// the lib-level tests + the renderer.
    pub fn fingerprint(&self) -> Option<&str> {
        match self {
            Self::Healthy { fingerprint, .. } => Some(fingerprint),
            Self::NotInTrustStore { fingerprint, .. } => Some(fingerprint),
            _ => None,
        }
    }
}

/// Probe the CA trust-store status. Reads the PEM from
/// `ca_pem_override` if supplied; otherwise resolves the canonical path
/// from [`crate::doctor::default_ca_pem_path`].
///
/// Cross-checks the resulting fingerprint against the trust store via
/// [`crate::trust::dispatch`]. When the production dispatch fails (e.g.
/// on an unsupported OS), surfaces `NotInTrustStore` with the computed
/// fingerprint rather than erroring — the operator can still see which
/// fingerprint they have on disk.
pub fn check(env: &BaseEnv<'_>, ca_pem_override: Option<&Path>) -> CaCheckResult {
    let trust = dispatch_or_noop();
    check_with_trust(env, ca_pem_override, trust.as_ref())
}

/// Same as [`check`] but with an injected `TrustStore` so unit tests can
/// drive the cross-check deterministically. `ca_pem_override=None` falls
/// back to the same canonical path resolution as [`check`].
pub fn check_with_trust(
    env: &BaseEnv<'_>,
    ca_pem_override: Option<&Path>,
    trust: &dyn crate::trust::TrustStore,
) -> CaCheckResult {
    let pem_path = match ca_pem_override {
        Some(p) => p.to_path_buf(),
        None => match crate::doctor::default_ca_pem_path(env) {
            Some(p) => p,
            None => return CaCheckResult::NotInstalled,
        },
    };

    let pem_bytes = match std::fs::read(&pem_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return CaCheckResult::NotInstalled,
        Err(e) => {
            return CaCheckResult::FileNotReadable {
                ca_pem_path: pem_path,
                error: e.to_string(),
            };
        }
    };

    let fingerprint = match cert_der_sha256_hex(&pem_bytes) {
        Ok(fp) => fp,
        Err(e) => {
            return CaCheckResult::FileNotReadable {
                ca_pem_path: pem_path,
                error: format!("decode PEM: {e}"),
            };
        }
    };

    // Cross-check against the trust store. Default to `--scope user`
    // because that's the install default (see InstallOpts::scope
    // default at lib.rs:55). An operator who ran `--scope system`
    // should re-run with the same scope; we don't auto-probe both
    // because that would surface "missing from system" warnings on
    // user-scope installs.
    //
    // We call `verify_trusted` (present AND actually trusted as a root),
    // NOT `verify_installed` (presence only). A present-but-untrusted CA —
    // e.g. one whose macOS trust settings were denied/removed — must NOT
    // report Healthy: that over-reports health in the less-safe direction
    // for a control whose whole job is to confirm the MITM CA is trusted.
    // `Err(_)` (locked keychain / backend failure / any inconclusive
    // state) stays fail-closed: it maps to NotInTrustStore, never Healthy.
    let scope = TrustScope::User;
    match trust.verify_trusted(&pem_path, &fingerprint, scope) {
        Ok(true) => CaCheckResult::Healthy {
            fingerprint,
            trust_store_locations: trust_locations_for(scope),
        },
        Ok(false) => CaCheckResult::NotInTrustStore {
            fingerprint,
            ca_pem_path: pem_path,
        },
        Err(_) => CaCheckResult::NotInTrustStore {
            fingerprint,
            ca_pem_path: pem_path,
        },
    }
}

/// Compute SHA-256 hex of the DER-encoded cert inside a PEM blob. Mirrors
/// what `ca::fingerprint_hex` produces for the install-side fingerprint.
///
/// `pub(crate)` so SLICE 8 (`uninstall`) can re-derive the install-time
/// fingerprint from the on-disk CA PEM when the operator omits
/// `--ca-fingerprint`. Same identity-equivalence reasoning as the SLICE 7
/// doctor path: the hash is over the cert DER body, which `add_root` /
/// `remove_root` use to match the trust-store entry.
pub(crate) fn cert_der_sha256_hex(pem_bytes: &[u8]) -> std::result::Result<String, &'static str> {
    let pem = std::str::from_utf8(pem_bytes).map_err(|_| "PEM is not UTF-8")?;
    // PEM is a base64 body bracketed by BEGIN / END lines. We strip the
    // headers + concatenate the body without any external dep beyond the
    // `base64` crate which is already in the workspace via tokio-rustls
    // transitive (verified by the SLICE 4 tokenizer crate).
    let mut body = String::new();
    let mut in_body = false;
    for line in pem.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("-----BEGIN") {
            in_body = true;
            continue;
        }
        if trimmed.starts_with("-----END") {
            break;
        }
        if in_body {
            body.push_str(trimmed);
        }
    }
    if body.is_empty() {
        return Err("PEM has no body — missing BEGIN/END markers");
    }
    let der = base64::engine::general_purpose::STANDARD
        .decode(body.as_bytes())
        .map_err(|_| "PEM body is not valid base64")?;
    let digest = Sha256::digest(&der);
    Ok(hex::encode(digest))
}

/// Best-effort: return the OS-canonical trust-store paths so the
/// `Healthy` render can show *where* the cert is trusted. We don't
/// actually probe the store — those paths are stable per OS.
fn trust_locations_for(_scope: TrustScope) -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        match _scope {
            TrustScope::User => {
                if let Some(home) = std::env::var_os("HOME") {
                    return vec![PathBuf::from(home)
                        .join("Library")
                        .join("Keychains")
                        .join("login.keychain-db")];
                }
                Vec::new()
            }
            TrustScope::System => vec![PathBuf::from("/Library/Keychains/System.keychain")],
        }
    }
    #[cfg(target_os = "linux")]
    {
        vec![
            PathBuf::from("/etc/ssl/certs/ca-certificates.crt"),
            PathBuf::from("/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem"),
        ]
    }
    #[cfg(target_os = "windows")]
    {
        vec![PathBuf::from("CurrentUser\\Root")]
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = _scope;
        Vec::new()
    }
}

/// Fallback TrustStore impl used when the production dispatch fails (no
/// OS backend matches). Always reports `verify_installed = false` so the
/// doctor surfaces "not in store" rather than erroring.
struct NoOpDispatchTrustStore;

impl crate::trust::TrustStore for NoOpDispatchTrustStore {
    fn add_root(
        &self,
        _: &Path,
        _: TrustScope,
    ) -> std::result::Result<Vec<PathBuf>, anyhow::Error> {
        unreachable!("doctor must not call add_root")
    }
    fn remove_root(
        &self,
        _: &str,
        _: TrustScope,
    ) -> std::result::Result<Vec<PathBuf>, anyhow::Error> {
        unreachable!("doctor must not call remove_root")
    }
    fn verify_installed(&self, _: &str, _: TrustScope) -> std::result::Result<bool, anyhow::Error> {
        Ok(false)
    }
}

/// Dispatch to the OS backend or fall through to a no-op that yields
/// "not in store" so the doctor degrades gracefully on unsupported
/// hosts.
fn dispatch_or_noop() -> Box<dyn crate::trust::TrustStore> {
    crate::trust::dispatch().unwrap_or_else(|_| Box::new(NoOpDispatchTrustStore))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Mutex;

    /// Test TrustStore that returns a fixed answer + records queries.
    struct FakeTrust {
        present: bool,
        queries: Mutex<Vec<String>>,
    }
    impl FakeTrust {
        fn new(present: bool) -> Self {
            Self {
                present,
                queries: Mutex::new(Vec::new()),
            }
        }
    }
    impl crate::trust::TrustStore for FakeTrust {
        fn add_root(
            &self,
            _: &Path,
            _: TrustScope,
        ) -> std::result::Result<Vec<PathBuf>, anyhow::Error> {
            unreachable!("doctor must not call add_root")
        }
        fn remove_root(
            &self,
            _: &str,
            _: TrustScope,
        ) -> std::result::Result<Vec<PathBuf>, anyhow::Error> {
            unreachable!("doctor must not call remove_root")
        }
        fn verify_installed(
            &self,
            fp: &str,
            _: TrustScope,
        ) -> std::result::Result<bool, anyhow::Error> {
            self.queries.lock().unwrap().push(fp.to_string());
            Ok(self.present)
        }
    }

    /// Real CA PEM substrate — issue once and reuse so the per-test
    /// fingerprint stays stable. We don't depend on `ca::fingerprint_hex`
    /// here — the test verifies that our cert_der_sha256_hex computes
    /// the same DER hash by sha256-ing the rcgen-emitted DER directly.
    fn issue_real_ca(dir: &Path) -> (PathBuf, String) {
        let root = crate::ca::generate_root_ca().expect("issue CA");
        let pem_path = dir.join("root_ca.pem");
        std::fs::write(&pem_path, root.cert_pem.as_bytes()).expect("write PEM");
        let expected_fp = crate::ca::fingerprint_hex(&root.fingerprint_sha256);
        (pem_path, expected_fp)
    }

    /// Happy path: PEM on disk, trust says present → Healthy + fingerprint
    /// matches install-side hex.
    #[test]
    fn check_with_trust_returns_healthy_when_fingerprint_in_store() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (pem_path, expected_fp) = issue_real_ca(tmp.path());
        let trust = FakeTrust::new(true);
        let env = BaseEnv {
            home: Some(tmp.path()),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        let res = check_with_trust(&env, Some(&pem_path), &trust);
        match res {
            CaCheckResult::Healthy { fingerprint, .. } => {
                assert_eq!(fingerprint, expected_fp);
            }
            other => panic!("expected Healthy, got {other:?}"),
        }
        let queries = trust.queries.lock().unwrap().clone();
        assert_eq!(queries, vec![expected_fp.clone()]);
        // No verify_installed call beyond the single user-scope probe.
        assert_eq!(queries.len(), 1);
    }

    /// PEM on disk + trust says NOT present → NotInTrustStore.
    #[test]
    fn check_with_trust_returns_not_in_trust_store_when_absent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (pem_path, expected_fp) = issue_real_ca(tmp.path());
        let trust = FakeTrust::new(false);
        let env = BaseEnv {
            home: Some(tmp.path()),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        let res = check_with_trust(&env, Some(&pem_path), &trust);
        match res {
            CaCheckResult::NotInTrustStore {
                fingerprint,
                ca_pem_path,
            } => {
                assert_eq!(fingerprint, expected_fp);
                assert_eq!(ca_pem_path, pem_path);
            }
            other => panic!("expected NotInTrustStore, got {other:?}"),
        }
    }

    /// PEM missing → NotInstalled, no trust query happens.
    #[test]
    fn check_with_trust_returns_not_installed_when_pem_absent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let trust = FakeTrust::new(true);
        let env = BaseEnv {
            home: Some(tmp.path()),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        let missing = tmp.path().join("root_ca.pem");
        let res = check_with_trust(&env, Some(&missing), &trust);
        assert!(matches!(res, CaCheckResult::NotInstalled));
        assert!(trust.queries.lock().unwrap().is_empty());
    }

    /// Garbage PEM → FileNotReadable; trust query never happens.
    #[test]
    fn check_with_trust_returns_file_not_readable_for_invalid_pem() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let pem_path = tmp.path().join("root_ca.pem");
        std::fs::write(&pem_path, b"this is not a PEM").expect("write garbage");
        let trust = FakeTrust::new(true);
        let env = BaseEnv {
            home: Some(tmp.path()),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        let res = check_with_trust(&env, Some(&pem_path), &trust);
        match res {
            CaCheckResult::FileNotReadable { ca_pem_path, .. } => {
                assert_eq!(ca_pem_path, pem_path);
            }
            other => panic!("expected FileNotReadable, got {other:?}"),
        }
        assert!(trust.queries.lock().unwrap().is_empty());
    }

    /// Renderer emits the colour-coded prefix when `use_color=true` and
    /// strips it when `use_color=false`. We only assert presence of the
    /// status token (OK / WARN / MISSING / ERROR) — the exact ANSI
    /// escape sequence is the `paint()` test's concern.
    #[test]
    fn render_emits_expected_status_tokens() {
        let healthy = CaCheckResult::Healthy {
            fingerprint: "deadbeef".repeat(8),
            trust_store_locations: vec![PathBuf::from("/keychain")],
        };
        let plain = healthy.render(false);
        assert!(plain.contains("OK"));
        assert!(plain.contains("deadbeef"));
        assert!(plain.contains("/keychain"));

        let absent = CaCheckResult::NotInstalled;
        assert!(absent.render(false).contains("MISSING"));

        let not_in_store = CaCheckResult::NotInTrustStore {
            fingerprint: "abcd".repeat(16),
            ca_pem_path: PathBuf::from("/tmp/root_ca.pem"),
        };
        assert!(not_in_store.render(false).contains("WARN"));
        assert!(not_in_store.render(false).contains("/tmp/root_ca.pem"));

        let unreadable = CaCheckResult::FileNotReadable {
            ca_pem_path: PathBuf::from("/tmp/x.pem"),
            error: "denied".into(),
        };
        let unread = unreadable.render(false);
        assert!(unread.contains("ERROR"));
        assert!(unread.contains("denied"));
    }
}
