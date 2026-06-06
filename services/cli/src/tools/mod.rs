//! Per-tool override matrix (SLICE 5 / COV_09).
//!
//! Maps the 14 BYOK CLIs from `design.md` §5 to the env var or config key
//! each one reads for "trust this extra CA bundle". The shell rc writer
//! consumes [`env_vars_for_install`] to render the marker block; the
//! install report carries the same data structurally so `doctor` / SLICE 7
//! can verify each override landed.
//!
//! ## Source of truth — deviation #2
//!
//! The slice-prompt's "CLAUDE_PROXY / OPENAI_PROXY / SRC_HTTPS_PROXY" pseudo
//! table was illustrative; the **authoritative** matrix is `design.md` §5
//! (lines 53-68 of the spec, table "Per-tool override matrix"). The names
//! below are copied verbatim from that table — the review-standards `M2`
//! / `M3` / `M4` / `M5` / `M6` checks bake those names in too. Concretely:
//!
//! - Node tools set `NODE_EXTRA_CA_CERTS` (Node ignores OS trust)
//! - Codex sets `CODEX_CA_CERTIFICATE` (native env var, Codex 0.30+)
//! - Aider sets `REQUESTS_CA_BUNDLE` (Python `requests` ships its own bundle)
//! - OpenHands sets `SSL_CERT_FILE` (Python httpx / aiohttp default path)
//! - Cody self-hosted sets `SRC_HTTPS_PROXY` (this is a per-tool *proxy*
//!   override, not a CA path — same surface name as the proxy URL)
//! - Goose, Amazon Q, Zed set NO extra env var — they honour OS trust +
//!   the global `HTTPS_PROXY`. `M5` rejects redundant exports.
//! - Tabnine Enterprise writes a config-file key (`tabnine.caBundle`),
//!   NOT a shell env var. We record it for the install report but skip
//!   it in [`env_vars_for_install`].
//!
//! ## Why a static table
//!
//! The full matrix is data, not code. `tools.toml` shipping at runtime is
//! a SLICE 7 / SLICE 8 follow-up (operator can `--include` / `--exclude`
//! against the file). For SLICE 5 we burn the table into the binary as
//! a `&'static [ToolOverride]` so the rc writer + install report can be
//! tested hermetically without filesystem state. The TOML loader plugs
//! in as a constructor that returns this same slice when the file is
//! absent.

/// Mechanism the tool uses to honour the CA / proxy bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverrideKind {
    /// Standard env-var assignment in the shell rc block.
    EnvVar,
    /// Tool reads a config-file key, not an env var. Out of scope for
    /// SLICE 5 rc emission; carried in the install report for the
    /// SLICE 7 doctor + the operator-facing checklist.
    ConfigFile,
    /// Tool honours OS trust + `HTTPS_PROXY` alone. No extra entry
    /// emitted (review-standards `M5`).
    OsTrustOnly,
}

/// One row of the 14-tool matrix from `design.md` §5.
#[derive(Debug, Clone, Copy)]
pub struct ToolOverride {
    /// Display name shown in `InstallReport` / `doctor`. Matches the
    /// first column of `design.md` §5.
    pub display: &'static str,
    /// Short id used for `--include` / `--exclude` matching and for the
    /// install-report JSON.
    pub id: &'static str,
    /// Env var the tool reads for the extra CA bundle / proxy override.
    /// Empty when [`Self::kind`] is `ConfigFile` or `OsTrustOnly`.
    pub env_var: &'static str,
    /// Mechanism family — drives whether the rc writer emits a line.
    pub kind: OverrideKind,
    /// Free-form notes surfaced in the install report's per-tool stanza.
    /// `None` for the unsurprising cases.
    pub notes: Option<&'static str>,
}

