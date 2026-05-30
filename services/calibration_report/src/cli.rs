//! CLI surface — clap-derived.
//!
//! Spec ancestor: `docs/calibration-report-spec-v1alpha1.md` §2.
//!
//! ## Flag contracts
//!
//! Per spec §2.2:
//!   * `--tenant <uuid>` is required for `report` (default subcommand).
//!     For `verify-chain` it is optional (matches the legacy `verify-chain`
//!     CLI shape exported from canonical_ingest).
//!   * `--from <iso-ts-or-relative>` defaults to `7d` (relative duration
//!     parsed as "N{d,h,m}" before now()).
//!   * `--to` defaults to `now`.
//!   * `--format` ∈ {text, json, markdown} (default: text).
//!   * `--proof-mode` ∈ {cache, canonical} (default: cache). Canonical
//!     reads `canonical_events` directly instead of the cache tables.
//!   * `--output` is a path or `-` (stdout). Default: `-`.
//!   * `--include-recommendations` defaults to false in JSON, true in
//!     text/markdown. CLI exposes a tri-state via `Option<bool>` so the
//!     orchestrator can compute the default per format.
//!   * `--verify-chain` implies `--proof-mode=canonical` and triggers the
//!     full audit-chain replay on every event in the window.
//!
//! ## Exit codes (spec §2.3)
//!
//! - `0` — success, no critical findings.
//! - `1` — critical findings present (Tier 3 > 0.1%, drift > 0,
//!   calibration P95 > 1.50).
//! - `2` — query / canonical-events unreachable / cross-tenant rejection.
//! - `3` — verify-chain failure.
//!
//! Exit-code enums live in `report::ReportExitCode`.

use clap::{Parser, Subcommand as ClapSubcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "spendguard-calibration-report")]
#[command(
    about = "SLICE_13: SpendGuard calibration-report CLI (audit-grade calibration evidence)"
)]
#[command(version)]
pub struct Cli {
    /// Subcommand. Default is `report` (the operator-facing report
    /// surface). `verify-chain` exposes the audit-chain replay verifier
    /// inline so operators do not need a second binary.
    #[command(subcommand)]
    pub command: Option<Subcommand>,

    /// Tenant id (UUID). Required for `report` (per spec §5.2). Cross-
    /// tenant queries fail with exit code 2 + audit event.
    #[arg(long, env = "SPENDGUARD_CALIBRATION_TENANT")]
    pub tenant: Option<String>,

    /// Window start. ISO-8601 timestamp or relative duration (e.g. `7d`,
    /// `30d`, `1m`). Default `7d`.
    #[arg(long, default_value = "7d")]
    pub from: String,

    /// Window end. ISO-8601 timestamp or `now`. Default `now`.
    #[arg(long, default_value = "now")]
    pub to: String,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,

    /// Source of truth.
    #[arg(long, value_enum, default_value_t = ProofMode::Cache)]
    pub proof_mode: ProofMode,

    /// Output destination. `-` (default) writes to stdout.
    #[arg(long, default_value = "-")]
    pub output: String,

    /// Whether to include the recommendation section. Defaults to
    /// `false` in JSON output (downstream SIEM consumers usually want
    /// raw signals), `true` in text/markdown.
    #[arg(long)]
    pub include_recommendations: Option<bool>,

    /// Run the audit-chain replay verifier. Implies
    /// `--proof-mode=canonical`. Any failing row aborts the report with
    /// exit code 3 (spec §2.3).
    #[arg(long, default_value_t = false)]
    pub verify_chain: bool,

    /// Database connection URL for the canonical_events store. Defaults
    /// to `SPENDGUARD_CALIBRATION_CANONICAL_URL` env var.
    #[arg(long, env = "SPENDGUARD_CALIBRATION_CANONICAL_URL")]
    pub canonical_url: Option<String>,

    /// Auth principal (mTLS subject or env-supplied operator id). The
    /// cross-tenant rejection (§5.2) compares this to `--tenant`. For
    /// production deployments this is set by mTLS termination upstream;
    /// for dev/demo the operator passes `SPENDGUARD_CALIBRATION_AUTH_SUBJECT`.
    #[arg(long, env = "SPENDGUARD_CALIBRATION_AUTH_SUBJECT")]
    pub auth_subject: Option<String>,

