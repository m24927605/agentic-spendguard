//! Captures actual rendered output for the spec §4.1 example.
//!
//! Run with: `cargo test --test sample_output -- --nocapture`
//! Prints text + JSON + markdown variants so reviewers can confirm
//! the §4.1 sample matches.

use chrono::{TimeZone, Utc};
use spendguard_calibration_report::{
    cli::Format,
    formatters::{self, FormatOptions},
    recommendations,
    report::{
        CalibrationRatio, DriftAlert, Report, TierDistribution, Window,
    },
};

fn sample() -> Report {
    let mut r = Report {
        tenant_id: "acme-corp-tenant-uuid".into(),
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
                strategy: "A".into(),
                p50: 2.14,
                p95: 4.32,
                p99: 8.10,
                sample_size: 200,
            },
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
            CalibrationRatio {
                model: "claude-3-5-sonnet".into(),
                strategy: "B".into(),
                p50: 1.02,
                p95: 1.11,
                p99: 1.22,
                sample_size: 8_000,
            },
        ],
        drift_alerts: vec![
            DriftAlert {
                event_id: "id1".into(),
                event_time: Utc.with_ymd_and_hms(2026, 5, 15, 14, 32, 0).unwrap(),
                bucket: "(gpt-4o, support-agent, chat_long)".into(),
                z_score: 2.4,
            },
            DriftAlert {
                event_id: "id2".into(),
                event_time: Utc.with_ymd_and_hms(2026, 5, 20, 9, 18, 0).unwrap(),
                bucket: "(claude-3-5-sonnet, code-reviewer, code_gen)".into(),
                z_score: 2.1,
            },
            DriftAlert {
                event_id: "id3".into(),
                event_time: Utc.with_ymd_and_hms(2026, 5, 22, 11, 45, 0).unwrap(),
                bucket: "(gpt-4o, support-agent, chat_long)".into(),
                z_score: 2.6,
            },
        ],
        run_budget_projection_exceeded_count: 12,
        run_drift_detected_count: 0,
        run_total_count: 240,
        recommendations: vec![],
        verify_chain_run: false,
        verify_chain_failure: None,
    };
    r.recommendations = recommendations::evaluate(&r);
    r
}

#[test]
fn print_sample_text_output() {
    let r = sample();
    let opts = FormatOptions {
        include_recommendations: true,
        verify_chain_run: false,
    };
    let out = formatters::render(&r, Format::Text, &opts);
    println!("\n========== TEXT FORMAT SAMPLE ==========\n");
    println!("{out}");
    println!("\n========== END TEXT ==========");
    assert!(!out.is_empty());
}

#[test]
fn print_sample_json_output() {
    let r = sample();
    let opts = FormatOptions {
        include_recommendations: true,
        verify_chain_run: false,
    };
    let out = formatters::render(&r, Format::Json, &opts);
    println!("\n========== JSON FORMAT SAMPLE ==========\n");
    println!("{out}");
    println!("\n========== END JSON ==========");
    assert!(!out.is_empty());
}

#[test]
fn print_sample_markdown_output() {
    let r = sample();
    let opts = FormatOptions {
        include_recommendations: true,
        verify_chain_run: false,
    };
    let out = formatters::render(&r, Format::Markdown, &opts);
    println!("\n========== MARKDOWN FORMAT SAMPLE ==========\n");
    println!("{out}");
    println!("\n========== END MARKDOWN ==========");
    assert!(!out.is_empty());
}
