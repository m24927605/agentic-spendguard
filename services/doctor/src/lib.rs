//! Phase 5 GA hardening S21: `spendguard doctor` core.
//!
//! Library crate so checks are unit-testable. Binary is a thin
//! arg-parser → orchestration wrapper.
//!
//! Each check returns a typed `CheckResult` carrying:
//!   * `name`: stable id (e.g. "sidecar.uds_reachable")
//!   * `status`: Pass | Fail | Skipped
//!   * `code`: actionable error code on fail (e.g.
//!     `BUNDLE_NOT_MOUNTED`)
//!   * `human_message`: short operator-readable text
//!   * `remediation`: one-line instruction (or doc link)
//!
//! Output: Report = vec of CheckResults + an overall verdict.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// One precondition's outcome.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    /// Skipped because the check was disabled or its dependencies
    /// (e.g. canonical DB URL) weren't configured. Doctor reports
    /// these but doesn't count them as failures.
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub name: String,
    pub status: CheckStatus,
    pub code: Option<String>,
    pub human_message: String,
    pub remediation: Option<String>,
}

impl CheckResult {
    pub fn pass(name: impl Into<String>, msg: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: CheckStatus::Pass,
            code: None,
            human_message: msg.into(),
            remediation: None,
        }
    }

    pub fn fail(
        name: impl Into<String>,
        code: impl Into<String>,
        msg: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            status: CheckStatus::Fail,
            code: Some(code.into()),
            human_message: msg.into(),
            remediation: Some(remediation.into()),
        }
    }

    pub fn skipped(name: impl Into<String>, why: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: CheckStatus::Skipped,
            code: None,
            human_message: why.into(),
            remediation: None,
        }
    }
}

/// Aggregate outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub checks: Vec<CheckResult>,
    pub overall: CheckStatus,
    pub fail_count: usize,
    pub pass_count: usize,
    pub skip_count: usize,
}

impl Report {
    pub fn from_checks(checks: Vec<CheckResult>) -> Self {
        let fail = checks.iter().filter(|c| c.status == CheckStatus::Fail).count();
        let pass = checks.iter().filter(|c| c.status == CheckStatus::Pass).count();
        let skip = checks.iter().filter(|c| c.status == CheckStatus::Skipped).count();
        let overall = if fail > 0 {
            CheckStatus::Fail
        } else {
            CheckStatus::Pass
        };
        Self {
            checks,
            overall,
            fail_count: fail,
            pass_count: pass,
            skip_count: skip,
        }
    }

    /// Render as a human-readable text block. Operator pipes to
    /// stdout; the JSON form is also available for SIEM / dashboard
    /// consumption.
    pub fn render_human(&self) -> String {
        let mut out = String::new();
        out.push_str("=== spendguard doctor ===\n");
        for c in &self.checks {
            let icon = match c.status {
                CheckStatus::Pass => "[PASS]",
                CheckStatus::Fail => "[FAIL]",
                CheckStatus::Skipped => "[SKIP]",
            };
            out.push_str(&format!("{icon} {} — {}\n", c.name, c.human_message));
            if let Some(code) = &c.code {
                out.push_str(&format!("       code: {code}\n"));
            }
            if let Some(rem) = &c.remediation {
                out.push_str(&format!("       fix: {rem}\n"));
            }
        }
        out.push_str(&format!(
            "\nSummary: {} pass, {} fail, {} skip — overall {}\n",
            self.pass_count,
            self.fail_count,
            self.skip_count,
            match self.overall {
                CheckStatus::Pass => "GREEN",
                CheckStatus::Fail => "RED",
                CheckStatus::Skipped => "GRAY",
            }
        ));
        out
    }
}

// ============================================================================
// Individual checks
// ============================================================================

