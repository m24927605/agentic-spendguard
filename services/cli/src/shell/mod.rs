//! Shell rc emission backend dispatch (SLICE 5 / COV_09).
//!
//! Bridges the OS trust-store install (SLICE 2-4) to BYOK CLIs by writing a
//! marker-delimited block of `export` (or shell-equivalent) statements into
//! the per-shell rc file. Every block is bracketed by:
//!
//! ```text
//! # >>> spendguard (managed by spendguard install) >>>
//! …
//! # <<< spendguard <<<
//! ```
//!
//! Re-running install MUST replace the existing block, not append a second
//! copy. Uninstall MUST strip exactly the bracketed lines and nothing else.
//! These two invariants — idempotency + tight strip — are why we own the rc
//! mutation in-tree rather than shelling out to `>> ~/.zshrc`. The shell-out
//! approach also breaks on read-only homes and on profile files behind
//! `chezmoi` symlinks (review-standards §7 footgun #2); the in-tree writer
//! resolves the real path and uses a temp-file + atomic rename pipeline so
//! the file mode is preserved (review-standards `T2`).
//!
//! ## Why a `ShellRcWriter` trait
//!
//! Mirrors the [`crate::trust::CommandRunner`] / `TrustStore` split:
//! production callers grab the trait object from [`dispatch_writer`] and the
//! unit tests inject a `FakeRunner`-equivalent in-process writer that
//! captures `(path, content)` pairs without touching the developer's
//! `~/.zshrc`. The trait owns the rc-file mutation; the per-shell impl owns
//! the `export VAR="value"` syntax shape (POSIX vs fish vs PowerShell).
//!
//! ## `Cmd` / `Unknown` deviation #1
//!
//! The clap-facing [`crate::ShellKind`] is the user-selectable subset
//! (`bash|zsh|fish|pwsh`). `Cmd` is detection-only — we never mutate the
//! Windows `AutoRun` registry from this binary (operator runs `setx`
//! themselves, per design §4 / slice doc §3). `Unknown` is modelled as
//! `Option<DetectedShell>::None` from [`detect`] rather than as a third
//! enum variant so the dispatcher's signature stays exhaustive over the
//! happy-path shell families and the "no shell detected" error is a single
//! call site.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

pub mod posix;
pub mod windows;

/// Marker block opener — must match the closer letter-for-letter so
/// [`strip_marker_block`] can locate either end. Lives as a single constant
/// rather than two to keep the closer rendering symmetric.
pub const MARKER_BEGIN: &str = "# >>> spendguard (managed by spendguard install) >>>";
pub const MARKER_END: &str = "# <<< spendguard <<<";

/// PowerShell region markers — pwsh-native equivalents of POSIX
/// `# >>> ... >>>` blocks. PowerShell's `# region` / `# endregion`
/// are recognised by every editor we care about and avoid the `>>>`
/// braces colliding with PowerShell's own redirection syntax.
pub const PWSH_MARKER_BEGIN: &str = "# region spendguard (managed by spendguard install)";
pub const PWSH_MARKER_END: &str = "# endregion spendguard";

/// Shell families this writer understands. `Cmd` is detection-only — the
/// design's §4 last bullet says we surface the `setx` instructions in the
/// install report rather than mutate the Windows `AutoRun` registry from
/// the binary. `Unknown` is intentionally NOT a variant: detection returns
/// `Option<DetectedShell>` so the dispatcher's match arms stay finite
/// (deviation #1 in mod doc comment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectedShell {
    Bash,
    Zsh,
    Fish,
    Pwsh,
    /// Detection-only: install records breadcrumbs in `InstallReport`
    /// but does NOT mutate the registry. `dispatch_writer` returns a
    /// stub writer whose `write_rc` is a no-op that succeeds.
    Cmd,
}

impl DetectedShell {
    /// Detect the user's shell from `$SHELL` (POSIX) or
    /// `$PSModulePath` / `$ComSpec` (Windows). Best-effort: returns
    /// `None` when nothing matches so the caller can surface an
    /// actionable error or honour the `--shell` override.
    ///
    /// The Windows branch checks for PowerShell first because almost
    /// every modern Windows dev shell is pwsh — `$ComSpec` always
    /// points at `cmd.exe` even inside a PowerShell session.
    pub fn detect() -> Option<Self> {
        Self::detect_from_env(EnvView::from_process())
    }