/// The 14-tool matrix per `design.md` §5. Order matches the spec table
/// top-to-bottom; `M1` review-standards check cross-references this list
/// against the spec.
pub const TOOL_OVERRIDES: &[ToolOverride] = &[
    ToolOverride {
        display: "Claude Code (Node)",
        id: "claude_code",
        env_var: "NODE_EXTRA_CA_CERTS",
        kind: OverrideKind::EnvVar,
        notes: None,
    },
    ToolOverride {
        display: "Codex CLI (Node)",
        id: "codex",
        env_var: "CODEX_CA_CERTIFICATE",
        kind: OverrideKind::EnvVar,
        notes: Some("native env var since Codex CLI 0.30; `M3` review-standards"),
    },
    ToolOverride {
        display: "Gemini CLI (Node, API-key)",
        id: "gemini",
        env_var: "NODE_EXTRA_CA_CERTS",
        kind: OverrideKind::EnvVar,
        notes: Some("API-key mode only; OAuth free-tier refused in SLICE 6"),
    },
    ToolOverride {
        display: "Aider (Python)",
        id: "aider",
        env_var: "REQUESTS_CA_BUNDLE",
        kind: OverrideKind::EnvVar,
        notes: Some("`requests` ships its own CA bundle; `M4` review-standards"),
    },
    ToolOverride {
        display: "Continue (Node, VS Code)",
        id: "continue",
        env_var: "NODE_EXTRA_CA_CERTS",
        kind: OverrideKind::EnvVar,
        notes: None,
    },
    ToolOverride {
        display: "Cline / Roo (Node, VS Code BYOK)",
        id: "cline_roo",
        env_var: "NODE_EXTRA_CA_CERTS",
        kind: OverrideKind::EnvVar,
        notes: None,
    },
    ToolOverride {
        display: "OpenHands (Python)",
        id: "openhands",
        env_var: "SSL_CERT_FILE",
        kind: OverrideKind::EnvVar,
        notes: None,
    },
    ToolOverride {
        display: "Goose (Rust)",
        id: "goose",
        env_var: "",
        kind: OverrideKind::OsTrustOnly,
        notes: Some("honours OS trust + HTTPS_PROXY; `M5` review-standards"),
    },
    ToolOverride {
        display: "Amazon Q v1.8+ (Rust)",
        id: "amazon_q",
        env_var: "",
        kind: OverrideKind::OsTrustOnly,
        notes: Some("honours OS trust + HTTPS_PROXY; `M5` review-standards"),
    },
    ToolOverride {
        display: "GitHub Copilot CLI BYOK (Node)",
        id: "copilot_byok",
        env_var: "NODE_EXTRA_CA_CERTS",
        kind: OverrideKind::EnvVar,
        notes: None,
    },
    ToolOverride {
        display: "Tabnine Enterprise",
        id: "tabnine_enterprise",
        env_var: "",
        kind: OverrideKind::ConfigFile,
        notes: Some(
            "writes `tabnine.caBundle` to ~/.config/TabNine/config.json; `M6` review-standards",
        ),
    },
    ToolOverride {
        display: "Cody self-hosted",
        id: "cody_selfhosted",
        env_var: "SRC_HTTPS_PROXY",
        kind: OverrideKind::EnvVar,
        notes: Some("per-tool proxy override (not a CA path)"),
    },
    ToolOverride {
        display: "Augment BYOK (Node)",
        id: "augment_byok",
        env_var: "NODE_EXTRA_CA_CERTS",
        kind: OverrideKind::EnvVar,
        notes: None,
    },
    ToolOverride {
        display: "Zed (Rust)",
        id: "zed",
        env_var: "",
        kind: OverrideKind::OsTrustOnly,
        notes: Some("honours OS trust + HTTPS_PROXY; `M5` review-standards"),
    },
];

/// Compute the `(env_var, value)` pairs the rc writer should emit. The
/// first entry is always `HTTPS_PROXY=<proxy_url>` — that's the universal
/// bridge every Pattern-3 tool reads regardless of OS trust mechanism.
///
/// `ca_pem_path` is the OS-resolved path returned by SLICE 1's CA writer
/// (e.g. `~/Library/Application Support/SpendGuard/ca/root_ca.pem` on
/// macOS). Node + Python tools point their CA env vars at the same file.
///
/// `ConfigFile` and `OsTrustOnly` entries are skipped — those don't
/// contribute env vars. The install report surfaces them separately so
/// the operator sees the full tool list.
///
/// Per-tool env vars are de-duplicated by var name: if both `claude_code`
/// and `gemini` map to `NODE_EXTRA_CA_CERTS` we emit the export once.
pub fn env_vars_for_install(proxy_url: &str, ca_pem_path: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen: std::collections::BTreeSet<&'static str> = Default::default();

    out.push(("HTTPS_PROXY".into(), proxy_url.into()));
    seen.insert("HTTPS_PROXY");

    for tool in TOOL_OVERRIDES {
        if tool.kind != OverrideKind::EnvVar {
            continue;
        }
        if tool.env_var.is_empty() {
            continue;
        }
        if !seen.insert(tool.env_var) {
            continue;
        }
        let value = match tool.env_var {
            // Proxy override — points at the local egress proxy URL.
            "SRC_HTTPS_PROXY" => proxy_url.to_string(),
            // CA bundle override — points at the on-disk root CA PEM.
            _ => ca_pem_path.to_string(),
        };
        out.push((tool.env_var.to_string(), value));
    }
    out
}

