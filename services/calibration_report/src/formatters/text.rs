//! Text formatter — default operator output.
//!
//! Spec ancestor: `docs/calibration-report-spec-v1alpha1.md` §4.1.
//!
//! ## Layout
//!
//! Five sections, each preceded by an `=== HEADER ===` divider so a
//! `grep` from a SIEM tail picks them out cleanly:
//!
//!   1. Report header (tenant, window, proof mode).
//!   2. Tokenizer tier distribution.
//!   3. Per-(model, strategy) calibration ratio.
//!   4. Drift alerts (events + run-level summary).
//!   5. Recommendations (optional; controlled by `FormatOptions`).
//!   6. Integrity attestation (whether verify-chain was run).
//!
//! Monospace alignment uses fixed-width columns; the formatter targets
//! 100-col terminals. Operators on narrower terminals still get a
//! readable layout because each row is independent.

use crate::formatters::FormatOptions;
use crate::report::{CalibrationRatio, Recommendation, Report, Severity, TierDistribution};

pub fn render(report: &Report, opts: &FormatOptions) -> String {
    let mut out = String::new();

    // ----- Header -----
    out.push_str("SpendGuard Calibration Report\n");
    out.push_str(&format!("Tenant: {}\n", report.tenant_id));
    out.push_str(&format!(
        "Window: {} → {}\n",
        report.window.from.format("%Y-%m-%d %H:%M"),
        report.window.to.format("%Y-%m-%d %H:%M")
    ));
    out.push_str(&format!(
        "Proof mode: {} {}\n",
        report.proof_mode,
        if report.proof_mode == "cache" {
            "(use --proof-mode=canonical for tamper-evident proof)"
        } else {
            "(reads canonical_events directly — tamper-evident)"
        }
    ));
    out.push('\n');

    // ----- Tokenizer tier distribution (§4.1) -----
    out.push_str("=== Tokenizer tier distribution ===\n");
    if report.tier_distribution.is_empty() {
        out.push_str("  (no decision events in window)\n");
    } else {
        for tier in &report.tier_distribution {
            out.push_str(&format_tier_row(tier));
        }
    }
    out.push('\n');

    // ----- Calibration ratios -----
    out.push_str("=== Per-(model, strategy) calibration ratio (actual / predicted) ===\n");
    if report.calibration_ratios.is_empty() {
        out.push_str("  (no paired decision/outcome events in window)\n");
    } else {
        for r in &report.calibration_ratios {
            out.push_str(&format_calibration_row(r));
        }
    }
    out.push('\n');

    // ----- Drift alerts -----
    out.push_str("=== Drift alerts in window ===\n");
    out.push_str(&format!(
        "  prediction_drift_alert events: {}\n",
        report.drift_alerts.len()
    ));
    for d in &report.drift_alerts {
        out.push_str(&format!(
            "    - {}  bucket={}  z_score={:.1}\n",
            d.event_time.format("%Y-%m-%d %H:%M UTC"),
            d.bucket,
            d.z_score
        ));
    }
    out.push('\n');
    out.push_str(&format!(
        "  RUN_DRIFT_DETECTED events: {}\n",
        report.run_drift_detected_count
    ));
    out.push_str(&format!(
        "  RUN_BUDGET_PROJECTION_EXCEEDED events: {}",
        report.run_budget_projection_exceeded_count
    ));
    if report.run_total_count > 0 {
        let rate = report.run_budget_projection_exceeded_count as f64
            / report.run_total_count as f64
            * 100.0;
        out.push_str(&format!("  ({:.1}% of runs)", rate));
    }
    out.push_str("\n\n");

    // ----- Recommendations -----
    if opts.include_recommendations && !report.recommendations.is_empty() {
        out.push_str("=== Recommendations ===\n");
        for (idx, rec) in report.recommendations.iter().enumerate() {
            out.push_str(&format_recommendation(idx + 1, rec));
        }
        out.push('\n');
    } else if opts.include_recommendations {
        out.push_str("=== Recommendations ===\n");
        out.push_str("  (no rules triggered — report clean)\n\n");
    }

    // ----- Integrity attestation -----
    if let Some(failure) = &report.verify_chain_failure {
        out.push_str(&format!(
            "Report integrity: ABORTED — verify-chain failed at event {} ({})\n",
            failure.event_id, failure.reason
        ));
    } else if opts.verify_chain_run {
        out.push_str(
            "Report integrity: verify-chain replay passed; all rows cryptographically attested.\n",
        );
    } else {
        out.push_str(
            "Report integrity: verify-chain check NOT run.\n   \
             To validate cryptographic integrity, re-run with --verify-chain.\n",
        );
    }

    out
}

