//! XDG-friendly path resolver for SpendGuard CLI state.
//!
//! Returns the OS-correct location for CA / leaf material:
//!
//! | OS      | Location                                              |
//! |---------|-------------------------------------------------------|
//! | Linux   | `$XDG_DATA_HOME/spendguard/ca/` (default `~/.local/share/spendguard/ca/`) |
//! | macOS   | `~/Library/Application Support/SpendGuard/ca/`        |
//! | Windows | `%APPDATA%\SpendGuard\ca\`                            |
//!
//! Hand-rolled instead of pulling the `directories` crate — only one knob is
//! actually OS-specific (the base data dir), and the `directories` API is
//! overkill for that.
//!
//! ## Test isolation
//!
//! R2 fix per security review (B3): base-dir resolution is factored into the
//! pure function [`base_data_dir_from`], which takes an explicit [`BaseEnv`]
//! borrow rather than reading process-global env. The previous test approach
//! (`std::env::set_var` inside `#[test]`s) races other tests in the same
//! cargo-test binary that read `HOME` / `APPDATA` / `XDG_DATA_HOME` — cargo
//! parallelises *within* a binary, not just *across* binaries, and Rust 2024
//! marks `set_var` `unsafe` for exactly this reason. Tests now construct
//! `BaseEnv` literals and exercise `base_data_dir_from` directly; the public
//! `ca_root_dir()` wrappers stay env-reading but are not unit-tested with
//! mutation.

use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;

/// Borrowed view of the env vars `base_data_dir_from` needs. Lifetime is
/// tied to the strings the caller owns (typically `std::env::var(...).ok()`
/// at runtime, or string literals in tests).
#[derive(Debug, Clone, Copy)]
pub struct BaseEnv<'a> {
    pub home: Option<&'a str>,
    pub xdg_data_home: Option<&'a str>,
    pub appdata: Option<&'a str>,
}

/// Resolve the per-user SpendGuard CA directory and ensure it exists.
///
/// Idempotent: re-creating an existing directory is a no-op. Honours
/// `$XDG_DATA_HOME` and `$HOME` (POSIX) / `%APPDATA%` (Windows) overrides so
/// tests can pin a sandbox via `tempfile::TempDir`.
pub fn ca_root_dir() -> Result<PathBuf> {
    let dir = ca_root_dir_unchecked()?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("create ca root dir {}", dir.display()))?;
    Ok(dir)
}

/// Same as [`ca_root_dir`] but does not touch the filesystem. Used by tests
/// asserting the resolved path is OS-correct without polluting `$HOME`.
pub fn ca_root_dir_unchecked() -> Result<PathBuf> {
    let home = std::env::var("HOME").ok();
    let xdg = std::env::var("XDG_DATA_HOME").ok();
    let appdata = std::env::var("APPDATA").ok();
    let env = BaseEnv {
        home: home.as_deref(),
        xdg_data_home: xdg.as_deref(),
        appdata: appdata.as_deref(),
    };
    base_data_dir_from(&env).map(|d| d.join("ca"))
}

/// Pure, env-free base data dir resolver. All OS-conditional logic lives
/// here and reads from `env` rather than process-global state, so tests can
/// exercise every branch deterministically and in parallel.
#[cfg(target_os = "linux")]
pub fn base_data_dir_from(env: &BaseEnv<'_>) -> Result<PathBuf> {
    if let Some(xdg) = env.xdg_data_home {
        if !xdg.is_empty() {
            return Ok(PathBuf::from(xdg).join("spendguard"));
        }
    }
    let home = env
        .home
        .ok_or_else(|| anyhow!("HOME not set; cannot resolve XDG data dir"))?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("spendguard"))
}

#[cfg(target_os = "macos")]
pub fn base_data_dir_from(env: &BaseEnv<'_>) -> Result<PathBuf> {
    let home = env
        .home
        .ok_or_else(|| anyhow!("HOME not set; cannot resolve Application Support dir"))?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("SpendGuard"))
}

#[cfg(target_os = "windows")]
pub fn base_data_dir_from(env: &BaseEnv<'_>) -> Result<PathBuf> {
    let appdata = env
        .appdata
        .ok_or_else(|| anyhow!("APPDATA not set; cannot resolve user data dir"))?;
    Ok(PathBuf::from(appdata).join("SpendGuard"))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub fn base_data_dir_from(_env: &BaseEnv<'_>) -> Result<PathBuf> {
    Err(anyhow!(
        "unsupported target_os for SpendGuard CLI path resolution"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // R2 fix per security review (B3): all tests below take an explicit
    // BaseEnv literal — NO process-global env mutation. Safe under cargo's
    // intra-binary parallel test execution and under Rust 2024's `unsafe
    // set_var` rule.

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_prefers_xdg_data_home_when_set() {
        let env = BaseEnv {
            home: Some("/home/u"),
            xdg_data_home: Some("/srv/xdg"),
            appdata: None,
        };
        let dir = base_data_dir_from(&env).expect("resolve");
        assert_eq!(dir, PathBuf::from("/srv/xdg/spendguard"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_falls_back_to_home_local_share_when_xdg_empty_or_missing() {
        for xdg in [None, Some("")] {
            let env = BaseEnv {
                home: Some("/home/u"),
                xdg_data_home: xdg,
                appdata: None,
            };
            let dir = base_data_dir_from(&env).expect("resolve");
            assert_eq!(dir, PathBuf::from("/home/u/.local/share/spendguard"));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_errors_without_home_or_xdg() {
        let env = BaseEnv {
            home: None,
            xdg_data_home: None,
            appdata: None,
        };
        let err = base_data_dir_from(&env).expect_err("no HOME, no XDG");
        assert!(format!("{err}").contains("HOME"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_uses_home_library_application_support() {
        let env = BaseEnv {
            home: Some("/Users/u"),
            xdg_data_home: None,
            appdata: None,
        };
        let dir = base_data_dir_from(&env).expect("resolve");
        assert_eq!(
            dir,
            PathBuf::from("/Users/u/Library/Application Support/SpendGuard")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_errors_without_home() {
        let env = BaseEnv {
            home: None,
            xdg_data_home: None,
            appdata: None,
        };
        let err = base_data_dir_from(&env).expect_err("no HOME");
        assert!(format!("{err}").contains("HOME"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_uses_appdata() {
        let env = BaseEnv {
            home: None,
            xdg_data_home: None,
            appdata: Some(r"C:\Users\test\AppData\Roaming"),
        };
        let dir = base_data_dir_from(&env).expect("resolve");
        assert_eq!(
            dir,
            PathBuf::from(r"C:\Users\test\AppData\Roaming\SpendGuard")
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_errors_without_appdata() {
        let env = BaseEnv {
            home: None,
            xdg_data_home: None,
            appdata: None,
        };
        let err = base_data_dir_from(&env).expect_err("no APPDATA");
        assert!(format!("{err}").contains("APPDATA"));
    }

    /// The exposed `ca_root_dir_unchecked` simply appends `ca` to the base.
    /// We can verify that contract without mutating env by checking the
    /// suffix of the resolved path against whatever env the test process
    /// happens to inherit (read-only access only).
    #[test]
    fn ca_root_dir_unchecked_ends_in_ca() {
        if let Ok(dir) = ca_root_dir_unchecked() {
            assert!(dir.ends_with("ca"), "expected to end with ca, got {dir:?}");
        }
        // If the test runner happens to launch with HOME / APPDATA unset
        // (uncommon but possible in some sandboxes) we simply skip — the
        // pure base_data_dir_from tests above already cover the error path.
    }
}