    /// Tenant scope that the caller is authenticated for. Multi-tenant
    /// deploys pass a comma-separated list; single-tenant deploys pass
    /// one UUID. Cross-tenant queries (caller's scope does not contain
    /// `--tenant`) exit 2 (spec §5.2).
    #[arg(long, env = "SPENDGUARD_CALIBRATION_AUTH_TENANTS")]
    pub auth_tenants: Option<String>,

    /// JSON-emit a `spendguard.audit.calibration.report_generated`
    /// CloudEvent at the end of the run. Defaults to true; operators
    /// can disable for unit tests / dry runs.
    #[arg(long, default_value_t = true)]
    pub self_audit: bool,
}

#[derive(ClapSubcommand, Debug, Clone)]
pub enum Subcommand {
    /// Generate a calibration report (default).
    Report,
    /// Run the audit-chain replay verifier inline. Mirrors the legacy
    /// `verify-chain` CLI exported from canonical_ingest.
    VerifyChain,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[clap(rename_all = "lowercase")]
pub enum Format {
    Text,
    Json,
    Markdown,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[clap(rename_all = "lowercase")]
pub enum ProofMode {
    /// Read from the stats_aggregator cache tables (fast; operator
    /// daily use).
    Cache,
    /// Read from `canonical_events` directly (tamper-evident; audit).
    Canonical,
}

impl Cli {
    /// Resolve the subcommand, defaulting to `Report` for backwards-compat
    /// with the bare `spendguard-calibration-report --tenant ...` form.
    pub fn subcommand(&self) -> Subcommand {
        self.command.clone().unwrap_or(Subcommand::Report)
    }

    /// Per-format default for `include_recommendations` when the operator
    /// did not pass the flag explicitly.
    ///
    /// Spec §2.2: JSON defaults to `false`; text/markdown defaults to
    /// `true`.
    pub fn effective_include_recommendations(&self) -> bool {
        match self.include_recommendations {
            Some(v) => v,
            None => !matches!(self.format, Format::Json),
        }
    }

    /// `--verify-chain` implies `--proof-mode=canonical` per spec §2.2.
    /// Caller uses this to override the user-supplied flag silently
    /// (and emits a tracing warn if the operator passed cache+verify).
    pub fn effective_proof_mode(&self) -> ProofMode {
        if self.verify_chain {
            ProofMode::Canonical
        } else {
            self.proof_mode
        }
    }

    /// Cross-tenant rejection per spec §5.2.
    ///
    /// Returns `true` iff the authenticated principal is allowed to
    /// query `--tenant`. Returns `Ok(false)` for refusal; `Err` for
    /// missing / malformed inputs.
    ///
    /// Behaviour:
    ///   * If `--auth-tenants` is unset → returns `Ok(true)` (assume
    ///     the deployment is single-tenant + un-scoped, e.g. demo).
    ///   * If `--auth-tenants` is set → caller must list the requested
    ///     `--tenant` in the comma-separated scope.
    ///   * `--tenant` must parse as a UUID.
    pub fn check_tenant_scope(&self) -> Result<bool, String> {
        let tenant = self
            .tenant
            .as_deref()
            .ok_or_else(|| "--tenant is required".to_string())?;
        // Validate UUID shape so a malformed value cannot silently
        // accept a typo-as-tenant.
        uuid::Uuid::parse_str(tenant)
            .map_err(|e| format!("--tenant is not a valid UUID: {e}"))?;
        match self.auth_tenants.as_deref() {
            None => Ok(true),
            Some(scope) => {
                let allowed = scope
                    .split(',')
                    .map(|s| s.trim())
                    .any(|s| s.eq_ignore_ascii_case(tenant));
                Ok(allowed)
            }
        }
    }