fn format_tier_row(tier: &TierDistribution) -> String {
    let label = match tier.tier.as_deref() {
        Some("T1") => "Tier 1 (provider API shadow)",
        Some("T2") => "Tier 2 (local exact)         ",
        Some("T3") => "Tier 3 (heuristic)           ",
        Some(other) => return format!("  {:30}  {:.1}%   ({} events)\n", other, tier.pct, tier.count),
        None => "(unspecified)                ",
    };
    let warning_marker = if tier.threshold_violation {
        "        ⚠ exceeds 0.1% target — see recommendations"
    } else {
        ""
    };
    format!(
        "  {}: {:>6.1}%   ({} events){}\n",
        label, tier.pct, tier.count, warning_marker
    )
}

fn format_calibration_row(r: &CalibrationRatio) -> String {
    let health_marker = if r.p95 > crate::report::CRITICAL_P95_THRESHOLD {
        "  ⚠ P95 exceeds 1.50 critical threshold"
    } else if r.p95 > 1.30 {
        "  ⚠ P95 exceeds 1.30 warning threshold"
    } else if r.strategy == "A" {
        "  (ceiling; expected conservative ratio)"
    } else if r.p95 > 1.05 && r.strategy == "C" {
        "  ⚠ under-prediction (C P95 > 1.05)"
    } else {
        "  ✓ healthy"
    };
    format!(
        "  {:24} + Strategy {}:  P50={:>5.2}  P95={:>5.2}  P99={:>5.2}  (n={}){}\n",
        truncate(&r.model, 24),
        r.strategy,
        r.p50,
        r.p95,
        r.p99,
        r.sample_size,
        health_marker
    )
}

fn format_recommendation(idx: usize, rec: &Recommendation) -> String {
    let sev_label = match rec.severity {
        Severity::Info => "INFO",
        Severity::Warning => "WARNING",
        Severity::Critical => "CRITICAL",
    };
    format!(
        "  {}. [{}] {}\n     Possible cause: {}\n     Suggested action: {}\n\n",
        idx, sev_label, rec.headline, rec.possible_cause, rec.suggested_action
    )
}