    /// Pure detection used by tests — takes an [`EnvView`] borrow rather
    /// than reading process-global state so multiple `#[test]`s can run in
    /// parallel without `set_var` races (same pattern as
    /// [`crate::paths::base_data_dir_from`]).
    pub fn detect_from_env(env: EnvView<'_>) -> Option<Self> {
        // Explicit PowerShell signal trumps `$SHELL` — running pwsh under
        // WSL can leave `$SHELL=/bin/bash` from the underlying Linux
        // process, which would mis-route to bash otherwise.
        if env.psmodulepath.is_some() {
            return Some(DetectedShell::Pwsh);
        }
        if let Some(shell) = env.shell {
            if shell.ends_with("/zsh") || shell == "zsh" {
                return Some(DetectedShell::Zsh);
            }
            if shell.ends_with("/bash") || shell == "bash" {
                return Some(DetectedShell::Bash);
            }
            if shell.ends_with("/fish") || shell == "fish" {
                return Some(DetectedShell::Fish);
            }
            if shell.ends_with("/pwsh")
                || shell == "pwsh"
                || shell.ends_with("/powershell")
                || shell == "powershell"
            {
                return Some(DetectedShell::Pwsh);
            }
            if shell.ends_with("cmd.exe") || shell == "cmd" {
                return Some(DetectedShell::Cmd);
            }
        }
        // ComSpec is the Windows-native fallback when neither `$SHELL`
        // nor `$PSModulePath` is set — happens inside legacy `cmd.exe`
        // sessions.
        if env.comspec.is_some() {
            return Some(DetectedShell::Cmd);
        }
        None
    }

    /// Resolve the rc-file path for this shell rooted at `home`. Returns
    /// `None` for `Cmd` (no file mutation), so the install report can
    /// distinguish "wrote rc at <path>" from "left breadcrumb for setx".
    pub fn rc_path(&self, home: &Path) -> Option<PathBuf> {
        match self {
            DetectedShell::Bash => Some(home.join(".bashrc")),
            DetectedShell::Zsh => Some(home.join(".zshrc")),
            DetectedShell::Fish => Some(
                home.join(".config")
                    .join("fish")
                    .join("conf.d")
                    .join("spendguard.fish"),
            ),
            DetectedShell::Pwsh => Some(
                // `$PROFILE` resolution: on macOS / Linux pwsh users keep
                // it at `~/.config/powershell/Microsoft.PowerShell_profile.ps1`;
                // on Windows it's `~/Documents/PowerShell/Profile.ps1`.
                // We pick the OS-correct fallback so the writer can land
                // a profile even when pwsh isn't running to resolve
                // `$PROFILE` for us.
                pwsh_profile_path(home),
            ),
            DetectedShell::Cmd => None,
        }
    }
}

/// Borrowed view of the env vars [`DetectedShell::detect_from_env`] needs.
/// Pure-function input lets tests construct env without `std::env::set_var`
/// (same B3 pattern as `paths::BaseEnv`).
#[derive(Debug, Clone, Copy, Default)]
pub struct EnvView<'a> {
    pub shell: Option<&'a str>,
    pub psmodulepath: Option<&'a str>,
    pub comspec: Option<&'a str>,
}

impl EnvView<'_> {
    /// Snapshot the relevant process env vars. Only used by the
    /// production `detect()` call site; tests construct literals.
    fn from_process() -> EnvView<'static> {
        // Leak the env strings into 'static — only called once per process
        // by `detect()`, so the leak is bounded by the number of `--shell
        // auto-detect` install runs.
        fn leak(s: String) -> &'static str {
            Box::leak(s.into_boxed_str())
        }
        EnvView {
            shell: std::env::var("SHELL").ok().map(leak),
            psmodulepath: std::env::var("PSModulePath").ok().map(leak),
            comspec: std::env::var("ComSpec").ok().map(leak),
        }
    }
}

#[cfg(target_os = "windows")]
fn pwsh_profile_path(home: &Path) -> PathBuf {
    home.join("Documents")
        .join("PowerShell")
        .join("Profile.ps1")
}

#[cfg(not(target_os = "windows"))]
fn pwsh_profile_path(home: &Path) -> PathBuf {
    home.join(".config")
        .join("powershell")
        .join("Microsoft.PowerShell_profile.ps1")
}

/// In-tree rc-file writer. Owns the "write block / strip block" mutation;
/// per-shell impls own the `export VAR="value"` syntax line shape.
pub trait ShellRcWriter: Send + Sync {
    /// Detected shell kind — drives the marker variant (POSIX vs `# region`).
    fn kind(&self) -> DetectedShell;