/// Lookup by id. Returns `None` for an unknown id so the `--include` /
/// `--exclude` parser can produce a clear "no such tool" diagnostic
/// instead of silently dropping the entry.
pub fn find(id: &str) -> Option<&'static ToolOverride> {
    TOOL_OVERRIDES.iter().find(|t| t.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `M1` review-standards: every entry in `design.md` §5 is present.
    /// The spec table has exactly 14 rows.
    #[test]
    fn tool_overrides_table_has_exactly_14_entries() {
        assert_eq!(
            TOOL_OVERRIDES.len(),
            14,
            "design.md §5 table has 14 rows; TOOL_OVERRIDES must match"
        );
    }

    /// `M1`: the high-frequency tools that ship in every BYOK customer
    /// rollout are present.
    #[test]
    fn tool_overrides_contains_high_frequency_tools() {
        let ids: Vec<&str> = TOOL_OVERRIDES.iter().map(|t| t.id).collect();
        for expected in [
            "claude_code",
            "codex",
            "gemini",
            "aider",
            "continue",
            "cline_roo",
            "openhands",
            "goose",
            "amazon_q",
            "copilot_byok",
            "tabnine_enterprise",
            "cody_selfhosted",
            "augment_byok",
            "zed",
        ] {
            assert!(
                ids.contains(&expected),
                "expected tool id `{expected}` in TOOL_OVERRIDES, got {ids:?}"
            );
        }
    }

    /// `M2` review-standards: Node-based tools MUST set
    /// `NODE_EXTRA_CA_CERTS`, NOT `SSL_CERT_FILE`.
    #[test]
    fn node_tools_use_node_extra_ca_certs() {
        for id in [
            "claude_code",
            "gemini",
            "continue",
            "cline_roo",
            "copilot_byok",
            "augment_byok",
        ] {
            let tool = find(id).unwrap_or_else(|| panic!("missing {id}"));
            assert_eq!(
                tool.env_var, "NODE_EXTRA_CA_CERTS",
                "M2 review-standards: {id} must use NODE_EXTRA_CA_CERTS"
            );
        }
    }

    /// `M3` review-standards: Codex uses `CODEX_CA_CERTIFICATE`.
    #[test]
    fn codex_uses_native_codex_ca_certificate() {
        let tool = find("codex").expect("codex");
        assert_eq!(tool.env_var, "CODEX_CA_CERTIFICATE");
    }

    /// `M4` review-standards: aider uses `REQUESTS_CA_BUNDLE`.
    #[test]
    fn aider_uses_requests_ca_bundle() {
        let tool = find("aider").expect("aider");
        assert_eq!(tool.env_var, "REQUESTS_CA_BUNDLE");
    }

    /// `M5` review-standards: Goose / Amazon Q / Zed set no extra env vars.
    #[test]
    fn os_trust_only_tools_emit_no_env_var() {
        for id in ["goose", "amazon_q", "zed"] {
            let tool = find(id).expect(id);
            assert_eq!(
                tool.kind,
                OverrideKind::OsTrustOnly,
                "{id} must be OsTrustOnly"
            );
            assert!(
                tool.env_var.is_empty(),
                "{id} must NOT carry an env-var name"
            );
        }
    }

    /// `M6` review-standards: Tabnine writes a config file, not an env
    /// var. Skip in env_vars_for_install but record in TOOL_OVERRIDES.
    #[test]
    fn tabnine_uses_config_file_not_env_var() {
        let tool = find("tabnine_enterprise").expect("tabnine_enterprise");
        assert_eq!(tool.kind, OverrideKind::ConfigFile);
    }

    /// Smoke: env_vars_for_install emits HTTPS_PROXY plus the deduped
    /// env-var overrides — no ConfigFile / OsTrustOnly entries leak in.
    #[test]
    fn env_vars_for_install_emits_https_proxy_first_and_dedupes_node_extra_ca_certs() {
        let vars = env_vars_for_install("https://localhost:8443", "/tmp/spendguard/ca.pem");
        assert_eq!(
            vars[0],
            ("HTTPS_PROXY".into(), "https://localhost:8443".into())
        );

        // NODE_EXTRA_CA_CERTS appears exactly once even though 6 tools
        // map to it.
        let node_count = vars
            .iter()
            .filter(|(k, _)| k == "NODE_EXTRA_CA_CERTS")
            .count();
        assert_eq!(
            node_count, 1,
            "NODE_EXTRA_CA_CERTS must be deduped, got {node_count} entries"
        );

        // CODEX_CA_CERTIFICATE / REQUESTS_CA_BUNDLE / SSL_CERT_FILE
        // / SRC_HTTPS_PROXY all present.
        let names: Vec<&str> = vars.iter().map(|(k, _)| k.as_str()).collect();
        for expected in [
            "HTTPS_PROXY",
            "NODE_EXTRA_CA_CERTS",
            "CODEX_CA_CERTIFICATE",
            "REQUESTS_CA_BUNDLE",
            "SSL_CERT_FILE",
            "SRC_HTTPS_PROXY",
        ] {
            assert!(
                names.contains(&expected),
                "expected env var {expected} in {names:?}"
            );
        }
    }

    /// `SRC_HTTPS_PROXY` points at the proxy URL (proxy override), other
    /// CA-bundle env vars point at the PEM path.
    #[test]
    fn src_https_proxy_value_is_proxy_url_not_ca_path() {
        let vars = env_vars_for_install("https://localhost:8443", "/tmp/spendguard/ca.pem");
        let src = vars
            .iter()
            .find(|(k, _)| k == "SRC_HTTPS_PROXY")
            .expect("SRC_HTTPS_PROXY");
        assert_eq!(src.1, "https://localhost:8443");

        let node = vars
            .iter()
            .find(|(k, _)| k == "NODE_EXTRA_CA_CERTS")
            .expect("NODE_EXTRA_CA_CERTS");
        assert_eq!(node.1, "/tmp/spendguard/ca.pem");
    }

    /// `find` returns None for an unknown id.
    #[test]
    fn find_returns_none_for_unknown_id() {
        assert!(find("not-a-real-tool").is_none());
    }
}
