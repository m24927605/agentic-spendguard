//! SLICE 7 (COV_11) — Shell rc inspection probe.
//!
//! Read-only: detects the user's shell from the injected [`EnvView`],
//! resolves the rc path under the injected `BaseEnv::home`, opens the
//! file, and greps for the SpendGuard marker block from SLICE 5
//! ([`crate::shell::MARKER_BEGIN`] / [`crate::shell::MARKER_END`] or
//! the PowerShell `# region` variants for `Pwsh`).
//!
//! When found, also surfaces the `(name, value)` env pairs found inside
//! the block so the renderer can confirm exactly which env vars the
//! operator's shell will export. We use a deliberately conservative
//! regex (`<name>="<value>"`) so we don't accidentally surface
//! attacker-controlled strings — only quoted-value POSIX exports
//! contribute to the parsed pairs.
//!
//! ## Anti-scope
//!
//! - No mutation. We do NOT update the rc file even if we detect a
//!   stale block.
//! - No `setx` breadcrumb evaluation on Windows — that's an operator-
//!   visible breadcrumb in the install report, not something we probe.

use crate::preflight::BaseEnv;
use crate::shell::{
    DetectedShell, EnvView, MARKER_BEGIN, MARKER_END, PWSH_MARKER_BEGIN, PWSH_MARKER_END,
};
use std::path::PathBuf;

/// Outcome of the shell-rc probe.
#[derive(Debug, Clone)]
pub enum ShellRcCheckResult {
    /// Marker block present.
    Present {
        rc_path: PathBuf,
        env_vars: Vec<(String, String)>,
    },
    /// Marker block absent — either no rc file, or rc file present but
    /// no SpendGuard block inside. `reason` distinguishes the two for
    /// rendering.
    Absent { reason: AbsentReason },
}

#[derive(Debug, Clone)]
pub enum AbsentReason {
    NoShellDetected,
    NoHomeResolvable,
    RcPathNotApplicable { kind: DetectedShell },
    RcFileNotFound { rc_path: PathBuf },
    MarkerBlockMissing { rc_path: PathBuf },
    RcFileUnreadable { rc_path: PathBuf, error: String },
}

impl ShellRcCheckResult {
    /// One-line render.
    pub fn render(&self, use_color: bool) -> String {
        use crate::doctor::{paint, Color};
        match self {
            Self::Present { rc_path, env_vars } => {
                let head = paint("OK", Color::Green, use_color);
                let var_names = env_vars
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let vars_section = if var_names.is_empty() {
                    String::new()
                } else {
                    format!(" — exports: {var_names}")
                };
                format!("{head} marker block in {}{vars_section}", rc_path.display())
            }
            Self::Absent { reason } => {
                let head = paint("WARN", Color::Yellow, use_color);
                let body = match reason {
                    AbsentReason::NoShellDetected => {
                        "no shell detected from $SHELL / $PSModulePath / $ComSpec".to_string()
                    }
                    AbsentReason::NoHomeResolvable => "HOME / USERPROFILE not set".to_string(),
                    AbsentReason::RcPathNotApplicable { kind } => {
                        format!("{kind:?} shell uses an out-of-band mechanism (setx breadcrumb)")
                    }
                    AbsentReason::RcFileNotFound { rc_path } => {
                        format!(
                            "no rc file at {} — run `spendguard install`",
                            rc_path.display()
                        )
                    }
                    AbsentReason::MarkerBlockMissing { rc_path } => {
                        format!(
                            "rc file at {} has no SpendGuard marker block — \
                             re-run `spendguard install`",
                            rc_path.display()
                        )
                    }
                    AbsentReason::RcFileUnreadable { rc_path, error } => {
                        format!("rc file at {} unreadable: {error}", rc_path.display())
                    }
                };
                format!("{head} {body}")
            }
        }
    }
}