    /// Render a single env-var assignment in this shell's native syntax.
    /// Must produce ONE line (no trailing newline — the writer joins).
    /// Implementations MUST quote `value` to prevent shell injection if
    /// the value somehow contains a quote or `;` (review-standards `T8`).
    fn render_line(&self, name: &str, value: &str) -> String;

    /// Write the marker-bracketed block of `vars` into `rc_path`. The
    /// writer:
    ///   1. Reads the existing file (or starts from empty).
    ///   2. Strips any prior SpendGuard marker block — idempotency.
    ///   3. Appends the new block.
    ///   4. Atomically renames over `rc_path`, preserving mode.
    fn write_rc(&self, rc_path: &Path, vars: &[(String, String)]) -> Result<()> {
        let begin = self.marker_begin();
        let end = self.marker_end();

        let existing = match std::fs::read_to_string(rc_path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                return Err(e).with_context(|| format!("read rc file {}", rc_path.display()));
            }
        };
        let stripped = strip_marker_block(&existing, begin, end);

        let mut block = String::new();
        block.push_str(begin);
        block.push('\n');
        for (name, value) in vars {
            block.push_str(&self.render_line(name, value));
            block.push('\n');
        }
        block.push_str(end);
        block.push('\n');

        let mut out = stripped;
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&block);

        atomic_write_preserving_mode(rc_path, out.as_bytes())
            .with_context(|| format!("atomic write rc {}", rc_path.display()))?;
        Ok(())
    }

    /// Strip the SpendGuard marker block from `rc_path` (uninstall path).
    /// Idempotent: if the block is absent the call is a no-op and returns
    /// `Ok(())`. If the rc file itself is absent that's also `Ok(())`.
    fn strip_rc(&self, rc_path: &Path) -> Result<()> {
        let begin = self.marker_begin();
        let end = self.marker_end();

        let existing = match std::fs::read_to_string(rc_path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                return Err(e).with_context(|| format!("read rc file {}", rc_path.display()));
            }
        };
        let stripped = strip_marker_block(&existing, begin, end);
        if stripped == existing {
            // Nothing to do — block absent.
            return Ok(());
        }
        atomic_write_preserving_mode(rc_path, stripped.as_bytes())
            .with_context(|| format!("atomic strip rc {}", rc_path.display()))?;
        Ok(())
    }

    fn marker_begin(&self) -> &'static str {
        match self.kind() {
            DetectedShell::Pwsh => PWSH_MARKER_BEGIN,
            _ => MARKER_BEGIN,
        }
    }

    fn marker_end(&self) -> &'static str {
        match self.kind() {
            DetectedShell::Pwsh => PWSH_MARKER_END,
            _ => MARKER_END,
        }
    }
}

/// Pick the writer for `kind`. `Cmd` returns a stub that no-ops with a
/// breadcrumb error if the caller tries to write — by contract, `Cmd`
/// users are steered to `setx` instructions in the install report and
/// `write_rc` is not invoked.
pub fn dispatch_writer(kind: DetectedShell) -> Box<dyn ShellRcWriter> {
    match kind {
        DetectedShell::Bash => Box::new(posix::PosixWriter::bash()),
        DetectedShell::Zsh => Box::new(posix::PosixWriter::zsh()),
        DetectedShell::Fish => Box::new(posix::FishWriter::new()),
        DetectedShell::Pwsh => Box::new(windows::PwshWriter::new()),
        DetectedShell::Cmd => Box::new(windows::CmdStubWriter::new()),
    }
}