/// Sidecar UDS reachability — the socket file exists and is a unix
/// stream socket. We don't actually attempt a gRPC handshake here
/// (network dependency); a separate `sidecar.handshake_ok` check
/// would do that.
pub fn check_sidecar_uds(path: &Path) -> CheckResult {
    if !path.exists() {
        return CheckResult::fail(
            "sidecar.uds_present",
            "UDS_NOT_PRESENT",
            format!("expected UDS at {} but no socket file", path.display()),
            "verify sidecar.hostPath.enabled=true and the volumeMount is correct \
             on this pod (see operations/multi-pod.md)",
        );
    }
    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) => {
            return CheckResult::fail(
                "sidecar.uds_present",
                "UDS_STAT_FAILED",
                format!("stat({}) failed: {e}", path.display()),
                "check filesystem permissions",
            );
        }
    };
    use std::os::unix::fs::FileTypeExt;
    if !metadata.file_type().is_socket() {
        return CheckResult::fail(
            "sidecar.uds_present",
            "UDS_NOT_SOCKET",
            format!("{} exists but isn't a unix socket", path.display()),
            "remove the file and let the sidecar recreate it",
        );
    }
    CheckResult::pass(
        "sidecar.uds_present",
        format!("UDS reachable at {}", path.display()),
    )
}

/// Contract bundle present + readable.
pub fn check_contract_bundle(bundle_dir: &Path) -> CheckResult {
    if !bundle_dir.exists() {
        return CheckResult::fail(
            "contract.bundle_mounted",
            "BUNDLE_NOT_MOUNTED",
            format!(
                "contract bundle dir {} does not exist",
                bundle_dir.display()
            ),
            "verify spendguard-bundles Secret is mounted at /var/lib/spendguard/bundles",
        );
    }
    let metadata = match bundle_dir.read_dir() {
        Ok(d) => d,
        Err(e) => {
            return CheckResult::fail(
                "contract.bundle_mounted",
                "BUNDLE_READ_FAILED",
                format!("read_dir({}) failed: {e}", bundle_dir.display()),
                "fix filesystem permissions on the mounted Secret",
            );
        }
    };
    let count = metadata.count();
    if count == 0 {
        return CheckResult::fail(
            "contract.bundle_mounted",
            "BUNDLE_EMPTY",
            format!("{} is empty", bundle_dir.display()),
            "rebuild and re-upload the contract bundle to the Secret",
        );
    }
    CheckResult::pass(
        "contract.bundle_mounted",
        format!("{} entries in {}", count, bundle_dir.display()),
    )
}

/// Signing mode introspection. Reads SPENDGUARD_<service>_SIGNING_MODE
/// (caller picks the prefix). Skipped if mode env is unset.
pub fn check_signing_mode(env_prefix: &str, profile: &str) -> CheckResult {
    let var = format!("{env_prefix}_SIGNING_MODE");
    let mode = match std::env::var(&var) {
        Ok(m) => m,
        Err(_) => return CheckResult::skipped(
            "signing.mode_configured",
            format!("env {var} unset"),
        ),
    };
    if mode == "disabled" && profile != "demo" {
        return CheckResult::fail(
            "signing.mode_configured",
            "SIGNING_DISABLED_OUTSIDE_DEMO",
            format!("{var}=disabled with profile={profile}"),
            "set SPENDGUARD_PROFILE=demo OR pick mode=local|kms",
        );
    }
    CheckResult::pass(
        "signing.mode_configured",
        format!("{var}={mode}, profile={profile}"),
    )
}

