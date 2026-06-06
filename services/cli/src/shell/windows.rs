//! Windows shell rc writers — PowerShell + `cmd.exe` stub.
//!
//! ## PowerShell ([`PwshWriter`])
//!
//! Writes a `# region spendguard` / `# endregion spendguard` block into the
//! resolved `$PROFILE` path. PowerShell profile resolution is normally
//! deferred to pwsh itself (`pwsh -NoLogo -NoProfile -Command '$PROFILE'`),
//! but we avoid a shell-out at install time by computing the OS-correct
//! fallback in [`super::pwsh_profile_path`]:
//!
//! - Windows: `~/Documents/PowerShell/Profile.ps1`
//! - non-Windows pwsh: `~/.config/powershell/Microsoft.PowerShell_profile.ps1`
//!
//! Either matches the path `pwsh` itself returns from `$PROFILE` on a
//! default install. Operators with a customised `$PROFILE` can pass
//! `--shell` plus a target profile via a follow-up env knob (out of scope
//! for SLICE 5; spec §4 doesn't ship it).
//!
//! ## `cmd.exe` ([`CmdStubWriter`])
//!
//! Per design §4 last bullet + slice doc §3, `cmd` is detection-only — we
//! do NOT mutate the Windows registry's `AutoRun` value from this binary
//! (that would silently affect every future `cmd.exe` session, which is
//! the wrong default for an installer that the operator can `Ctrl-C` out
//! of). Instead the install report carries a breadcrumb instructing the
//! operator to run `setx HTTPS_PROXY https://localhost:8443` from an
//! elevated shell. `write_rc` on the stub is a no-op success; `strip_rc`
//! is likewise a no-op (uninstall just clears the report breadcrumb).
//!
//! ## `T8` quoting
//!
//! PowerShell values are wrapped in single quotes so `$variable`
//! expansion does NOT happen — the value lands verbatim. Embedded `'`
//! characters are doubled (`''`), which is PowerShell's literal-string
//! escape per `about_Quoting_Rules`.

use super::{DetectedShell, ShellRcWriter};

pub struct PwshWriter;

impl PwshWriter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PwshWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl ShellRcWriter for PwshWriter {
    fn kind(&self) -> DetectedShell {
        DetectedShell::Pwsh
    }

    fn render_line(&self, name: &str, value: &str) -> String {
        // Single-quoted PowerShell literal: doubles embedded `'` per
        // about_Quoting_Rules. No variable expansion happens in '…'.
        let escaped = value.replace('\'', "''");
        format!("$env:{name} = '{escaped}'")
    }
}

/// `cmd.exe` detection-only stub. `write_rc` / `strip_rc` are no-ops; the
/// install/uninstall driver reads `kind() == DetectedShell::Cmd` and emits
/// the `setx` breadcrumb into `InstallReport.shell_rc_paths` instead of a
/// real file path.
pub struct CmdStubWriter;

impl CmdStubWriter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CmdStubWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl ShellRcWriter for CmdStubWriter {
    fn kind(&self) -> DetectedShell {
        DetectedShell::Cmd
    }

    fn render_line(&self, name: &str, value: &str) -> String {
        // `cmd.exe` syntax — emitted only for the install-report breadcrumb;
        // never written to a file from this binary.
        format!("setx {name} \"{value}\"")
    }

    fn write_rc(
        &self,
        _rc_path: &std::path::Path,
        _vars: &[(String, String)],
    ) -> anyhow::Result<()> {
        // Documented no-op: caller is expected to read kind() == Cmd and
        // emit the breadcrumb into InstallReport.shell_rc_paths instead of
        // routing through write_rc.
        Ok(())
    }

    fn strip_rc(&self, _rc_path: &std::path::Path) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{PWSH_MARKER_BEGIN, PWSH_MARKER_END};
    use super::*;
    use crate::shell::ShellRcWriter;

    fn vars() -> Vec<(String, String)> {
        vec![
            ("HTTPS_PROXY".into(), "https://localhost:8443".into()),
            (
                "NODE_EXTRA_CA_CERTS".into(),
                r"C:\Users\u\AppData\Roaming\SpendGuard\ca\root_ca.pem".into(),
            ),
        ]
    }

    #[test]
    fn pwsh_write_rc_uses_env_assignment_syntax() {
        let dir = tempfile::tempdir().expect("tempdir");
        let profile = dir.path().join("Profile.ps1");
        let writer = PwshWriter::new();
        writer.write_rc(&profile, &vars()).expect("write profile");
        let content = std::fs::read_to_string(&profile).expect("read");
        // Must use PowerShell region markers, not POSIX `>>>` braces.
        assert!(content.contains(PWSH_MARKER_BEGIN));
        assert!(content.contains(PWSH_MARKER_END));
        // PowerShell env var syntax.
        assert!(content.contains("$env:HTTPS_PROXY = 'https://localhost:8443'"));
        assert!(content.contains("$env:NODE_EXTRA_CA_CERTS = '"));
        // Must NOT use bash export syntax.
        assert!(
            !content.contains("export HTTPS_PROXY"),
            "pwsh must not emit POSIX export"
        );
    }

    #[test]
    fn pwsh_render_line_escapes_embedded_single_quote() {
        let writer = PwshWriter::new();
        let line = writer.render_line("FOO", "a'b");
        // PowerShell single-quoted literal: doubled '' is the escape.
        assert!(line.contains("'a''b'"), "got: {line}");
    }

    #[test]
    fn pwsh_strip_rc_removes_region_block() {
        let dir = tempfile::tempdir().expect("tempdir");
        let profile = dir.path().join("Profile.ps1");
        std::fs::write(&profile, "function prompt { 'PS> ' }\n").expect("seed");

        let writer = PwshWriter::new();
        writer.write_rc(&profile, &vars()).expect("write");
        writer.strip_rc(&profile).expect("strip");
        let content = std::fs::read_to_string(&profile).expect("read");
        assert!(!content.contains(PWSH_MARKER_BEGIN));
        assert!(!content.contains("HTTPS_PROXY"));
        assert!(
            content.contains("function prompt"),
            "user content preserved"
        );
    }

    #[test]
    fn cmd_stub_write_rc_is_noop() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("autoexec.bat");
        let writer = CmdStubWriter::new();
        // The stub MUST succeed without creating the file — caller is
        // expected to use the breadcrumb path, not the file path.
        writer.write_rc(&path, &vars()).expect("noop ok");
        assert!(!path.exists(), "cmd stub must not write a file");
    }

    #[test]
    fn cmd_stub_render_line_emits_setx_breadcrumb() {
        let writer = CmdStubWriter::new();
        let line = writer.render_line("HTTPS_PROXY", "https://localhost:8443");
        assert_eq!(line, r#"setx HTTPS_PROXY "https://localhost:8443""#);
    }
}
