//! POSIX shell rc writers — bash, zsh, fish.
//!
//! Bash and zsh accept identical `export VAR="value"` syntax so they share
//! a single [`PosixWriter`] with a [`DetectedShell`] discriminant carried
//! for `marker_begin` / `marker_end` symmetry (both use the POSIX `>>>` /
//! `<<<` markers).
//!
//! Fish is the odd one — it does NOT understand `export`. The fish
//! equivalent is `set -x VAR value`, and fish profile files live under
//! `~/.config/fish/conf.d/spendguard.fish` rather than a `~/.bashrc`
//! analogue. That conf.d sourcing model lets us drop a dedicated
//! `spendguard.fish` per-user file with no merge step against
//! `config.fish` — install puts it there, uninstall deletes the file (or
//! strips the marker block if other tools share the file, but in practice
//! fish users have one conf.d file per tool).
//!
//! ## Quoting / `T8` no-shell-injection
//!
//! `render_line` wraps `value` in double quotes for bash/zsh and single
//! quotes for fish, escaping any embedded quote character. This mirrors
//! how reputable installers (rustup, nvm) write rc lines. The values we
//! actually write — URLs, file-system paths from `paths::ca_root_dir()`
//! — never contain a `"` in practice, but the escape closes the door on
//! a hostile env-var name from a future override-table extension.

use super::{DetectedShell, ShellRcWriter};

/// Shared bash + zsh writer — identical `export NAME="value"` syntax.
pub struct PosixWriter {
    kind: DetectedShell,
}

impl PosixWriter {
    pub fn bash() -> Self {
        Self {
            kind: DetectedShell::Bash,
        }
    }

    pub fn zsh() -> Self {
        Self {
            kind: DetectedShell::Zsh,
        }
    }
}

impl ShellRcWriter for PosixWriter {
    fn kind(&self) -> DetectedShell {
        self.kind
    }