/// Pricing snapshot freshness — query the latest pricing_versions.cut_at
/// and compare to `max_staleness_seconds`. Skipped if no DB pool.
pub async fn check_pricing_freshness(
    canonical_pool: Option<&sqlx::PgPool>,
    max_staleness_seconds: u64,
) -> CheckResult {
    let pool = match canonical_pool {
        Some(p) => p,
        None => return CheckResult::skipped(
            "pricing.freshness",
            "canonical DB pool not provided",
        ),
    };
    let row: Option<(chrono::DateTime<chrono::Utc>,)> = match sqlx::query_as(
        "SELECT cut_at FROM pricing_versions ORDER BY cut_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            return CheckResult::fail(
                "pricing.freshness",
                "PRICING_QUERY_FAILED",
                format!("query failed: {e}"),
                "check canonical DB connectivity + that 0006_pricing_table.sql ran",
            );
        }
    };
    let cut_at = match row {
        Some((t,)) => t,
        None => {
            return CheckResult::fail(
                "pricing.freshness",
                "PRICING_VERSION_MISSING",
                "no rows in pricing_versions",
                "run pricing-sync OR seed via the onboarding pricing import",
            );
        }
    };
    let age_secs = (chrono::Utc::now() - cut_at).num_seconds();
    if age_secs > max_staleness_seconds as i64 {
        return CheckResult::fail(
            "pricing.freshness",
            "PRICING_STALE",
            format!("latest pricing_version is {age_secs}s old, max {max_staleness_seconds}s"),
            "run pricing-sync OR raise pricing.maxStalenessSeconds (carefully)",
        );
    }
    CheckResult::pass(
        "pricing.freshness",
        format!("latest pricing_version is {age_secs}s old (limit {max_staleness_seconds}s)"),
    )
}

/// Tenant budget existence — confirm there's at least one
/// reservations / ledger_accounts row for the tenant. Catches
/// "operator forgot to provision the tenant" before customer runs
/// a real agent.
pub async fn check_tenant_provisioned(
    ledger_pool: Option<&sqlx::PgPool>,
    tenant_id: uuid::Uuid,
) -> CheckResult {
    let pool = match ledger_pool {
        Some(p) => p,
        None => return CheckResult::skipped(
            "tenant.provisioned",
            "ledger DB pool not provided",
        ),
    };
    let row: Option<(i64,)> = match sqlx::query_as(
        "SELECT count(*) FROM ledger_accounts WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            return CheckResult::fail(
                "tenant.provisioned",
                "TENANT_QUERY_FAILED",
                format!("ledger_accounts query failed: {e}"),
                "verify ledger DB connection + migrations applied",
            );
        }
    };
    let count = row.map(|(c,)| c).unwrap_or(0);
    if count == 0 {
        return CheckResult::fail(
            "tenant.provisioned",
            "TENANT_NOT_PROVISIONED",
            format!("no ledger_accounts rows for tenant {}", tenant_id),
            "POST /v1/tenants on Control Plane to provision the tenant + budget",
        );
    }
    CheckResult::pass(
        "tenant.provisioned",
        format!("{count} ledger_accounts rows for tenant"),
    )
}

/// Ledger DB reachable — basic SELECT 1.
pub async fn check_ledger_db(pool: Option<&sqlx::PgPool>) -> CheckResult {
    let pool = match pool {
        Some(p) => p,
        None => return CheckResult::skipped(
            "ledger.db_reachable",
            "ledger DB pool not provided",
        ),
    };
    match sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(pool).await {
        Ok(_) => CheckResult::pass(
            "ledger.db_reachable",
            "SELECT 1 ok",
        ),
        Err(e) => CheckResult::fail(
            "ledger.db_reachable",
            "LEDGER_DB_UNREACHABLE",
            format!("SELECT 1 failed: {e}"),
            "verify ledger DB URL + network policy + Postgres up",
        ),
    }
}