fn truncate(s: &str, n: usize) -> &str {
    if s.len() <= n {
        s
    } else {
        // Truncate at a char boundary so we never split a UTF-8
        // sequence (model names can contain non-ASCII).
        let mut end = n;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        &s[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{DriftAlert, Recommendation, Window};
    use chrono::{TimeZone, Utc};

    fn fixture() -> Report {
        Report {
            tenant_id: "00000000-0000-4000-8000-000000000001".into(),
            window: Window {
                from: Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap(),
                to: Utc.with_ymd_and_hms(2026, 5, 29, 0, 0, 0).unwrap(),
            },
            proof_mode: "cache".into(),
            tier_distribution: vec![
                TierDistribution {
                    tier: Some("T2".into()),
                    count: 985_000,
                    pct: 98.5,
                    threshold_violation: false,
                },
                TierDistribution {
                    tier: Some("T3".into()),
                    count: 15_000,
                    pct: 1.5,
                    threshold_violation: true,
                },
            ],
            calibration_ratios: vec![
                CalibrationRatio {
                    model: "gpt-4o".into(),
                    strategy: "B".into(),
                    p50: 1.04,
                    p95: 1.18,
                    p99: 1.34,
                    sample_size: 50_000,
                },
                CalibrationRatio {
                    model: "gpt-4o".into(),
                    strategy: "C".into(),
                    p50: 0.98,
                    p95: 1.05,
                    p99: 1.12,
                    sample_size: 12_000,
                },
            ],
            drift_alerts: vec![DriftAlert {
                event_id: "11111111-1111-7000-a000-000000000001".into(),
                event_time: Utc.with_ymd_and_hms(2026, 5, 15, 14, 32, 0).unwrap(),
                bucket: "(gpt-4o, support-agent, chat_long)".into(),
                z_score: 2.4,
            }],
            run_budget_projection_exceeded_count: 12,
            run_drift_detected_count: 0,
            run_total_count: 240,
            recommendations: vec![Recommendation {
                severity: Severity::Warning,
                code: "TIER3_BURST".into(),
                headline: "Tier 3 hit rate 1.5% exceeds 0.1% target".into(),
                possible_cause: "Unknown model fingerprints in dispatch table".into(),
                suggested_action: "Add top contributing models to dispatch table".into(),
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
    fn renders_header_section() {
        let r = fixture();
        let out = render(&r, &opts(true, false));
        assert!(out.contains("SpendGuard Calibration Report"));
        assert!(out.contains("Tenant: 00000000-0000-4000-8000-000000000001"));
        assert!(out.contains("Window: 2026-05-01 00:00 → 2026-05-29 00:00"));
        assert!(out.contains("Proof mode: cache"));
    }

    #[test]
    fn renders_tier_distribution() {
        let r = fixture();
        let out = render(&r, &opts(true, false));
        assert!(out.contains("=== Tokenizer tier distribution ==="));
        assert!(out.contains("Tier 2"));
        assert!(out.contains("Tier 3"));
        assert!(out.contains("98.5%"));
        assert!(out.contains("1.5%"));
        // T3 must show the warning marker because threshold_violation
        // is true on the fixture.
        assert!(out.contains("exceeds 0.1% target"));
    }

    #[test]
    fn renders_calibration_ratios() {
        let r = fixture();
        let out = render(&r, &opts(true, false));
        assert!(out.contains("=== Per-(model, strategy) calibration ratio"));
        assert!(out.contains("gpt-4o"));
        assert!(out.contains("Strategy B"));
        assert!(out.contains("P50= 1.04"));
        assert!(out.contains("P95= 1.18"));
        assert!(out.contains("✓ healthy"));
    }

    #[test]
    fn renders_drift_alerts() {
        let r = fixture();
        let out = render(&r, &opts(true, false));
        assert!(out.contains("=== Drift alerts in window ==="));
        assert!(out.contains("prediction_drift_alert events: 1"));
        assert!(out.contains("(gpt-4o, support-agent, chat_long)"));
        assert!(out.contains("z_score=2.4"));
        assert!(out.contains("RUN_BUDGET_PROJECTION_EXCEEDED events: 12"));
        assert!(out.contains("5.0% of runs"));
    }

    #[test]
    fn renders_recommendations_when_enabled() {
        let r = fixture();
        let out = render(&r, &opts(true, false));
        assert!(out.contains("=== Recommendations ==="));
        assert!(out.contains("TIER3_BURST") || out.contains("Tier 3 hit rate"));
        assert!(out.contains("Possible cause:"));
        assert!(out.contains("Suggested action:"));
    }

    #[test]
    fn suppresses_recommendations_when_disabled() {
        let r = fixture();
        let out = render(&r, &opts(false, false));
        assert!(!out.contains("=== Recommendations ==="));
    }

    #[test]
    fn empty_recommendation_section_shows_clean_message() {
        let mut r = fixture();
        r.recommendations.clear();
        let out = render(&r, &opts(true, false));
        assert!(out.contains("(no rules triggered — report clean)"));
    }

    #[test]
    fn renders_integrity_attestation_default() {
        let r = fixture();
        let out = render(&r, &opts(true, false));
        assert!(out.contains("verify-chain check NOT run"));
        assert!(out.contains("re-run with --verify-chain"));
    }

    #[test]
    fn renders_integrity_attestation_after_verify_chain() {
        let r = fixture();
        let out = render(&r, &opts(true, true));
        assert!(out.contains("verify-chain replay passed"));
    }

    #[test]
    fn renders_integrity_attestation_after_failure() {
        let mut r = fixture();
        r.verify_chain_failure = Some(crate::report::VerifyChainFailure {
            event_id: "deadbeef".into(),
            reason: "signature mismatch".into(),
        });
        let out = render(&r, &opts(true, true));
        assert!(out.contains("ABORTED — verify-chain failed"));
        assert!(out.contains("deadbeef"));
        assert!(out.contains("signature mismatch"));
    }

    #[test]
    fn renders_empty_window_gracefully() {
        let mut r = fixture();
        r.tier_distribution.clear();
        r.calibration_ratios.clear();
        let out = render(&r, &opts(true, false));
        assert!(out.contains("(no decision events in window)"));
        assert!(out.contains("(no paired decision/outcome events in window)"));
    }

    #[test]
    fn truncate_preserves_utf8_boundaries() {
        // Multi-byte char must not split.
        let s = "abc日本語";
        let t = truncate(s, 4);
        // 'a','b','c' are 1 byte each; '日' is 3 bytes — at idx 4 we
        // are mid-char so truncate steps back to idx 3.
        assert_eq!(t, "abc");
    }

    #[test]
    fn calibration_row_marks_critical_p95() {
        let r = Report {
            tenant_id: "x".into(),
            window: Window {
                from: Utc::now(),
                to: Utc::now() + chrono::Duration::days(1),
            },
            proof_mode: "cache".into(),
            tier_distribution: vec![],
            calibration_ratios: vec![CalibrationRatio {
                model: "gpt-4o".into(),
                strategy: "B".into(),
                p50: 1.0,
                p95: 1.6,
                p99: 2.0,
                sample_size: 100,
            }],
            drift_alerts: vec![],
            run_budget_projection_exceeded_count: 0,
            run_drift_detected_count: 0,
            run_total_count: 0,
            recommendations: vec![],
            verify_chain_run: false,
            verify_chain_failure: None,
        };
        let out = render(&r, &opts(false, false));
        assert!(out.contains("P95 exceeds 1.50"));
    }

    #[test]
    fn strategy_a_high_actual_over_predicted_ratio_marks_critical() {
        let mut r = fixture();
        r.calibration_ratios = vec![CalibrationRatio {
            model: "gpt-4o".into(),
            strategy: "A".into(),
            p50: 1.1,
            p95: 1.6,
            p99: 2.0,
            sample_size: 100,
        }];
        let out = render(&r, &opts(false, false));
        assert!(out.contains("P95 exceeds 1.50 critical threshold"));
        assert!(!out.contains("expected conservative ratio"));
    }
}