    fn render_line(&self, name: &str, value: &str) -> String {
        // `T8`: escape any embedded `"` so the value stays safely inside
        // the double-quoted shell string. POSIX `\"` works in both bash
        // and zsh.
        let escaped = value.replace('"', r#"\""#);
        format!(r#"export {name}="{escaped}""#)
    }
}

/// Fish writer — `set -x VAR value` syntax, conf.d sourcing path.
pub struct FishWriter;

impl FishWriter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for FishWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl ShellRcWriter for FishWriter {
    fn kind(&self) -> DetectedShell {
        DetectedShell::Fish
    }

    fn render_line(&self, name: &str, value: &str) -> String {
        // Fish uses single-quoted strings — backslash-escape any embedded
        // `'` per fish's manual on string literals.
        let escaped = value.replace('\'', r"\'");
        format!("set -x {name} '{escaped}'")
    }
}

#[cfg(test)]
mod tests {
    use super::super::{MARKER_BEGIN, MARKER_END};
    use super::*;
    use crate::shell::ShellRcWriter;
    use std::path::PathBuf;

    fn vars() -> Vec<(String, String)> {
        vec![
            ("HTTPS_PROXY".into(), "https://localhost:8443".into()),
            (
                "NODE_EXTRA_CA_CERTS".into(),
                "/home/u/.local/share/spendguard/ca/root_ca.pem".into(),
            ),
        ]
    }

    #[test]
    fn bash_write_rc_emits_marker_block_with_export_syntax() {
        let dir = tempfile::tempdir().expect("tempdir");
        let rc = dir.path().join(".bashrc");
        let writer = PosixWriter::bash();
        writer.write_rc(&rc, &vars()).expect("write rc");
        let content = std::fs::read_to_string(&rc).expect("read");
        assert!(content.contains(MARKER_BEGIN));
        assert!(content.contains(MARKER_END));
        assert!(content.contains(r#"export HTTPS_PROXY="https://localhost:8443""#));
        assert!(content.contains("export NODE_EXTRA_CA_CERTS="));
    }

    #[test]
    fn zsh_write_rc_uses_posix_export_syntax() {
        let dir = tempfile::tempdir().expect("tempdir");
        let rc = dir.path().join(".zshrc");
        let writer = PosixWriter::zsh();
        writer.write_rc(&rc, &vars()).expect("write rc");
        let content = std::fs::read_to_string(&rc).expect("read");
        // `T8`: never `set -x` — that's fish syntax, would break zsh.
        assert!(!content.contains("set -x"));
        assert!(content.contains("export HTTPS_PROXY="));
    }

    #[test]
    fn fish_write_rc_uses_set_x_syntax_not_export() {
        let dir = tempfile::tempdir().expect("tempdir");
        let rc = dir
            .path()
            .join(".config")
            .join("fish")
            .join("conf.d")
            .join("spendguard.fish");
        let writer = FishWriter::new();
        writer.write_rc(&rc, &vars()).expect("write rc");
        let content = std::fs::read_to_string(&rc).expect("read");
        // Fish must NOT use `export` — that's the regression we're guarding.
        assert!(
            !content.contains("export HTTPS_PROXY"),
            "fish must use set -x, not export; got:\n{content}"
        );
        assert!(content.contains("set -x HTTPS_PROXY 'https://localhost:8443'"));
        assert!(content.contains("set -x NODE_EXTRA_CA_CERTS '"));
    }

    /// Idempotency: re-running `write_rc` over a file that already has a
    /// SpendGuard block MUST replace, not append a duplicate.
    #[test]
    fn write_rc_is_idempotent_replaces_existing_block() {
        let dir = tempfile::tempdir().expect("tempdir");
        let rc = dir.path().join(".bashrc");
        // Seed with unrelated content + a stale SpendGuard block.
        let seed = format!(
            "alias l='ls -la'\n{MARKER_BEGIN}\nexport STALE=\"old-value\"\n{MARKER_END}\nexport PATH=\"/usr/bin\"\n"
        );
        std::fs::write(&rc, &seed).expect("seed");

        let writer = PosixWriter::bash();
        writer.write_rc(&rc, &vars()).expect("write rc twice");
        writer.write_rc(&rc, &vars()).expect("re-write rc");

        let content = std::fs::read_to_string(&rc).expect("read");
        // Block appears exactly once.
        let begin_count = content.matches(MARKER_BEGIN).count();
        assert_eq!(begin_count, 1, "expected 1 begin marker, got {begin_count}");
        let end_count = content.matches(MARKER_END).count();
        assert_eq!(end_count, 1, "expected 1 end marker, got {end_count}");
        // Pre-existing unrelated content is preserved.
        assert!(content.contains("alias l='ls -la'"));
        assert!(content.contains(r#"export PATH="/usr/bin""#));
        // The stale STALE var is GONE — the previous block was replaced.
        assert!(
            !content.contains("STALE"),
            "stale block must be stripped on re-write"
        );
    }

    /// strip_rc removes ONLY the SpendGuard block, preserves the rest.
    #[test]
    fn strip_rc_removes_only_marker_block_preserves_rest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let rc = dir.path().join(".bashrc");
        let writer = PosixWriter::bash();
        writer.write_rc(&rc, &vars()).expect("write");
        // Add unrelated content the user types post-install.
        let after = format!(
            "{}\nalias g='git'\n",
            std::fs::read_to_string(&rc).expect("read")
        );
        std::fs::write(&rc, &after).expect("rewrite");

        writer.strip_rc(&rc).expect("strip");
        let content = std::fs::read_to_string(&rc).expect("read post-strip");
        assert!(!content.contains(MARKER_BEGIN));
        assert!(!content.contains(MARKER_END));
        assert!(!content.contains("HTTPS_PROXY"));
        assert!(content.contains("alias g='git'"), "user alias preserved");
    }

    /// strip_rc on a file with no marker block is a no-op (doesn't error,
    /// doesn't touch the file).
    #[test]
    fn strip_rc_is_noop_when_block_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let rc = dir.path().join(".bashrc");
        let seed = "alias l='ls -la'\n";
        std::fs::write(&rc, seed).expect("seed");
        let writer = PosixWriter::bash();
        writer.strip_rc(&rc).expect("strip absent");
        assert_eq!(std::fs::read_to_string(&rc).expect("read"), seed);
    }

    /// strip_rc on a missing rc file is also a no-op.
    #[test]
    fn strip_rc_is_noop_when_rc_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let rc: PathBuf = dir.path().join("never-existed.rc");
        let writer = PosixWriter::bash();
        writer.strip_rc(&rc).expect("strip absent rc");
        assert!(!rc.exists());
    }

    /// `T2` regression: atomic temp+rename pipeline preserves the file's
    /// existing mode bits across the rewrite.
    #[cfg(unix)]
    #[test]
    fn write_rc_preserves_file_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let rc = dir.path().join(".bashrc");
        std::fs::write(&rc, "alias l='ls'\n").expect("seed");
        // User keeps their rc at 0o600 for paranoia — we must not silently
        // widen it to 0o644.
        std::fs::set_permissions(&rc, std::fs::Permissions::from_mode(0o600))
            .expect("set 0o600 seed");

        let writer = PosixWriter::bash();
        writer.write_rc(&rc, &vars()).expect("write rc");

        let mode = std::fs::metadata(&rc)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "expected mode preserved at 0600, got {mode:o}");
    }

    /// Quoting: a value containing a double quote MUST be escaped — defends
    /// `T8` against a future tool-override entry whose path or URL carries
    /// a literal quote character.
    #[test]
    fn bash_render_line_escapes_embedded_double_quote() {
        let writer = PosixWriter::bash();
        let line = writer.render_line("FOO", r#"a"b"#);
        assert!(line.contains(r#""a\"b""#), "got: {line}");
        // Sanity: the surrounding quotes are still balanced.
        assert!(line.starts_with(r#"export FOO=""#));
        assert!(line.ends_with('"'));
    }

    /// Fish quoting: single-quote escapes for embedded `'`.
    #[test]
    fn fish_render_line_escapes_embedded_single_quote() {
        let writer = FishWriter::new();
        let line = writer.render_line("FOO", "a'b");
        assert!(line.contains(r"'a\'b'"), "got: {line}");
    }
}