/// Redact known-sensitive env vars before printing the report —
/// spec invariant ("secrets are redacted from output").
pub fn redact_secrets(input: &str, env: &HashMap<String, String>) -> String {
    let mut out = input.to_string();
    for (k, v) in env {
        if v.is_empty() {
            continue;
        }
        // Redact any env var whose name contains common secret
        // markers. Conservative — false positives are fine
        // (operator just sees `<redacted>` instead of the value).
        let lower = k.to_lowercase();
        if lower.contains("token")
            || lower.contains("secret")
            || lower.contains("password")
            || lower.contains("api_key")
            || lower.contains("private")
        {
            out = out.replace(v, "<redacted>");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_overall_pass_when_no_failures() {
        let r = Report::from_checks(vec![
            CheckResult::pass("a", "ok"),
            CheckResult::skipped("b", "skipped"),
        ]);
        assert_eq!(r.overall, CheckStatus::Pass);
        assert_eq!(r.pass_count, 1);
        assert_eq!(r.skip_count, 1);
        assert_eq!(r.fail_count, 0);
    }

    #[test]
    fn report_overall_fail_when_any_failure() {
        let r = Report::from_checks(vec![
            CheckResult::pass("a", "ok"),
            CheckResult::fail("b", "CODE", "bad", "fix it"),
        ]);
        assert_eq!(r.overall, CheckStatus::Fail);
        assert_eq!(r.fail_count, 1);
    }

    #[test]
    fn check_signing_mode_skips_when_unset() {
        std::env::remove_var("DOCTOR_TEST_PREFIX_SIGNING_MODE");
        let r = check_signing_mode("DOCTOR_TEST_PREFIX", "production");
        assert_eq!(r.status, CheckStatus::Skipped);
    }

    #[test]
    fn check_signing_mode_fails_when_disabled_outside_demo() {
        std::env::set_var("S21_TEST_SIGNING_MODE", "disabled");
        let r = check_signing_mode("S21_TEST", "production");
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.code.as_deref(), Some("SIGNING_DISABLED_OUTSIDE_DEMO"));
        std::env::remove_var("S21_TEST_SIGNING_MODE");
    }

    #[test]
    fn check_signing_mode_passes_when_disabled_in_demo() {
        std::env::set_var("S21_TEST_DEMO_SIGNING_MODE", "disabled");
        let r = check_signing_mode("S21_TEST_DEMO", "demo");
        assert_eq!(r.status, CheckStatus::Pass);
        std::env::remove_var("S21_TEST_DEMO_SIGNING_MODE");
    }

    #[test]
    fn check_contract_bundle_fails_when_dir_missing() {
        let r = check_contract_bundle(Path::new("/definitely/does/not/exist"));
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.code.as_deref(), Some("BUNDLE_NOT_MOUNTED"));
    }

    #[test]
    fn check_contract_bundle_passes_when_dir_has_entries() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("dummy.tgz"), b"x").unwrap();
        let r = check_contract_bundle(dir.path());
        assert_eq!(r.status, CheckStatus::Pass);
    }

    #[test]
    fn check_sidecar_uds_fails_when_path_missing() {
        let r = check_sidecar_uds(Path::new("/definitely/does/not/exist.sock"));
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.code.as_deref(), Some("UDS_NOT_PRESENT"));
    }

    #[test]
    fn redact_secrets_replaces_known_secret_envs() {
        let mut env = HashMap::new();
        env.insert("ADMIN_TOKEN".into(), "abc123".into());
        env.insert("OPENAI_API_KEY".into(), "sk-xyz".into());
        env.insert("HARMLESS".into(), "fine-value".into());
        let input = "token=abc123 key=sk-xyz harmless=fine-value";
        let out = redact_secrets(input, &env);
        assert!(out.contains("<redacted>"));
        assert!(!out.contains("abc123"));
        assert!(!out.contains("sk-xyz"));
        assert!(out.contains("fine-value"));
    }

    #[test]
    fn render_human_includes_pass_and_fail_lines() {
        let r = Report::from_checks(vec![
            CheckResult::pass("a", "ok"),
            CheckResult::fail("b", "CODE_X", "bad", "do this"),
        ]);
        let txt = r.render_human();
        assert!(txt.contains("[PASS] a"));
        assert!(txt.contains("[FAIL] b"));
        assert!(txt.contains("code: CODE_X"));
        assert!(txt.contains("fix: do this"));
        assert!(txt.contains("Summary"));
    }
}
