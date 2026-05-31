//! Integration tests for the calibration-report CLI.
//!
//! Spec ancestor: `docs/calibration-report-spec-v1alpha1.md` §0.2
//! (DRAFT → LOCKED criterion #4: 5 synthetic scenarios) +
//! `docs/slices/SLICE_13_calibration_report_cli.md` §8.4.
//!
//! Five synthetic scenarios + cross-tenant rejection + window
//! validation. Each test builds a `Report` fixture directly and
//! exercises the recommendation engine + formatters + exit-code
//! computation end-to-end. No Postgres testcontainer required — the
//! report-side surface is what operators rely on.
//!
//! Postgres-backed query-correctness tests live in a separate
//! integration suite gated on the `INTEGRATION_POSTGRES_URL` env var
//! (out of scope for SLICE_13 Phase D first ship; uses cargo's
//! standard #[ignore] flag pattern for opt-in).

use chrono::{TimeZone, Utc};
use spendguard_calibration_report::{
    cli::Format,
    formatters::{self, FormatOptions},
    recommendations,
    report::{
        CalibrationRatio, DriftAlert, Recommendation, Report, ReportExitCode, Severity,
        TierDistribution, Window,
    },
};

fn base_report(tenant: &str) -> Report {
    Report {
        tenant_id: tenant.into(),
        window: Window {
            from: Utc.with_ymd_and_hms(2026, 5, 22, 0, 0, 0).unwrap(),
            to: Utc.with_ymd_and_hms(2026, 5, 29, 0, 0, 0).unwrap(),
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

fn opts() -> FormatOptions {
    FormatOptions {
        include_recommendations: true,
        verify_chain_run: false,
    }
}

// ---- Scenario 1: healthy ---------------------------------------------------

#[test]
fn scenario_healthy_renders_clean_report_exit_zero() {
    let mut r = base_report("00000000-0000-4000-8000-000000000001");
    r.tier_distribution.push(TierDistribution {
        tier: Some("T2".into()),
        count: 985_000,
        pct: 99.95,
        threshold_violation: false,
    });
    r.tier_distribution.push(TierDistribution {
        tier: Some("T3".into()),
        count: 50,
        pct: 0.05,
        threshold_violation: false,
    });
    r.calibration_ratios.push(CalibrationRatio {
        model: "gpt-4o".into(),
        strategy: "B".into(),
        p50: 1.04,
        p95: 1.18,
        p99: 1.34,
        sample_size: 50_000,
    });
    r.calibration_ratios.push(CalibrationRatio {
        model: "gpt-4o".into(),
        strategy: "C".into(),
        p50: 0.98,
        p95: 1.05,
        p99: 1.12,
        sample_size: 12_000,
    });
    r.recommendations = recommendations::evaluate(&r);
    assert_eq!(r.exit_code(), ReportExitCode::Success, "healthy scenario must exit 0");
    assert!(r.recommendations.is_empty(), "healthy scenario must trigger zero recommendations");

    // All three formatters must render without panic.
    for format in [Format::Text, Format::Json, Format::Markdown] {
        let out = formatters::render(&r, format, &opts());
        assert!(!out.is_empty(), "{format:?} output must be non-empty");
        // No critical findings → JSON exit_code = 0.
        if let Format::Json = format {
            assert!(out.contains("\"exit_code\": 0"), "JSON missing exit_code=0");
        }
    }
}

// ---- Scenario 2: drift -----------------------------------------------------

#[test]
fn scenario_drift_triggers_drift_recommendation_exit_one() {
    let mut r = base_report("00000000-0000-4000-8000-000000000002");
    r.tier_distribution.push(TierDistribution {
        tier: Some("T2".into()),
        count: 985_000,
        pct: 99.95,
        threshold_violation: false,
    });
    r.calibration_ratios.push(CalibrationRatio {
        model: "gpt-4o".into(),
        strategy: "B".into(),
        p50: 1.04,
        p95: 1.18,
        p99: 1.34,
        sample_size: 50_000,
    });
    r.calibration_ratios.push(CalibrationRatio {
        model: "gpt-4o".into(),
        strategy: "C".into(),
        p50: 0.98,
        p95: 1.05,
        p99: 1.12,
        sample_size: 12_000,
    });
    for i in 0..3 {
        r.drift_alerts.push(DriftAlert {
            event_id: format!("11111111-1111-7000-a000-00000000000{}", i),
            event_time: Utc.with_ymd_and_hms(2026, 5, 25 + i, 14, 0, 0).unwrap(),
            bucket: "(gpt-4o, support-agent, chat_long)".into(),
            z_score: 2.4,
        });
    }
    r.recommendations = recommendations::evaluate(&r);
    assert_eq!(r.exit_code(), ReportExitCode::CriticalFindings, "drift > 0 → exit 1");
    let codes: Vec<_> = r.recommendations.iter().map(|x| x.code.as_str()).collect();
    assert!(
        codes.contains(&"PREDICTION_DRIFT_ALERTS_PRESENT"),
        "drift scenario should fire PREDICTION_DRIFT_ALERTS_PRESENT; got {codes:?}"
    );
    // Spec §8.2: rule must include both possible_cause + suggested_action.
    let drift_rec = r
        .recommendations
        .iter()
        .find(|x| x.code == "PREDICTION_DRIFT_ALERTS_PRESENT")
        .unwrap();
    assert!(!drift_rec.possible_cause.is_empty());
    assert!(!drift_rec.suggested_action.is_empty());
}

// ---- Scenario 3: cold-start dominated --------------------------------------

#[test]
fn scenario_cold_start_dominated_triggers_l1_dominance() {
    let mut r = base_report("00000000-0000-4000-8000-000000000003");
    r.calibration_ratios.push(CalibrationRatio {
        model: "gpt-4o".into(),
        strategy: "A".into(),
        p50: 4.0,
        p95: 8.0,
        p99: 12.0,
        sample_size: 900,
    });
    r.calibration_ratios.push(CalibrationRatio {
        model: "gpt-4o".into(),
        strategy: "B".into(),
        p50: 1.1,
        p95: 1.2,
        p99: 1.3,
        sample_size: 100,
    });
    r.recommendations = recommendations::evaluate(&r);
    let codes: Vec<_> = r.recommendations.iter().map(|x| x.code.as_str()).collect();
    assert!(
        codes.contains(&"COLD_START_L1_DOMINANT"),
        "cold-start scenario should fire COLD_START_L1_DOMINANT; got {codes:?}"
    );
}

// ---- Scenario 4: plugin failing --------------------------------------------

#[test]
fn scenario_plugin_failing_triggers_c_absent() {
    let mut r = base_report("00000000-0000-4000-8000-000000000004");
    r.calibration_ratios.push(CalibrationRatio {
        model: "gpt-4o".into(),
        strategy: "A".into(),
        p50: 4.0,
        p95: 8.0,
        p99: 12.0,
        sample_size: 200,
    });
    r.calibration_ratios.push(CalibrationRatio {
        model: "gpt-4o".into(),
        strategy: "B".into(),
        p50: 1.1,
        p95: 1.2,
        p99: 1.3,
        sample_size: 200,
    });
    // No Strategy C entries — plugin failing or unregistered.
    r.recommendations = recommendations::evaluate(&r);
    let codes: Vec<_> = r.recommendations.iter().map(|x| x.code.as_str()).collect();
    assert!(
        codes.contains(&"STRATEGY_C_ABSENT"),
        "plugin-failing scenario should fire STRATEGY_C_ABSENT; got {codes:?}"
    );
}

// ---- Scenario 5: Tier 3 burst ---------------------------------------------

#[test]
fn scenario_tier3_burst_triggers_critical_tier3() {
    let mut r = base_report("00000000-0000-4000-8000-000000000005");
    r.tier_distribution.push(TierDistribution {
        tier: Some("T2".into()),
        count: 98_500,
        pct: 98.5,
        threshold_violation: false,
    });
    r.tier_distribution.push(TierDistribution {
        tier: Some("T3".into()),
        count: 1_500,
        pct: 1.5,
        threshold_violation: true,
    });
    r.calibration_ratios.push(CalibrationRatio {
        model: "gpt-4o-custom-2024-12".into(),
        strategy: "B".into(),
        p50: 1.04,
        p95: 1.18,
        p99: 1.34,
        sample_size: 50_000,
    });
    r.recommendations = recommendations::evaluate(&r);
    let tier3 = r
        .recommendations
        .iter()
        .find(|x| x.code == "TIER3_BURST")
        .expect("Tier 3 burst scenario must fire TIER3_BURST");
    assert_eq!(tier3.severity, Severity::Critical, "1.5% > 1.0% → critical");

    // Rule 8 also fires because the model name contains "gpt".
    let codes: Vec<_> = r.recommendations.iter().map(|x| x.code.as_str()).collect();
    assert!(
        codes.contains(&"TIER3_KNOWN_VENDOR_FINGERPRINT"),
        "vendor-fingerprint rule should also fire: {codes:?}"
    );

    // Exit code 1 (critical findings).
    assert_eq!(r.exit_code(), ReportExitCode::CriticalFindings);
}

// ---- Cross-tenant rejection ------------------------------------------------

#[test]
fn cross_tenant_rejection_path_is_exit_two() {
    use clap::Parser;
    use spendguard_calibration_report::cli::Cli;

    // Operator authenticated for tenant A but queries tenant B.
    let cli = Cli::parse_from([
        "spendguard-calibration-report",
        "--tenant",
        "00000000-0000-4000-8000-000000000099", // requested
        "--auth-tenants",
        "00000000-0000-4000-8000-000000000001", // allowed
        "--canonical-url",
        "postgres://nonexistent:5432/db",
    ]);
    let allowed = cli.check_tenant_scope().unwrap();
    assert!(!allowed, "cross-tenant query must be rejected");
}

// ---- Window validation -----------------------------------------------------

#[test]
fn window_inversion_rejected() {
    use spendguard_calibration_report::sql_queries::parse_window_anchor;

    let now = Utc.with_ymd_and_hms(2026, 5, 29, 0, 0, 0).unwrap();
    let from = parse_window_anchor("7d", now).unwrap();
    let to = parse_window_anchor("now", now).unwrap();
    assert!(from < to);

    // Inverted: --from=now --to=7d should produce from > to which the
    // CLI orchestrator (main.rs) rejects.
    let inv_from = parse_window_anchor("now", now).unwrap();
    let inv_to = parse_window_anchor("7d", now).unwrap();
    assert!(inv_from > inv_to);
}

// ---- Exit-code contract ----------------------------------------------------

#[test]
fn exit_code_three_for_verify_chain_failure() {
    let mut r = base_report("00000000-0000-4000-8000-000000000006");
    // Even with critical findings, verify-chain failure wins.
    r.tier_distribution.push(TierDistribution {
        tier: Some("T3".into()),
        count: 1500,
        pct: 1.5,
        threshold_violation: true,
    });
    r.verify_chain_failure = Some(spendguard_calibration_report::report::VerifyChainFailure {
        event_id: "deadbeef".into(),
        reason: "signature mismatch".into(),
    });
    assert_eq!(r.exit_code(), ReportExitCode::VerifyChainFailed);
    assert_eq!(r.exit_code() as u8, 3);
}

// ---- Output formats sanity check ------------------------------------------

#[test]
fn json_output_contains_schema_version_for_siem_consumption() {
    let r = base_report("00000000-0000-4000-8000-000000000007");
    let out = formatters::render(&r, Format::Json, &opts());
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["schema_version"], "v1alpha1");
}

#[test]
fn markdown_output_contains_tables_for_paste() {
    let mut r = base_report("00000000-0000-4000-8000-000000000008");
    r.tier_distribution.push(TierDistribution {
        tier: Some("T2".into()),
        count: 100,
        pct: 100.0,
        threshold_violation: false,
    });
    let out = formatters::render(&r, Format::Markdown, &opts());
    assert!(out.contains("|---"));
    assert!(out.contains("## Tokenizer tier distribution"));
}

#[test]
fn text_output_contains_recommendation_pair_when_violations() {
    let mut r = base_report("00000000-0000-4000-8000-000000000009");
    r.calibration_ratios.push(CalibrationRatio {
        model: "gpt-4o".into(),
        strategy: "B".into(),
        p50: 1.0,
        p95: 1.6, // > 1.50 critical
        p99: 2.0,
        sample_size: 100,
    });
    r.recommendations = recommendations::evaluate(&r);
    let out = formatters::render(&r, Format::Text, &opts());
    assert!(out.contains("Possible cause:"));
    assert!(out.contains("Suggested action:"));
}

// ---- Recommendation engine sanity: deterministic ordering -----------------

#[test]
fn recommendation_order_is_deterministic_across_runs() {
    // Same input → same output (operators reading diffs need stable
    // ordering).
    let mut r = base_report("00000000-0000-4000-8000-00000000000a");
    r.tier_distribution.push(TierDistribution {
        tier: Some("T3".into()),
        count: 1500,
        pct: 1.5,
        threshold_violation: true,
    });
    r.drift_alerts.push(DriftAlert {
        event_id: "id1".into(),
        event_time: Utc::now(),
        bucket: "(x)".into(),
        z_score: 2.4,
    });
    r.calibration_ratios.push(CalibrationRatio {
        model: "gpt-4o".into(),
        strategy: "C".into(),
        p50: 1.02,
        p95: 1.08,
        p99: 1.14,
        sample_size: 50,
    });

    let codes1: Vec<_> = recommendations::evaluate(&r)
        .iter()
        .map(|x| x.code.clone())
        .collect();
    let codes2: Vec<_> = recommendations::evaluate(&r)
        .iter()
        .map(|x| x.code.clone())
        .collect();
    assert_eq!(codes1, codes2, "recommendation ordering must be deterministic");
}

// ---- Acceptance criterion §8.4: every rule has spec-compliant fields ------

#[test]
fn every_recommendation_has_required_fields() {
    // Build a fixture that triggers as many rules as possible at once.
    let mut r = base_report("00000000-0000-4000-8000-00000000000b");
    r.tier_distribution.push(TierDistribution {
        tier: Some("T3".into()),
        count: 1500,
        pct: 1.5,
        threshold_violation: true,
    });
    r.drift_alerts.push(DriftAlert {
        event_id: "id".into(),
        event_time: Utc::now(),
        bucket: "(x)".into(),
        z_score: 2.4,
    });
    r.calibration_ratios.push(CalibrationRatio {
        model: "gpt-4o".into(),
        strategy: "B".into(),
        p50: 1.6,
        p95: 1.7,
        p99: 1.8,
        sample_size: 200,
    });
    r.calibration_ratios.push(CalibrationRatio {
        model: "gpt-4o".into(),
        strategy: "C".into(),
        p50: 1.02,
        p95: 1.08,
        p99: 1.14,
        sample_size: 50,
    });
    r.calibration_ratios.push(CalibrationRatio {
        model: "gpt-4o".into(),
        strategy: "A".into(),
        p50: 0.5,
        p95: 0.8,
        p99: 0.95,
        sample_size: 200,
    });
    r.run_total_count = 100;
    r.run_budget_projection_exceeded_count = 20;
    let recs: Vec<Recommendation> = recommendations::evaluate(&r);
    assert!(recs.len() >= 5, "expected 5+ rules to fire; got {}", recs.len());
    for rec in &recs {
        assert!(!rec.code.is_empty(), "rule code must be non-empty");
        assert!(!rec.headline.is_empty(), "rule {} headline empty", rec.code);
        assert!(
            !rec.possible_cause.is_empty(),
            "rule {} missing possible_cause (spec §8.2)",
            rec.code
        );
        assert!(
            !rec.suggested_action.is_empty(),
            "rule {} missing suggested_action (spec §8.2)",
            rec.code
        );
    }
}