/// Strip the FIRST marker block found between `begin` and `end`. Leaves
/// the rest of the file untouched, including any whitespace before/after
/// the block. Used by both `write_rc` (replace) and `strip_rc` (delete).
///
/// Tolerates a malformed file where the begin marker is present but the
/// end marker is missing — in that case we leave the file as-is so we
/// don't accidentally truncate user content. The caller's `write_rc`
/// re-appends a fresh block at the tail, which the user can manually
/// reconcile if the original block was corrupted.
fn strip_marker_block(content: &str, begin: &str, end: &str) -> String {
    let Some(begin_idx) = content.find(begin) else {
        return content.to_string();
    };
    // The end marker MUST come AFTER the begin marker; scan from there.
    let search_from = begin_idx + begin.len();
    let Some(rel_end_idx) = content[search_from..].find(end) else {
        // Malformed — preserve the file as-is rather than truncating.
        return content.to_string();
    };
    let end_idx = search_from + rel_end_idx + end.len();
    // Consume the trailing newline that follows the end marker so we don't
    // leave a blank line behind on strip_rc.
    let mut tail_start = end_idx;
    if content.as_bytes().get(tail_start) == Some(&b'\n') {
        tail_start += 1;
    }
    // Consume the preceding newline only if it would leave a stranded
    // blank line at the begin position.
    let mut head_end = begin_idx;
    if head_end > 0 && content.as_bytes().get(head_end - 1) == Some(&b'\n') {
        // Keep the prior line's newline only if there is real content
        // before it.
        let stripped_prefix = &content[..head_end];
        if stripped_prefix.trim_end().is_empty() {
            head_end -= 1;
        }
    }
    let mut out = String::with_capacity(content.len());
    out.push_str(&content[..head_end]);
    out.push_str(&content[tail_start..]);
    out
}

/// Atomic write that preserves the source file's mode on POSIX. Mirrors
/// `lib::write_secret` but does NOT clamp mode to 0o600 — rc files are
/// owner-readable / group-readable by convention (0o644).
fn atomic_write_preserving_mode(path: &Path, bytes: &[u8]) -> Result<()> {
    // Resolve the real path so `chezmoi`-style symlinks land their write
    // on the actual file (review-standards §7 footgun #2).
    let real_path = match std::fs::canonicalize(path) {
        Ok(p) => p,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => path.to_path_buf(),
        Err(e) => return Err(e).with_context(|| format!("canonicalize {}", path.display())),
    };
    if let Some(parent) = real_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create rc parent dir {}", parent.display()))?;
    }

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let existing_mode = std::fs::metadata(&real_path)
            .map(|m| m.permissions().mode() & 0o777)
            .unwrap_or(0o644);

        let tmp_path = tmp_sibling_for(&real_path);
        let mut opts = std::fs::OpenOptions::new();
        opts.create_new(true).write(true).mode(existing_mode);
        let mut f = opts
            .open(&tmp_path)
            .with_context(|| format!("open temp {} for rc write", tmp_path.display()))?;
        f.write_all(bytes)
            .with_context(|| format!("write temp {}", tmp_path.display()))?;
        f.sync_all()
            .with_context(|| format!("sync temp {}", tmp_path.display()))?;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(existing_mode))
            .with_context(|| format!("clamp mode on {}", tmp_path.display()))?;
        if let Err(e) = std::fs::rename(&tmp_path, &real_path) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e).with_context(|| {
                format!("rename {} -> {}", tmp_path.display(), real_path.display())
            });
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&real_path, bytes)
            .with_context(|| format!("write {}", real_path.display()))?;
        Ok(())
    }
}

#[cfg(unix)]
fn tmp_sibling_for(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "rc".to_string());
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let tmp_name = format!(".{file_name}.tmp-{}-{suffix}", std::process::id());
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    parent.join(tmp_name)
}

/// Map [`crate::ShellKind`] (the clap value-enum surface) onto the
/// detection-side [`DetectedShell`]. `--shell` flag → writer routing.
impl From<crate::ShellKind> for DetectedShell {
    fn from(value: crate::ShellKind) -> Self {
        match value {
            crate::ShellKind::Bash => DetectedShell::Bash,
            crate::ShellKind::Zsh => DetectedShell::Zsh,
            crate::ShellKind::Fish => DetectedShell::Fish,
            crate::ShellKind::Pwsh => DetectedShell::Pwsh,
        }
    }
}