/// Probe the shell rc. `shell_env=None` snapshots from process env;
/// tests pass `Some(literal)` to drive deterministic detection.
pub fn check(env: &BaseEnv<'_>, shell_env: Option<EnvView<'_>>) -> ShellRcCheckResult {
    let shell_env = shell_env.unwrap_or_default();
    let kind = match DetectedShell::detect_from_env(shell_env) {
        Some(k) => k,
        None => {
            return ShellRcCheckResult::Absent {
                reason: AbsentReason::NoShellDetected,
            };
        }
    };
    let home = match env.home {
        Some(h) => h,
        None => {
            return ShellRcCheckResult::Absent {
                reason: AbsentReason::NoHomeResolvable,
            };
        }
    };
    let rc_path = match kind.rc_path(home) {
        Some(p) => p,
        None => {
            return ShellRcCheckResult::Absent {
                reason: AbsentReason::RcPathNotApplicable { kind },
            };
        }
    };

    let content = match std::fs::read_to_string(&rc_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ShellRcCheckResult::Absent {
                reason: AbsentReason::RcFileNotFound { rc_path },
            };
        }
        Err(e) => {
            return ShellRcCheckResult::Absent {
                reason: AbsentReason::RcFileUnreadable {
                    rc_path,
                    error: e.to_string(),
                },
            };
        }
    };

    let (begin, end) = match kind {
        DetectedShell::Pwsh => (PWSH_MARKER_BEGIN, PWSH_MARKER_END),
        _ => (MARKER_BEGIN, MARKER_END),
    };

    let begin_idx = match content.find(begin) {
        Some(i) => i,
        None => {
            return ShellRcCheckResult::Absent {
                reason: AbsentReason::MarkerBlockMissing { rc_path },
            };
        }
    };
    let after_begin = begin_idx + begin.len();
    let end_idx = match content[after_begin..].find(end) {
        Some(rel) => after_begin + rel,
        None => {
            // Malformed (begin without end). Per SLICE 5 contract we
            // surface this as "marker block missing" rather than
            // attempting recovery — doctor is read-only.
            return ShellRcCheckResult::Absent {
                reason: AbsentReason::MarkerBlockMissing { rc_path },
            };
        }
    };
    let block_body = &content[after_begin..end_idx];
    let env_vars = parse_block_env_vars(block_body, kind);
    ShellRcCheckResult::Present { rc_path, env_vars }
}

/// Parse the `(name, value)` pairs out of the marker block body. We
/// recognise only:
///
/// - `export NAME="value"` (bash / zsh)
/// - `set -gx NAME "value"` (fish)
/// - `$env:NAME = "value"` (pwsh)
///
/// Other lines (comments, blank, unexpected syntax) are skipped. The
/// parser does NOT honour shell-level escaping — the SLICE 5 writer
/// only ever emits straight double-quoted values, so any value with an
/// embedded quote would already have been a writer bug. This keeps the
/// parser deliberately small.
fn parse_block_env_vars(body: &str, kind: DetectedShell) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let pair = match kind {
            DetectedShell::Bash | DetectedShell::Zsh => parse_export_line(trimmed),
            DetectedShell::Fish => parse_fish_set_line(trimmed),
            DetectedShell::Pwsh => parse_pwsh_assign_line(trimmed),
            DetectedShell::Cmd => None,
        };
        if let Some(p) = pair {
            out.push(p);
        }
    }
    out
}

fn parse_export_line(line: &str) -> Option<(String, String)> {
    // `export NAME="value"`
    let rest = line.strip_prefix("export ")?;
    parse_name_eq_quoted(rest)
}

fn parse_fish_set_line(line: &str) -> Option<(String, String)> {
    // `set -gx NAME "value"`
    let rest = line.strip_prefix("set ")?;
    // The flags (e.g. `-gx`) come before NAME. Skip leading `-…` tokens.
    let mut tokens = rest.split_whitespace();
    let mut name = None;
    for tok in &mut tokens {
        if let Some(stripped) = tok.strip_prefix('-') {
            let _ = stripped; // discard flag
            continue;
        }
        name = Some(tok.to_string());
        break;
    }
    let name = name?;
    let value_quoted = tokens.collect::<Vec<_>>().join(" ");
    let value = strip_outer_quotes(&value_quoted)?;
    Some((name, value))
}

