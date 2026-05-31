//! Report data model + exit-code rules.
//!
//! Spec ancestor: `docs/calibration-report-spec-v1alpha1.md` §2.3 +
//! §4.2 (JSON shape) + §7 (metric definitions).
//!
//! This module is **shape-only**: it has no SQL / IO / formatting
//! responsibility. Callers (sql_queries, formatters, recommendations)
//! depend on it for stable cross-module types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Exit code per spec §2.3. CI / monitoring callers parse this
/// directly; `as u8` gives the numeric code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ReportExitCode {
    /// Report generated, no critical findings.
    Success = 0,
    /// Report generated, critical findings present.
    CriticalFindings = 1,
    /// Cannot query / canonical_events unreachable / cross-tenant.
    QueryError = 2,
    /// verify-chain failed; chain integrity violated.
    VerifyChainFailed = 3,
}

impl ReportExitCode {
    pub fn to_process_exit_code(self) -> std::process::ExitCode {
        std::process::ExitCode::from(self as u8)
    }
}

/// Tokenizer tier distribution row. Spec §3.1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TierDistribution {
    /// "T1" / "T2" / "T3" (per `audit_outbox_tokenizer_tier_chk`
    /// constraint in `0046_audit_outbox_prediction_columns.sql`).
    /// May be `None` for the legacy-NULL aggregation row.
    pub tier: Option<String>,
    pub count: i64,
    pub pct: f64,
    /// True iff this row contributes to a `Tier 3 > 0.1%` (warning)
    /// or `> 1.0%` (critical) recommendation rule (§8.1).
    pub threshold_violation: bool,
}

/// Per-(model, strategy) calibration ratio row. Spec §3.2 + §7.1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CalibrationRatio {
    pub model: String,
    /// "A" / "B" / "C" (per `audit_outbox_prediction_strategy_used_chk`).
    pub strategy: String,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub sample_size: i64,
}

/// Drift alert (single row out of the §3.3 query).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DriftAlert {
    pub event_id: String,
    pub event_time: DateTime<Utc>,
    pub bucket: String,
    pub z_score: f64,
}

/// Recommendation entry produced by the §8 engine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Recommendation {
    /// `info` / `warning` / `critical` per spec §8.1.
    pub severity: Severity,
    /// Stable rule code (e.g. `TIER3_BURST`). Spec §4.2 schema.
    pub code: String,
    /// Operator-readable single-line.
    pub headline: String,
    /// "Possible cause / Suggested action" detail block (spec §8.2).
    pub possible_cause: String,
    pub suggested_action: String,
    /// Free-form extra context (rule-specific metrics).
    pub details: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

