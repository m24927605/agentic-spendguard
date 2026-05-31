//! Markdown formatter — for Slack / Confluence / GitHub Issue paste.
//!
//! Spec ancestor: `docs/calibration-report-spec-v1alpha1.md` §4.3.
//!
//! ## Why markdown matters
//!
//! Spec §1.2 calls out three audiences:
//!   * Platform operator (Slack / oncall channel)
//!   * CFO / FinOps (Confluence)
//!   * Third-party auditor (GitHub Issue → PDF export)
//!
//! All three render markdown natively; the markdown formatter is the
//! "share this report" surface. Tables render correctly in GitHub +
//! Notion + Slack canvas; emoji-prefix indicators (✓/⚠) work in
//! monospace fallback (unlike ANSI colour codes).

use crate::formatters::FormatOptions;
use crate::report::{CalibrationRatio, Recommendation, Report, Severity, TierDistribution};

pub fn render(report: &Report, opts: &FormatOptions) -> String {
    let mut out = String::new();

    // ----- Header -----
    out.push_str("# SpendGuard Calibration Report\n\n");
    out.push_str(&format!("- **Tenant**: `{}`\n", report.tenant_id));
    out.push_str(&format!(
        "- **Window**: {} → {}\n",
        report.window.from.format("%Y-%m-%d %H:%M UTC"),
        report.window.to.format("%Y-%m-%d %H:%M UTC")
    ));
    out.push_str(&format!("- **Proof mode**: `{}`\n", report.proof_mode));
    if let Some(failure) = &report.verify_chain_failure {
        out.push_str(&format!(
            "- **Status**: ❌ ABORTED — verify-chain failed at event `{}` ({})\n",
            failure.event_id, failure.reason
        ));
    } else {
        let code = report.exit_code() as u8;
        let status = match code {
            0 => "✅ Pass",
            1 => "⚠️  Critical findings",
            _ => "❓ Unknown",
        };
        out.push_str(&format!("- **Exit code**: `{code}` ({status})\n"));
    }
    out.push('\n');

    // ----- Tier distribution table -----
    out.push_str("## Tokenizer tier distribution\n\n");
    out.push_str("| Tier | Description | Count | Percent | Threshold |\n");
    out.push_str("|---|---|---:|---:|---|\n");
    if report.tier_distribution.is_empty() {
        out.push_str("| _(none)_ | _(no decision events in window)_ |  |  |  |\n");
    } else {
        for tier in &report.tier_distribution {
            out.push_str(&format_tier_row(tier));
        }
    }
    out.push('\n');

    // ----- Calibration ratios table -----
    out.push_str("## Per-(model, strategy) calibration ratio\n\n");
    out.push_str(
        "Ratio is `actual_output_tokens / predicted_<strategy>_tokens`. \
         Healthy band per spec §7.2.\n\n",
    );
    out.push_str("| Model | Strategy | P50 | P95 | P99 | Samples | Health |\n");
    out.push_str("|---|---|---:|---:|---:|---:|---|\n");
    if report.calibration_ratios.is_empty() {
        out.push_str("| _(none)_ | | | | | | _(no paired events)_ |\n");
    } else {
        for r in &report.calibration_ratios {
            out.push_str(&format_calibration_row(r));
        }
    }
    out.push('\n');

    // ----- Drift alerts -----
    out.push_str("## Drift alerts\n\n");
    out.push_str(&format!(
        "- `prediction_drift_alert` events: **{}**\n",
        report.drift_alerts.len()
    ));
    out.push_str(&format!(
        "- `RUN_DRIFT_DETECTED` events: **{}**\n",
        report.run_drift_detected_count
    ));
    if report.run_total_count > 0 {
        let rate = report.run_budget_projection_exceeded_count as f64
            / report.run_total_count as f64
            * 100.0;
        out.push_str(&format!(
            "- `RUN_BUDGET_PROJECTION_EXCEEDED` events: **{}** ({:.1}% of runs)\n",
            report.run_budget_projection_exceeded_count, rate
        ));
    } else {
        out.push_str(&format!(
            "- `RUN_BUDGET_PROJECTION_EXCEEDED` events: **{}**\n",
            report.run_budget_projection_exceeded_count
        ));
    }
    if !report.drift_alerts.is_empty() {
        out.push_str("\n| Time | Bucket | z-score |\n|---|---|---:|\n");
        for d in &report.drift_alerts {
            out.push_str(&format!(
                "| {} | `{}` | {:.1} |\n",
                d.event_time.format("%Y-%m-%d %H:%M UTC"),
                d.bucket,
                d.z_score
            ));
        }
    }
    out.push('\n');

    // ----- Recommendations -----
    if opts.include_recommendations {
        out.push_str("## Recommendations\n\n");
        if report.recommendations.is_empty() {
            out.push_str("_(no rules triggered — report clean)_\n\n");
        } else {
            for (idx, rec) in report.recommendations.iter().enumerate() {
                out.push_str(&format_recommendation(idx + 1, rec));
            }
        }
    }

    // ----- Integrity attestation -----
    out.push_str("## Report integrity\n\n");
    if let Some(failure) = &report.verify_chain_failure {
        out.push_str(&format!(
            "❌ **verify-chain replay FAILED** at event `{}`: {}\n",
            failure.event_id, failure.reason
        ));
    } else if opts.verify_chain_run {
        out.push_str("✅ verify-chain replay passed; all rows cryptographically attested.\n");
    } else {
        out.push_str(
            "⚠️ verify-chain check NOT run. To validate cryptographic integrity, \
             re-run with `--verify-chain`.\n",
        );
    }

    out
}