/// Surface the "no shell detected, no `--shell` override" failure mode in
/// a single place so the install path and tests share the message body.
pub fn unknown_shell_error() -> anyhow::Error {
    anyhow!(
        "shell rc emission: unable to detect a supported shell (bash, zsh, fish, pwsh, cmd) \
         from $SHELL / $PSModulePath / $ComSpec; pass --shell to select one explicitly"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_zsh_from_shell_var() {
        let env = EnvView {
            shell: Some("/bin/zsh"),
            ..Default::default()
        };
        assert_eq!(
            DetectedShell::detect_from_env(env),
            Some(DetectedShell::Zsh)
        );
    }

    #[test]
    fn detect_bash_from_shell_var() {
        let env = EnvView {
            shell: Some("/usr/bin/bash"),
            ..Default::default()
        };
        assert_eq!(
            DetectedShell::detect_from_env(env),
            Some(DetectedShell::Bash)
        );
    }

    #[test]
    fn detect_fish_from_shell_var() {
        let env = EnvView {
            shell: Some("/opt/homebrew/bin/fish"),
            ..Default::default()
        };
        assert_eq!(
            DetectedShell::detect_from_env(env),
            Some(DetectedShell::Fish)
        );
    }

    #[test]
    fn detect_pwsh_prefers_psmodulepath_over_shell() {
        // WSL pwsh leaves $SHELL=/bin/bash even though the user is in pwsh.
        let env = EnvView {
            shell: Some("/bin/bash"),
            psmodulepath: Some(r"C:\Program Files\PowerShell\Modules"),
            comspec: None,
        };
        assert_eq!(
            DetectedShell::detect_from_env(env),
            Some(DetectedShell::Pwsh)
        );
    }

    #[test]
    fn detect_cmd_from_comspec_when_shell_unset() {
        let env = EnvView {
            shell: None,
            psmodulepath: None,
            comspec: Some(r"C:\Windows\System32\cmd.exe"),
        };
        assert_eq!(
            DetectedShell::detect_from_env(env),
            Some(DetectedShell::Cmd)
        );
    }

    #[test]
    fn detect_returns_none_when_all_env_vars_unset() {
        assert_eq!(DetectedShell::detect_from_env(EnvView::default()), None);
    }

    #[test]
    fn rc_path_resolution_per_shell() {
        let home = Path::new("/home/u");
        assert_eq!(
            DetectedShell::Bash.rc_path(home),
            Some(PathBuf::from("/home/u/.bashrc"))
        );
        assert_eq!(
            DetectedShell::Zsh.rc_path(home),
            Some(PathBuf::from("/home/u/.zshrc"))
        );
        assert_eq!(
            DetectedShell::Fish.rc_path(home),
            Some(PathBuf::from("/home/u/.config/fish/conf.d/spendguard.fish"))
        );
        // Cmd has no rc path (operator runs setx).
        assert_eq!(DetectedShell::Cmd.rc_path(home), None);
        // Pwsh has a non-None path; exact content is OS-dependent.
        assert!(DetectedShell::Pwsh.rc_path(home).is_some());
    }

    #[test]
    fn strip_marker_block_removes_only_block() {
        let content = "alias l='ls -la'\n\
                       # >>> spendguard (managed by spendguard install) >>>\n\
                       export HTTPS_PROXY=\"https://localhost:8443\"\n\
                       # <<< spendguard <<<\n\
                       export PATH=\"$PATH:$HOME/bin\"\n";
        let stripped = strip_marker_block(content, MARKER_BEGIN, MARKER_END);
        assert_eq!(
            stripped,
            "alias l='ls -la'\nexport PATH=\"$PATH:$HOME/bin\"\n"
        );
    }

    #[test]
    fn strip_marker_block_preserves_when_absent() {
        let content = "alias l='ls -la'\n";
        assert_eq!(
            strip_marker_block(content, MARKER_BEGIN, MARKER_END),
            content
        );
    }

    #[test]
    fn strip_marker_block_recovers_from_missing_end_marker() {
        // Malformed: begin without end. We MUST preserve the file rather
        // than truncating to the begin marker — the user can fix the file
        // manually and re-run install.
        let content = "alias l='ls -la'\n\
                       # >>> spendguard (managed by spendguard install) >>>\n\
                       export HTTPS_PROXY=\"https://localhost:8443\"\n\
                       export PATH=\"$PATH:$HOME/bin\"\n";
        let stripped = strip_marker_block(content, MARKER_BEGIN, MARKER_END);
        assert_eq!(stripped, content, "malformed must preserve file");
    }

    #[test]
    fn unknown_shell_error_mentions_override_flag() {
        let err = unknown_shell_error();
        assert!(format!("{err:#}").contains("--shell"));
    }

    #[test]
    fn from_clap_shell_kind_round_trips() {
        assert_eq!(
            DetectedShell::from(crate::ShellKind::Bash),
            DetectedShell::Bash
        );
        assert_eq!(
            DetectedShell::from(crate::ShellKind::Zsh),
            DetectedShell::Zsh
        );
        assert_eq!(
            DetectedShell::from(crate::ShellKind::Fish),
            DetectedShell::Fish
        );
        assert_eq!(
            DetectedShell::from(crate::ShellKind::Pwsh),
            DetectedShell::Pwsh
        );
    }
}