fn parse_pwsh_assign_line(line: &str) -> Option<(String, String)> {
    // `$env:NAME = "value"`
    let rest = line.strip_prefix("$env:")?;
    let (name, value_part) = rest.split_once('=')?;
    let name = name.trim().to_string();
    let value = strip_outer_quotes(value_part.trim())?;
    Some((name, value))
}

fn parse_name_eq_quoted(s: &str) -> Option<(String, String)> {
    let (name, value_part) = s.split_once('=')?;
    let name = name.trim().to_string();
    let value = strip_outer_quotes(value_part.trim())?;
    Some((name, value))
}

fn strip_outer_quotes(s: &str) -> Option<String> {
    let s = s.trim();
    let s = s.strip_prefix('"').unwrap_or(s);
    let s = s.strip_suffix('"').unwrap_or(s);
    Some(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{MARKER_BEGIN, MARKER_END};

    /// `check` with no shell detected → Absent::NoShellDetected.
    #[test]
    fn check_returns_no_shell_detected_when_envview_empty() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let env = BaseEnv {
            home: Some(tmp.path()),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        let res = check(&env, Some(EnvView::default()));
        match res {
            ShellRcCheckResult::Absent {
                reason: AbsentReason::NoShellDetected,
            } => {}
            other => panic!("expected Absent::NoShellDetected, got {other:?}"),
        }
    }

    /// `check` with a `.bashrc` containing the marker block → Present
    /// with the parsed env vars.
    #[test]
    fn check_returns_present_when_marker_block_in_bashrc() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let bashrc = tmp.path().join(".bashrc");
        std::fs::write(
            &bashrc,
            format!(
                "alias g='git'\n\
                 {MARKER_BEGIN}\n\
                 export HTTPS_PROXY=\"https://localhost:8443\"\n\
                 export NODE_EXTRA_CA_CERTS=\"/tmp/ca.pem\"\n\
                 {MARKER_END}\n"
            ),
        )
        .expect("seed bashrc");
        let env = BaseEnv {
            home: Some(tmp.path()),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        let env_view = EnvView {
            shell: Some("/bin/bash"),
            ..Default::default()
        };
        let res = check(&env, Some(env_view));
        match res {
            ShellRcCheckResult::Present { rc_path, env_vars } => {
                assert_eq!(rc_path, bashrc);
                assert_eq!(env_vars.len(), 2);
                assert_eq!(env_vars[0].0, "HTTPS_PROXY");
                assert_eq!(env_vars[0].1, "https://localhost:8443");
                assert_eq!(env_vars[1].0, "NODE_EXTRA_CA_CERTS");
            }
            other => panic!("expected Present, got {other:?}"),
        }
    }

    /// `check` with a `.bashrc` that exists but has no marker block →
    /// Absent::MarkerBlockMissing.
    #[test]
    fn check_returns_marker_block_missing_when_rc_present_but_block_absent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let bashrc = tmp.path().join(".bashrc");
        std::fs::write(&bashrc, "alias g='git'\n").expect("seed bashrc");
        let env = BaseEnv {
            home: Some(tmp.path()),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        let env_view = EnvView {
            shell: Some("/bin/bash"),
            ..Default::default()
        };
        let res = check(&env, Some(env_view));
        match res {
            ShellRcCheckResult::Absent {
                reason: AbsentReason::MarkerBlockMissing { rc_path },
            } => {
                assert_eq!(rc_path, bashrc);
            }
            other => panic!("expected Absent::MarkerBlockMissing, got {other:?}"),
        }
    }

    /// `check` with shell=bash but the `.bashrc` doesn't exist →
    /// Absent::RcFileNotFound.
    #[test]
    fn check_returns_rc_file_not_found_when_rc_absent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Do NOT write the .bashrc.
        let env = BaseEnv {
            home: Some(tmp.path()),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        let env_view = EnvView {
            shell: Some("/bin/bash"),
            ..Default::default()
        };
        let res = check(&env, Some(env_view));
        let expected = tmp.path().join(".bashrc");
        match res {
            ShellRcCheckResult::Absent {
                reason: AbsentReason::RcFileNotFound { rc_path },
            } => {
                assert_eq!(rc_path, expected);
            }
            other => panic!("expected Absent::RcFileNotFound, got {other:?}"),
        }
    }

    /// `check` with HOME=None → Absent::NoHomeResolvable, even with
    /// shell detected.
    #[test]
    fn check_returns_no_home_resolvable_when_base_env_home_unset() {
        let env = BaseEnv {
            home: None,
            gemini_api_key: None,
            google_application_credentials: None,
        };
        let env_view = EnvView {
            shell: Some("/bin/bash"),
            ..Default::default()
        };
        let res = check(&env, Some(env_view));
        match res {
            ShellRcCheckResult::Absent {
                reason: AbsentReason::NoHomeResolvable,
            } => {}
            other => panic!("expected Absent::NoHomeResolvable, got {other:?}"),
        }
    }

    /// Detected Cmd → Absent::RcPathNotApplicable.
    #[test]
    fn check_with_cmd_detected_returns_rc_path_not_applicable() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let env = BaseEnv {
            home: Some(tmp.path()),
            gemini_api_key: None,
            google_application_credentials: None,
        };
        let env_view = EnvView {
            comspec: Some(r"C:\Windows\System32\cmd.exe"),
            ..Default::default()
        };
        let res = check(&env, Some(env_view));
        match res {
            ShellRcCheckResult::Absent {
                reason: AbsentReason::RcPathNotApplicable { kind },
            } => {
                assert_eq!(kind, DetectedShell::Cmd);
            }
            other => panic!("expected Absent::RcPathNotApplicable, got {other:?}"),
        }
    }

    /// Fish parser handles `set -gx NAME "value"`.
    #[test]
    fn parse_block_env_vars_handles_fish() {
        let body = "\nset -gx HTTPS_PROXY \"https://localhost:8443\"\n";
        let vars = parse_block_env_vars(body, DetectedShell::Fish);
        assert_eq!(
            vars,
            vec![("HTTPS_PROXY".into(), "https://localhost:8443".into())]
        );
    }

    /// PowerShell parser handles `$env:NAME = "value"`.
    #[test]
    fn parse_block_env_vars_handles_pwsh() {
        let body = "\n$env:HTTPS_PROXY = \"https://localhost:8443\"\n";
        let vars = parse_block_env_vars(body, DetectedShell::Pwsh);
        assert_eq!(
            vars,
            vec![("HTTPS_PROXY".into(), "https://localhost:8443".into())]
        );
    }

    /// Renderer status tokens.
    #[test]
    fn render_emits_expected_status_tokens() {
        let present = ShellRcCheckResult::Present {
            rc_path: PathBuf::from("/home/u/.zshrc"),
            env_vars: vec![("HTTPS_PROXY".into(), "x".into())],
        };
        let plain = present.render(false);
        assert!(plain.contains("OK"));
        assert!(plain.contains("HTTPS_PROXY"));
        assert!(plain.contains("/home/u/.zshrc"));

        let absent_no_shell = ShellRcCheckResult::Absent {
            reason: AbsentReason::NoShellDetected,
        };
        assert!(absent_no_shell.render(false).contains("WARN"));

        let absent_missing = ShellRcCheckResult::Absent {
            reason: AbsentReason::MarkerBlockMissing {
                rc_path: PathBuf::from("/home/u/.zshrc"),
            },
        };
        let r = absent_missing.render(false);
        assert!(r.contains("WARN"));
        assert!(r.contains("no SpendGuard marker block"));
    }
}