/// Aggregate report carried between SQL layer and formatters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub tenant_id: String,
    pub window: Window,
    pub proof_mode: String,
    pub tier_distribution: Vec<TierDistribution>,
    pub calibration_ratios: Vec<CalibrationRatio>,
    pub drift_alerts: Vec<DriftAlert>,
    pub run_budget_projection_exceeded_count: i64,
    pub run_drift_detected_count: i64,
    pub run_total_count: i64,
    /// Set by the `recommendations` module after the raw SQL results
    /// are gathered. The formatter respects `include_recommendations`
    /// to decide whether to render the section.
    pub recommendations: Vec<Recommendation>,
    pub verify_chain_run: bool,
    /// On verify-chain failure, the offending row id (decision_id /
    /// event_id) is captured here for the operator. Exit code 3
    /// follows. Spec §3.4.
    pub verify_chain_failure: Option<VerifyChainFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Window {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerifyChainFailure {
    pub event_id: String,
    pub reason: String,
}

impl Report {
    /// Apply spec §2.3 exit-code rules.
    ///
    /// Critical-findings criteria per spec §2.3:
    ///   - any (model, strategy) actual/predicted P95 > 1.50
    ///     (under-prediction outlier),
    ///   - any Strategy C actual/predicted P95 > 1.05 with n >= 30
    ///     (plugin under-prediction),
    ///   - any Tier 3 entry with pct > 0.1% (T3 burst), or
    ///   - drift alerts > 0 in window.
    pub fn exit_code(&self) -> ReportExitCode {
        if self.verify_chain_failure.is_some() {
            return ReportExitCode::VerifyChainFailed;
        }
        let p95_violation = self
            .calibration_ratios
            .iter()
            .any(|r| r.p95 > CRITICAL_P95_THRESHOLD);
        let strategy_c_under_prediction = self.calibration_ratios.iter().any(|r| {
            r.strategy == "C"
                && r.p95 > STRATEGY_C_UNDER_PREDICTION_P95_THRESHOLD
                && r.sample_size >= STRATEGY_C_MIN_SAMPLE_SIZE
        });
        let tier3_violation = self.tier_distribution.iter().any(|t| {
            t.tier.as_deref() == Some(TIER3_LABEL) && t.pct > TIER3_CRITICAL_PCT_THRESHOLD
        });
        let drift_violation = !self.drift_alerts.is_empty();
        let critical_recommendation = self
            .recommendations
            .iter()
            .any(|r| r.severity == Severity::Critical);
        if p95_violation
            || strategy_c_under_prediction
            || tier3_violation
            || drift_violation
            || critical_recommendation
        {
            ReportExitCode::CriticalFindings
        } else {
            ReportExitCode::Success
        }
    }
}

/// Spec §2.3 critical threshold. Strategy B P95 1.50 is also the §8.1
/// "Critical" trigger for `Strategy B P95 ratio > 1.50 over 7 days`.
pub const CRITICAL_P95_THRESHOLD: f64 = 1.50;
/// Spec §8.1 Rule 4: Strategy C actual/predicted P95 > 1.05 is
/// critical when enough paired samples are present.
pub const STRATEGY_C_UNDER_PREDICTION_P95_THRESHOLD: f64 = 1.05;
pub const STRATEGY_C_MIN_SAMPLE_SIZE: i64 = 30;
/// Spec §8.1: Tier 3 hit rate > 0.1% → warning. Exit-code §2.3 promotes
/// the same threshold to "critical findings" for CI semantics.
pub const TIER3_CRITICAL_PCT_THRESHOLD: f64 = 0.1;
/// Stable string used by the §3.1 query.
pub const TIER3_LABEL: &str = "T3";

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn empty_report() -> Report {
        Report {
            tenant_id: "00000000-0000-4000-8000-000000000001".into(),
            window: Window {
                from: Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap(),
                to: Utc.with_ymd_and_hms(2026, 5, 30, 0, 0, 0).unwrap(),
            },
            proof_mode: "cache".into(),
            tier_distribution: vec![],
            calibration_ratios: vec![],
            drift_alerts: vec![],
            run_budget_projection_exceeded_count: 0,
            run_drift_detected_count: 0,
            run_total_count: 0,
            recommendations: vec![],
            verify_chain_run: false,
            verify_chain_failure: None,
        }
    }

    #[test]
    fn exit_zero_when_clean() {
        let r = empty_report();
        assert_eq!(r.exit_code(), ReportExitCode::Success);
    }

    #[test]
    fn exit_one_when_p95_violation() {
        let mut r = empty_report();
        r.calibration_ratios.push(CalibrationRatio {
            model: "gpt-4o".into(),
            strategy: "B".into(),
            p50: 1.0,
            p95: 1.6,
            p99: 2.0,
            sample_size: 100,
        });
        assert_eq!(r.exit_code(), ReportExitCode::CriticalFindings);
    }

    #[test]
    fn exit_one_when_strategy_c_under_prediction_is_critical() {
        let mut r = empty_report();
        r.calibration_ratios.push(CalibrationRatio {
            model: "gpt-4o".into(),
            strategy: "C".into(),
            p50: 1.01,
            p95: 1.08,
            p99: 1.12,
            sample_size: 50,
        });
        assert_eq!(r.exit_code(), ReportExitCode::CriticalFindings);
    }

    #[test]
    fn exit_zero_when_strategy_c_under_prediction_sample_is_too_small() {
        let mut r = empty_report();
        r.calibration_ratios.push(CalibrationRatio {
            model: "gpt-4o".into(),
            strategy: "C".into(),
            p50: 1.01,
            p95: 1.08,
            p99: 1.12,
            sample_size: 10,
        });
        assert_eq!(r.exit_code(), ReportExitCode::Success);
    }

    #[test]
    fn exit_one_when_tier3_burst() {
        let mut r = empty_report();
        r.tier_distribution.push(TierDistribution {
            tier: Some("T3".into()),
            count: 1500,
            pct: 1.5,
            threshold_violation: true,
        });
        assert_eq!(r.exit_code(), ReportExitCode::CriticalFindings);
    }

    #[test]
    fn exit_one_when_drift() {
        let mut r = empty_report();
        r.drift_alerts.push(DriftAlert {
            event_id: "11111111-1111-7000-a000-000000000001".into(),
            event_time: Utc.with_ymd_and_hms(2026, 5, 15, 14, 32, 0).unwrap(),
            bucket: "(gpt-4o, agent-x, chat_long)".into(),
            z_score: 2.4,
        });
        assert_eq!(r.exit_code(), ReportExitCode::CriticalFindings);
    }

    #[test]
    fn exit_three_when_verify_chain_failure_overrides() {
        let mut r = empty_report();
        // Even with a critical P95, verify-chain wins.
        r.calibration_ratios.push(CalibrationRatio {
            model: "gpt-4o".into(),
            strategy: "B".into(),
            p50: 1.0,
            p95: 1.6,
            p99: 2.0,
            sample_size: 100,
        });
        r.verify_chain_failure = Some(VerifyChainFailure {
            event_id: "deadbeef".into(),
            reason: "signature mismatch".into(),
        });
        assert_eq!(r.exit_code(), ReportExitCode::VerifyChainFailed);
    }

    #[test]
    fn exit_zero_when_tier3_below_threshold() {
        // Tier 3 at 0.05% must not promote to critical.
        let mut r = empty_report();
        r.tier_distribution.push(TierDistribution {
            tier: Some("T3".into()),
            count: 5,
            pct: 0.05,
            threshold_violation: false,
        });
        assert_eq!(r.exit_code(), ReportExitCode::Success);
    }

    #[test]
    fn exit_code_to_process_exit_code() {
        // Stable u8 mapping for CI consumers.
        assert_eq!(
            format!("{:?}", ReportExitCode::Success.to_process_exit_code()),
            format!("{:?}", std::process::ExitCode::from(0))
        );
        assert_eq!(
            format!(
                "{:?}",
                ReportExitCode::CriticalFindings.to_process_exit_code()
            ),
            format!("{:?}", std::process::ExitCode::from(1))
        );
        assert_eq!(
            format!("{:?}", ReportExitCode::QueryError.to_process_exit_code()),
            format!("{:?}", std::process::ExitCode::from(2))
        );
        assert_eq!(
            format!(
                "{:?}",
                ReportExitCode::VerifyChainFailed.to_process_exit_code()
            ),
            format!("{:?}", std::process::ExitCode::from(3))
        );
    }
}