    /// Resolve the output destination as a writer-target descriptor.
    /// Returns `None` if the operator passed `-` (stdout).
    pub fn output_path(&self) -> Option<PathBuf> {
        if self.output == "-" {
            None
        } else {
            Some(PathBuf::from(&self.output))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Cli {
        let mut full = vec!["spendguard-calibration-report"];
        full.extend_from_slice(args);
        Cli::parse_from(full)
    }

    #[test]
    fn default_subcommand_is_report() {
        // SLICE_13 §2.1: bare invocation with `--tenant` runs `report`.
        let cli = parse(&[
            "--tenant",
            "00000000-0000-4000-8000-000000000001",
        ]);
        assert!(matches!(cli.subcommand(), Subcommand::Report));
    }

    #[test]
    fn explicit_verify_chain_subcommand() {
        let cli = parse(&["verify-chain"]);
        assert!(matches!(cli.subcommand(), Subcommand::VerifyChain));
    }

    #[test]
    fn default_window_is_7d() {
        let cli = parse(&[
            "--tenant",
            "00000000-0000-4000-8000-000000000001",
        ]);
        assert_eq!(cli.from, "7d");
        assert_eq!(cli.to, "now");
    }

    #[test]
    fn default_format_is_text() {
        let cli = parse(&[
            "--tenant",
            "00000000-0000-4000-8000-000000000001",
        ]);
        assert_eq!(cli.format, Format::Text);
    }

    #[test]
    fn default_proof_mode_is_cache() {
        let cli = parse(&[
            "--tenant",
            "00000000-0000-4000-8000-000000000001",
        ]);
        assert_eq!(cli.proof_mode, ProofMode::Cache);
    }

    #[test]
    fn verify_chain_implies_canonical_proof_mode() {
        // Spec §2.2: `--verify-chain` implies `--proof-mode=canonical`.
        let cli = parse(&[
            "--tenant",
            "00000000-0000-4000-8000-000000000001",
            "--verify-chain",
        ]);
        assert_eq!(cli.effective_proof_mode(), ProofMode::Canonical);
    }

    #[test]
    fn verify_chain_overrides_explicit_cache() {
        // Operator passes contradictory flags; --verify-chain wins.
        let cli = parse(&[
            "--tenant",
            "00000000-0000-4000-8000-000000000001",
            "--proof-mode",
            "cache",
            "--verify-chain",
        ]);
        assert_eq!(cli.effective_proof_mode(), ProofMode::Canonical);
    }

    #[test]
    fn include_recommendations_defaults_per_format() {
        // JSON defaults to false; text/markdown to true. Spec §2.2.
        let text = parse(&[
            "--tenant",
            "00000000-0000-4000-8000-000000000001",
        ]);
        assert!(text.effective_include_recommendations());
        let json = parse(&[
            "--tenant",
            "00000000-0000-4000-8000-000000000001",
            "--format",
            "json",
        ]);
        assert!(!json.effective_include_recommendations());
        let json_forced = parse(&[
            "--tenant",
            "00000000-0000-4000-8000-000000000001",
            "--format",
            "json",
            "--include-recommendations",
            "true",
        ]);
        assert!(json_forced.effective_include_recommendations());
    }

    #[test]
    fn tenant_scope_check_allows_when_no_auth_tenants_set() {
        // Single-tenant / un-scoped deployment.
        let cli = parse(&[
            "--tenant",
            "00000000-0000-4000-8000-000000000001",
        ]);
        assert_eq!(cli.check_tenant_scope().unwrap(), true);
    }

    #[test]
    fn tenant_scope_check_allows_when_listed() {
        let cli = parse(&[
            "--tenant",
            "00000000-0000-4000-8000-000000000001",
            "--auth-tenants",
            "00000000-0000-4000-8000-000000000001,00000000-0000-4000-8000-000000000002",
        ]);
        assert_eq!(cli.check_tenant_scope().unwrap(), true);
    }

    #[test]
    fn tenant_scope_check_rejects_when_unlisted() {
        let cli = parse(&[
            "--tenant",
            "00000000-0000-4000-8000-000000000099",
            "--auth-tenants",
            "00000000-0000-4000-8000-000000000001",
        ]);
        assert_eq!(cli.check_tenant_scope().unwrap(), false);
    }

    #[test]
    fn tenant_scope_check_rejects_malformed_uuid() {
        let cli = parse(&["--tenant", "not-a-uuid"]);
        assert!(cli.check_tenant_scope().is_err());
    }

    #[test]
    fn output_path_dash_is_stdout() {
        let cli = parse(&[
            "--tenant",
            "00000000-0000-4000-8000-000000000001",
        ]);
        assert!(cli.output_path().is_none());
    }

    #[test]
    fn output_path_file() {
        let cli = parse(&[
            "--tenant",
            "00000000-0000-4000-8000-000000000001",
            "--output",
            "/tmp/report.json",
        ]);
        assert_eq!(
            cli.output_path().as_deref(),
            Some(std::path::Path::new("/tmp/report.json"))
        );
    }
}