fn format_tier_row(tier: &TierDistribution) -> String {
    let label = tier.tier.as_deref().unwrap_or("(unspecified)");
    let desc = match label {
        "T1" => "Provider API shadow",
        "T2" => "Local exact",
        "T3" => "Heuristic",
        _ => "Other",
    };
    let threshold = if tier.threshold_violation {
        "⚠ exceeds 0.1%"
    } else {
        "✓ within"
    };
    format!(
        "| `{}` | {} | {} | {:.1}% | {} |\n",
        label, desc, tier.count, tier.pct, threshold
    )
}

fn format_calibration_row(r: &CalibrationRatio) -> String {
    let health = match r.strategy.as_str() {
        "A" => "(ceiling)",
        _ => {
            if r.p95 > crate::report::CRITICAL_P95_THRESHOLD {
                "⚠ critical"
            } else if r.p95 > 1.30 {
                "⚠ warning"
            } else if r.p95 > 1.05 && r.strategy == "C" {
                "⚠ under-pred"
            } else {
                "✓ healthy"
            }
        }
    };
    format!(
        "| `{}` | `{}` | {:.2} | {:.2} | {:.2} | {} | {} |\n",
        r.model, r.strategy, r.p50, r.p95, r.p99, r.sample_size, health
    )
}

fn format_recommendation(idx: usize, rec: &Recommendation) -> String {
    let sev = match rec.severity {
        Severity::Info => "ℹ INFO",
        Severity::Warning => "⚠ WARNING",
        Severity::Critical => "🚨 CRITICAL",
    };
    format!(
        "### {}. `{}` — {}\n\n\
         **Severity**: {}\n\n\
         - **Possible cause**: {}\n\
         - **Suggested action**: {}\n\n",
        idx, rec.code, rec.headline, sev, rec.possible_cause, rec.suggested_action
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{
        CalibrationRatio, DriftAlert, Recommendation, Severity, TierDistribution, Window,
    };
    use chrono::{TimeZone, Utc};

    fn fixture() -> Report {
        Report {
            tenant_id: "00000000-0000-4000-8000-000000000001".into(),
            window: Window {
                from: Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap(),
                to: Utc.with_ymd_and_hms(2026, 5, 29, 0, 0, 0).unwrap(),
            },
            proof_mode: "cache".into(),
            tier_distribution: vec![TierDistribution {
                tier: Some("T3".into()),
                count: 15_000,
                pct: 1.5,
                threshold_violation: true,
            }],
            calibration_ratios: vec![CalibrationRatio {
                model: "gpt-4o".into(),
                strategy: "B".into(),
                p50: 1.04,
                p95: 1.18,
                p99: 1.34,
                sample_size: 50_000,
            }],
            drift_alerts: vec![DriftAlert {
                event_id: "id1".into(),
                event_time: Utc.with_ymd_and_hms(2026, 5, 15, 14, 32, 0).unwrap(),
                bucket: "(gpt-4o, support, chat_long)".into(),
                z_score: 2.4,
            }],
            run_budget_projection_exceeded_count: 12,
            run_drift_detected_count: 0,
            run_total_count: 240,
            recommendations: vec![Recommendation {
                severity: Severity::Warning,
                code: "TIER3_BURST".into(),
                headline: "Tier 3 hit rate 1.5%".into(),
                possible_cause: "Unknown fingerprints".into(),
                suggested_action: "Update dispatch".into(),
                details: serde_json::json!({}),
            }],
            verify_chain_run: false,
            verify_chain_failure: None,
        }
    }

    fn opts(include_recs: bool, verify_chain: bool) -> FormatOptions {
        FormatOptions {
            include_recommendations: include_recs,
            verify_chain_run: verify_chain,
        }
    }

    #[test]
    fn renders_top_level_header() {
        let r = fixture();
        let out = render(&r, &opts(true, false));
        assert!(out.starts_with("# SpendGuard Calibration Report\n"));
        assert!(out.contains("`00000000-0000-4000-8000-000000000001`"));
        assert!(out.contains("`cache`"));
    }

    #[test]
    fn renders_tier_table_with_alignment() {
        let r = fixture();
        let out = render(&r, &opts(true, false));
        assert!(out.contains("## Tokenizer tier distribution"));
        assert!(out.contains("| Tier | Description | Count | Percent | Threshold |"));
        // Right-align markers for the numeric columns.
        assert!(out.contains("|---|---|---:|---:|---|"));
        assert!(out.contains("| `T3` | Heuristic | 15000 | 1.5% | ⚠ exceeds 0.1% |"));
    }

    #[test]
    fn renders_calibration_table() {
        let r = fixture();
        let out = render(&r, &opts(true, false));
        assert!(out.contains("## Per-(model, strategy) calibration ratio"));
        assert!(out.contains("| Model | Strategy | P50 | P95 | P99 | Samples | Health |"));
        assert!(out.contains("| `gpt-4o` | `B` | 1.04 | 1.18 | 1.34 | 50000 | ✓ healthy |"));
    }

    #[test]
    fn renders_drift_table_with_details() {
        let r = fixture();
        let out = render(&r, &opts(true, false));
        assert!(out.contains("## Drift alerts"));
        assert!(out.contains("`prediction_drift_alert` events: **1**"));
        assert!(out.contains("| Time | Bucket | z-score |"));
        assert!(out.contains("| `(gpt-4o, support, chat_long)` | 2.4 |"));
    }

    #[test]
    fn recommendations_section_shown_when_enabled() {
        let r = fixture();
        let out = render(&r, &opts(true, false));
        assert!(out.contains("## Recommendations"));
        assert!(out.contains("### 1. `TIER3_BURST`"));
        assert!(out.contains("**Possible cause**: Unknown fingerprints"));
        assert!(out.contains("**Suggested action**: Update dispatch"));
    }

    #[test]
    fn recommendations_section_hidden_when_disabled() {
        let r = fixture();
        let out = render(&r, &opts(false, false));
        assert!(!out.contains("## Recommendations"));
    }

    #[test]
    fn integrity_section_default() {
        let r = fixture();
        let out = render(&r, &opts(true, false));
        assert!(out.contains("⚠️ verify-chain check NOT run"));
    }

    #[test]
    fn integrity_section_passed_when_verify_chain_run() {
        let r = fixture();
        let out = render(&r, &opts(true, true));
        assert!(out.contains("✅ verify-chain replay passed"));
    }

    #[test]
    fn integrity_section_failed() {
        let mut r = fixture();
        r.verify_chain_failure = Some(crate::report::VerifyChainFailure {
            event_id: "deadbeef".into(),
            reason: "sig mismatch".into(),
        });
        let out = render(&r, &opts(true, false));
        assert!(out.contains("❌ **verify-chain replay FAILED**"));
        assert!(out.contains("`deadbeef`"));
    }
}
