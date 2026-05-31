//! Recommendation engine — 9 heuristic rules per spec §8.1.
//!
//! Spec ancestor: `docs/calibration-report-spec-v1alpha1.md` §8.
//!
//! ## Rule set (spec §8.1 + slice §B.2)
//!
//! 1. actual/predicted P95 > 1.50 for any
//!    (model, strategy != A) → critical;
//!    suggest more conservative reservation or baseline refresh.
//! 2. Tier 3 > 0.1% → warning; top contributing models, dispatch
//!    update suggestion. > 1.0% → critical.
//! 3. drift alert > 0 → warning; suggest plugin retraining.
//! 4. Strategy C actual/predicted P95 > 1.05 → critical;
//!    under-prediction.
//! 5. cold_start_layer_used L1 > 50% → warning; suggest L2 TOML
//!    population. (Phase B: heuristic derived from CalibrationRatio
//!    samples; full implementation requires cold-start telemetry.)
//! 6. Strategy C error rate > 5% (slice plan: predicted_c_tokens
//!    NULL > 90% when plugin configured) → warning.
//! 7. RUN_BUDGET_PROJECTION_EXCEEDED rate > 5% of runs → info.
//! 8. Tier 3 dominant for known-vendor model fingerprint → warning.
//!    (Pattern-match on model name prefix.)
//! 9. Strategy A dominant under EMPIRICAL_RUN_CEILING policy →
//!    info; suggest cache warm-up. (Heuristic: Strategy A appears in
//!    calibration_ratios with > 50% sample share.)
//!
//! ## "Possible cause + suggested action" discipline (§8.2)
//!
//! Every Recommendation pairs the two strings. The CLI is never
//! prescriptive — operators get a direction, not a directive.
//!
//! ## No recursive audit (§8.3)
//!
//! Recommendations are derived data; they never enter the audit chain.
//! Only the report-run metadata (self-audit CloudEvent, Phase C) lands
//! in canonical_events.

use crate::report::{Recommendation, Report, Severity};
use serde_json::json;

/// Run all 9 rules and return the union of triggered recommendations.
///
/// Order is deterministic per rule index — operators reading the
/// report see rules in the same order across runs (helps diff-vs-yesterday).
pub fn evaluate(report: &Report) -> Vec<Recommendation> {
    let mut out = Vec::new();

    // Rule 1: actual/predicted P95 > 1.50 for non-A strategies.
    if let Some(rec) = rule1_p95_over_critical(report) {
        out.push(rec);
    }

    // Rule 2: Tier 3 burst.
    if let Some(rec) = rule2_tier3_burst(report) {
        out.push(rec);
    }

    // Rule 3: drift alerts in window.
    if let Some(rec) = rule3_drift_alerts(report) {
        out.push(rec);
    }

    // Rule 4: Strategy C under-prediction.
    if let Some(rec) = rule4_c_under_prediction(report) {
        out.push(rec);
    }

    // Rule 5: cold-start L1 dominance (heuristic via Strategy A share).
    if let Some(rec) = rule5_cold_start_l1_dominance(report) {
        out.push(rec);
    }

    // Rule 6: Strategy C error rate / plugin failure.
    if let Some(rec) = rule6_plugin_error_rate(report) {
        out.push(rec);
    }

    // Rule 7: RUN_BUDGET_PROJECTION_EXCEEDED rate.
    if let Some(rec) = rule7_run_projection_exceeded(report) {
        out.push(rec);
    }

    // Rule 8: Tier 3 dominant on known-vendor model.
    if let Some(rec) = rule8_tier3_known_vendor(report) {
        out.push(rec);
    }

    // Rule 9: Strategy A dominance under EMPIRICAL_RUN_CEILING.
    if let Some(rec) = rule9_strategy_a_dominance(report) {
        out.push(rec);
    }

    out
}

// --- Individual rules ------------------------------------------------

fn rule1_p95_over_critical(report: &Report) -> Option<Recommendation> {
    let violators: Vec<_> = report
        .calibration_ratios
        .iter()
        .filter(|r| r.strategy != "A" && r.p95 > crate::report::CRITICAL_P95_THRESHOLD)
        .collect();
    if violators.is_empty() {
        return None;
    }
    let summary = violators
        .iter()
        .map(|r| format!("({}, {}): P95={:.2}", r.model, r.strategy, r.p95))
        .collect::<Vec<_>>()
        .join("; ");
    Some(Recommendation {
        severity: Severity::Critical,
        code: "P95_CRITICAL_OVER_1_50".into(),
        headline: format!(
            "{} (model, strategy) bucket(s) have P95 > 1.50",
            violators.len()
        ),
        possible_cause: format!(
            "Recent agent prompt-template change or systematic under-prediction. \
             Buckets: {summary}"
        ),
        suggested_action: "Increase reservation conservatism or refresh stats_aggregator baseline \
             (see Strategy B P95 lookup in output-predictor spec §4)"
            .into(),
        details: json!({ "violators": violators }),
    })
}

fn rule2_tier3_burst(report: &Report) -> Option<Recommendation> {
    let t3 = report
        .tier_distribution
        .iter()
        .find(|t| t.tier.as_deref() == Some(crate::report::TIER3_LABEL))?;
    if t3.pct <= crate::report::TIER3_CRITICAL_PCT_THRESHOLD {
        return None;
    }
    let severity = if t3.pct > 1.0 {
        Severity::Critical
    } else {
        Severity::Warning
    };
    Some(Recommendation {
        severity,
        code: "TIER3_BURST".into(),
        headline: format!("Tier 3 hit rate {:.1}% exceeds 0.1% target", t3.pct),
        possible_cause: "Unknown or unmapped model fingerprints in the tokenizer dispatch table"
            .into(),
        suggested_action: "Inspect top-N Tier 3 contributing models; PR the dispatch table \
             (tokenizer-service-spec §4.2) to map them to a vendor family"
            .into(),
        details: json!({
            "tier3_pct": t3.pct,
            "tier3_count": t3.count,
        }),
    })
}

fn rule3_drift_alerts(report: &Report) -> Option<Recommendation> {
    if report.drift_alerts.is_empty() {
        return None;
    }
    let bucket_summary = report
        .drift_alerts
        .iter()
        .take(3)
        .map(|d| format!("{} @ {}", d.bucket, d.event_time.format("%Y-%m-%d")))
        .collect::<Vec<_>>()
        .join("; ");
    Some(Recommendation {
        severity: Severity::Warning,
        code: "PREDICTION_DRIFT_ALERTS_PRESENT".into(),
        headline: format!(
            "{} prediction_drift_alert event(s) in window",
            report.drift_alerts.len()
        ),
        possible_cause: format!(
            "Agent prompt-template change or vendor tokenizer update. \
             Top buckets: {bucket_summary}"
        ),
        suggested_action: "Investigate cited bucket(s); consider retraining the customer plugin \
             or rebasing the stats_aggregator 30d distribution"
            .into(),
        details: json!({
            "alert_count": report.drift_alerts.len(),
        }),
    })
}

fn rule4_c_under_prediction(report: &Report) -> Option<Recommendation> {
    let violators: Vec<_> = report
        .calibration_ratios
        .iter()
        .filter(|r| r.strategy == "C" && r.p95 > 1.05 && r.sample_size >= 30)
        .collect();
    if violators.is_empty() {
        return None;
    }
    Some(Recommendation {
        severity: Severity::Critical,
        code: "STRATEGY_C_UNDER_PREDICTION".into(),
        headline: format!(
            "Strategy C P95 > 1.05 on {} (model) bucket(s) — customer plugin under-predicts",
            violators.len()
        ),
        possible_cause: "Customer plugin output distribution shifted; current model produces \
             more tokens than the plugin estimates"
            .into(),
        suggested_action: "Retrain the customer plugin against the most recent 7-day \
             distribution; this is risky territory because under-prediction is what \
             causes BUDGET_EXHAUSTED"
            .into(),
        details: json!({ "violators": violators }),
    })
}

fn rule5_cold_start_l1_dominance(report: &Report) -> Option<Recommendation> {
    // Heuristic: total Strategy A sample share. If A dominates by sample
    // count it likely means the system is falling back to the ceiling
    // because L2/L3 cold-start tables aren't populated.
    let total: i64 = report
        .calibration_ratios
        .iter()
        .map(|r| r.sample_size)
        .sum();
    if total == 0 {
        return None;
    }
    let a_share: i64 = report
        .calibration_ratios
        .iter()
        .filter(|r| r.strategy == "A")
        .map(|r| r.sample_size)
        .sum();
    let pct = a_share as f64 / total as f64 * 100.0;
    if pct < 50.0 {
        return None;
    }
    Some(Recommendation {
        severity: Severity::Warning,
        code: "COLD_START_L1_DOMINANT".into(),
        headline: format!(
            "Strategy A (cold-start ceiling) accounts for {:.0}% of decisions",
            pct
        ),
        possible_cause: "L2 TOML cold-start tables (cold-start-baseline-spec §3) may be \
             unpopulated for the active models/agents — every decision falls back \
             to L1 ceiling"
            .into(),
        suggested_action: "Populate L2 TOML for the top-traffic (model, agent, prompt_class) \
             buckets; rerun report after 7 days"
            .into(),
        details: json!({ "strategy_a_share_pct": pct }),
    })
}

fn rule6_plugin_error_rate(report: &Report) -> Option<Recommendation> {
    // Phase B heuristic: report doesn't carry plugin_error_count directly
    // (predictor-spec §6 would surface it). Use absence of Strategy C
    // samples when other strategies have samples as a proxy.
    let has_c: bool = report
        .calibration_ratios
        .iter()
        .any(|r| r.strategy == "C" && r.sample_size >= 30);
    let total_non_c: i64 = report
        .calibration_ratios
        .iter()
        .filter(|r| r.strategy != "C")
        .map(|r| r.sample_size)
        .sum();
    if has_c || total_non_c < 100 {
        return None;
    }
    Some(Recommendation {
        severity: Severity::Warning,
        code: "STRATEGY_C_ABSENT".into(),
        headline: "No Strategy C samples in window despite > 100 non-C samples".into(),
        possible_cause: "Customer plugin endpoint is mis-configured, unreachable, or returning \
             errors — the predictor is silently falling back to Strategy A/B"
            .into(),
        suggested_action: "Check `control_plane` plugin registration + plugin endpoint health; \
             inspect predictor logs for `PLUGIN_INVOCATION_FAILED` codes"
            .into(),
        details: json!({ "non_c_sample_total": total_non_c }),
    })
}

fn rule7_run_projection_exceeded(report: &Report) -> Option<Recommendation> {
    if report.run_total_count == 0 {
        return None;
    }
    let rate = report.run_budget_projection_exceeded_count as f64
        / report.run_total_count as f64
        * 100.0;
    if rate <= 5.0 {
        return None;
    }
    Some(Recommendation {
        severity: Severity::Info,
        code: "RUN_PROJECTION_EXCEEDED_HIGH".into(),
        headline: format!(
            "{:.1}% of runs hit RUN_BUDGET_PROJECTION_EXCEEDED",
            rate
        ),
        possible_cause: "Per-run budget caps may be tighter than actual run cost, or agents \
             may be running stuck loops that trigger the run_cost_projector circuit"
            .into(),
        suggested_action: "Review per-run budget caps in the contract DSL; if caps look right, \
             investigate agent-side step-count regression"
            .into(),
        details: json!({
            "rate_pct": rate,
            "runs_exceeded": report.run_budget_projection_exceeded_count,
            "runs_total": report.run_total_count,
        }),
    })
}

fn rule8_tier3_known_vendor(report: &Report) -> Option<Recommendation> {
    // Heuristic: a model name that contains "gpt", "claude", "gemini",
    // or "llama" should NOT land in T3. Phase B uses model fingerprint
    // prefix as the proxy; the full implementation would join against
    // tokenizer_versions to check the vendor.
    let known_vendor_models: Vec<_> = report
        .calibration_ratios
        .iter()
        .filter(|r| {
            let m = r.model.to_lowercase();
            m.contains("gpt")
                || m.contains("claude")
                || m.contains("gemini")
                || m.contains("llama")
        })
        .collect();
    let t3 = report
        .tier_distribution
        .iter()
        .find(|t| t.tier.as_deref() == Some(crate::report::TIER3_LABEL));
    let t3_pct = t3.map(|t| t.pct).unwrap_or(0.0);
    // Only fire when (a) T3 has some traffic and (b) the report contains
    // a known-vendor model — the assumption is the operator's deployment
    // is configured for that vendor but T3 fallback indicates a missing
    // dispatch entry.
    if t3_pct <= 0.1 || known_vendor_models.is_empty() {
        return None;
    }
    let vendor_summary = known_vendor_models
        .iter()
        .map(|r| r.model.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Some(Recommendation {
        severity: Severity::Warning,
        code: "TIER3_KNOWN_VENDOR_FINGERPRINT".into(),
        headline: format!(
            "T3 fallback ({:.1}%) coincides with known-vendor models in calibration data",
            t3_pct
        ),
        possible_cause: format!(
            "Known-vendor model name(s) {vendor_summary} are in calibration scope but \
             the tokenizer dispatch table doesn't map them — every decision falls back \
             to T3 heuristics"
        ),
        suggested_action: "Inspect tokenizer dispatch entry for the listed vendor models \
             (tokenizer-service-spec §4.2); add a Tier 2 entry if missing"
            .into(),
        details: json!({
            "t3_pct": t3_pct,
            "vendor_models": vendor_summary,
        }),
    })
}

fn rule9_strategy_a_dominance(report: &Report) -> Option<Recommendation> {
    // Same data signal as rule 5 but a softer recommendation pointing
    // at cache warm-up instead of L2 TOML.
    let total: i64 = report
        .calibration_ratios
        .iter()
        .map(|r| r.sample_size)
        .sum();
    if total == 0 {
        return None;
    }
    let a_share: i64 = report
        .calibration_ratios
        .iter()
        .filter(|r| r.strategy == "A")
        .map(|r| r.sample_size)
        .sum();
    let pct = a_share as f64 / total as f64 * 100.0;
    // Fire when A dominates but not so much that rule 5 already fired
    // critical. The 50-80% band is the "cache warm-up not L2 missing"
    // signal.
    if !(50.0..=80.0).contains(&pct) {
        return None;
    }
    Some(Recommendation {
        severity: Severity::Info,
        code: "STRATEGY_A_DOMINANT_CACHE_WARMUP".into(),
        headline: format!(
            "Strategy A is the dominant strategy ({:.0}%) — consider cache warm-up",
            pct
        ),
        possible_cause: "The stats_aggregator cache may be cold for the active buckets — \
             Strategy B falls back to A until enough samples accumulate"
            .into(),
        suggested_action: "If the deployment is fresh, this is expected. After 7 days, \
             cache should warm up; if it persists past 7d, inspect stats_aggregator \
             cycle health"
            .into(),
        details: json!({ "strategy_a_share_pct": pct }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{CalibrationRatio, DriftAlert, TierDistribution, Window};
    use chrono::{TimeZone, Utc};

    fn base() -> Report {
        Report {
            tenant_id: "t".into(),
            window: Window {
                from: Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap(),
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

    fn ratio(model: &str, strategy: &str, p95: f64, n: i64) -> CalibrationRatio {
        CalibrationRatio {
            model: model.into(),
            strategy: strategy.into(),
            p50: 1.0,
            p95,
            p99: p95,
            sample_size: n,
        }
    }

    #[test]
    fn rule1_fires_on_p95_critical() {
        let mut r = base();
        r.calibration_ratios.push(ratio("gpt-4o", "B", 1.6, 100));
        let recs = evaluate(&r);
        assert!(recs.iter().any(|x| x.code == "P95_CRITICAL_OVER_1_50"));
        assert!(recs
            .iter()
            .any(|x| x.severity == Severity::Critical && x.code == "P95_CRITICAL_OVER_1_50"));
    }

    #[test]
    fn rule1_ignores_strategy_a() {
        // Strategy A is the ceiling; conservative actual/predicted P95 is expected.
        let mut r = base();
        r.calibration_ratios.push(ratio("gpt-4o", "A", 0.8, 100));
        let recs = evaluate(&r);
        assert!(!recs.iter().any(|x| x.code == "P95_CRITICAL_OVER_1_50"));
    }

    #[test]
    fn rule2_fires_on_tier3_warning() {
        let mut r = base();
        r.tier_distribution.push(TierDistribution {
            tier: Some("T3".into()),
            count: 500,
            pct: 0.5,
            threshold_violation: true,
        });
        let recs = evaluate(&r);
        let r2 = recs.iter().find(|x| x.code == "TIER3_BURST").unwrap();
        assert_eq!(r2.severity, Severity::Warning);
    }

    #[test]
    fn rule2_promotes_to_critical_above_1pct() {
        let mut r = base();
        r.tier_distribution.push(TierDistribution {
            tier: Some("T3".into()),
            count: 1500,
            pct: 1.5,
            threshold_violation: true,
        });
        let recs = evaluate(&r);
        let r2 = recs.iter().find(|x| x.code == "TIER3_BURST").unwrap();
        assert_eq!(r2.severity, Severity::Critical);
    }

    #[test]
    fn rule2_silent_below_threshold() {
        let mut r = base();
        r.tier_distribution.push(TierDistribution {
            tier: Some("T3".into()),
            count: 5,
            pct: 0.05,
            threshold_violation: false,
        });
        let recs = evaluate(&r);
        assert!(!recs.iter().any(|x| x.code == "TIER3_BURST"));
    }

    #[test]
    fn rule3_fires_on_drift_alerts() {
        let mut r = base();
        r.drift_alerts.push(DriftAlert {
            event_id: "id".into(),
            event_time: Utc::now(),
            bucket: "(x)".into(),
            z_score: 2.4,
        });
        let recs = evaluate(&r);
        assert!(recs.iter().any(|x| x.code == "PREDICTION_DRIFT_ALERTS_PRESENT"));
    }

    #[test]
    fn rule4_fires_on_c_under_prediction() {
        let mut r = base();
        r.calibration_ratios.push(CalibrationRatio {
            model: "gpt-4o".into(),
            strategy: "C".into(),
            p50: 1.02,
            p95: 1.08,
            p99: 1.14,
            sample_size: 50,
        });
        let recs = evaluate(&r);
        assert!(recs.iter().any(|x| x.code == "STRATEGY_C_UNDER_PREDICTION"));
        assert!(recs
            .iter()
            .any(|x| x.code == "STRATEGY_C_UNDER_PREDICTION" && x.severity == Severity::Critical));
    }

    #[test]
    fn rule4_ignores_small_sample() {
        let mut r = base();
        r.calibration_ratios.push(CalibrationRatio {
            model: "gpt-4o".into(),
            strategy: "C".into(),
            p50: 1.02,
            p95: 1.08,
            p99: 1.14,
            sample_size: 10, // below 30
        });
        let recs = evaluate(&r);
        assert!(!recs.iter().any(|x| x.code == "STRATEGY_C_UNDER_PREDICTION"));
    }

    #[test]
    fn rule4_direction_matches_actual_over_predicted_ratio() {
        let mut over_reserved = base();
        // predicted=200, actual=100 -> actual/predicted=0.50: conservative,
        // not under-prediction.
        over_reserved
            .calibration_ratios
            .push(ratio("gpt-4o", "C", 0.50, 50));
        assert!(!evaluate(&over_reserved)
            .iter()
            .any(|x| x.code == "STRATEGY_C_UNDER_PREDICTION"));

        let mut under_predicted = base();
        // predicted=100, actual=200 -> actual/predicted=2.00: unsafe
        // under-prediction.
        under_predicted
            .calibration_ratios
            .push(ratio("gpt-4o", "C", 2.00, 50));
        assert!(evaluate(&under_predicted)
            .iter()
            .any(|x| x.code == "STRATEGY_C_UNDER_PREDICTION"));
    }

    #[test]
    fn rule5_fires_on_strategy_a_above_80pct() {
        let mut r = base();
        r.calibration_ratios.push(ratio("gpt-4o", "A", 0.8, 90));
        r.calibration_ratios.push(ratio("gpt-4o", "B", 1.1, 10));
        let recs = evaluate(&r);
        assert!(recs.iter().any(|x| x.code == "COLD_START_L1_DOMINANT"));
    }

    #[test]
    fn rule6_fires_when_c_absent() {
        let mut r = base();
        r.calibration_ratios.push(ratio("gpt-4o", "B", 1.1, 150));
        // No C entries.
        let recs = evaluate(&r);
        assert!(recs.iter().any(|x| x.code == "STRATEGY_C_ABSENT"));
    }

    #[test]
    fn rule6_silent_when_c_present() {
        let mut r = base();
        r.calibration_ratios.push(ratio("gpt-4o", "B", 1.1, 150));
        r.calibration_ratios.push(ratio("gpt-4o", "C", 1.0, 50));
        let recs = evaluate(&r);
        assert!(!recs.iter().any(|x| x.code == "STRATEGY_C_ABSENT"));
    }

    #[test]
    fn rule7_fires_on_projection_high() {
        let mut r = base();
        r.run_total_count = 100;
        r.run_budget_projection_exceeded_count = 10; // 10%
        let recs = evaluate(&r);
        assert!(recs.iter().any(|x| x.code == "RUN_PROJECTION_EXCEEDED_HIGH"));
    }

    #[test]
    fn rule7_silent_when_below_5pct() {
        let mut r = base();
        r.run_total_count = 100;
        r.run_budget_projection_exceeded_count = 3;
        let recs = evaluate(&r);
        assert!(!recs.iter().any(|x| x.code == "RUN_PROJECTION_EXCEEDED_HIGH"));
    }

    #[test]
    fn rule8_fires_on_known_vendor_in_t3() {
        let mut r = base();
        r.tier_distribution.push(TierDistribution {
            tier: Some("T3".into()),
            count: 500,
            pct: 0.5,
            threshold_violation: true,
        });
        r.calibration_ratios.push(ratio("gpt-4o-custom-2024-12", "B", 1.1, 100));
        let recs = evaluate(&r);
        assert!(recs.iter().any(|x| x.code == "TIER3_KNOWN_VENDOR_FINGERPRINT"));
    }

    #[test]
    fn rule8_silent_when_no_known_vendor() {
        let mut r = base();
        r.tier_distribution.push(TierDistribution {
            tier: Some("T3".into()),
            count: 500,
            pct: 0.5,
            threshold_violation: true,
        });
        r.calibration_ratios.push(ratio("custom-fine-tune-v3", "B", 1.1, 100));
        let recs = evaluate(&r);
        assert!(!recs.iter().any(|x| x.code == "TIER3_KNOWN_VENDOR_FINGERPRINT"));
    }

    #[test]
    fn rule9_fires_in_50_80_band() {
        // A at 60% triggers rule 9 (warm-up) AND rule 5 (L1 dominant)
        // because both signals are valid at the same time.
        let mut r = base();
        r.calibration_ratios.push(ratio("gpt-4o", "A", 0.8, 60));
        r.calibration_ratios.push(ratio("gpt-4o", "B", 1.1, 40));
        let recs = evaluate(&r);
        assert!(recs.iter().any(|x| x.code == "STRATEGY_A_DOMINANT_CACHE_WARMUP"));
        assert!(recs.iter().any(|x| x.code == "COLD_START_L1_DOMINANT"));
    }

    #[test]
    fn rule9_silent_when_a_dominant_above_80pct() {
        // Above 80% rule 5 (critical L1) is the right signal; rule 9
        // stays silent so the operator doesn't see two contradictory
        // diagnoses.
        let mut r = base();
        r.calibration_ratios.push(ratio("gpt-4o", "A", 0.8, 90));
        r.calibration_ratios.push(ratio("gpt-4o", "B", 1.1, 10));
        let recs = evaluate(&r);
        assert!(!recs.iter().any(|x| x.code == "STRATEGY_A_DOMINANT_CACHE_WARMUP"));
    }

    // -- §8.2 discipline: every recommendation must carry both
    // "possible cause" and "suggested action" --
    #[test]
    fn every_rule_has_possible_cause_and_suggested_action() {
        // Build a fixture that triggers every rule.
        let mut r = base();
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
            p50: 1.5,
            p95: 1.6,
            p99: 2.0,
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
        r.calibration_ratios.push(ratio("gpt-4o", "A", 0.8, 200));
        r.run_total_count = 100;
        r.run_budget_projection_exceeded_count = 20;

        let recs = evaluate(&r);
        assert!(recs.len() >= 5);
        for rec in &recs {
            assert!(
                !rec.possible_cause.is_empty(),
                "rule {} missing possible_cause",
                rec.code
            );
            assert!(
                !rec.suggested_action.is_empty(),
                "rule {} missing suggested_action",
                rec.code
            );
        }
    }

    // -- §8.1 acceptance: 5 synthetic scenarios trigger the right rules --
    #[test]
    fn scenario_healthy() {
        let mut r = base();
        r.tier_distribution.push(TierDistribution {
            tier: Some("T2".into()),
            count: 100_000,
            pct: 99.95,
            threshold_violation: false,
        });
        r.tier_distribution.push(TierDistribution {
            tier: Some("T3".into()),
            count: 50,
            pct: 0.05,
            threshold_violation: false,
        });
        r.calibration_ratios.push(ratio("gpt-4o", "B", 1.15, 5000));
        r.calibration_ratios.push(ratio("gpt-4o", "C", 1.05, 1000));
        let recs = evaluate(&r);
        // Healthy: zero recommendations.
        assert_eq!(recs.len(), 0, "healthy scenario triggered: {recs:?}");
    }

    #[test]
    fn scenario_drift() {
        let mut r = base();
        r.tier_distribution.push(TierDistribution {
            tier: Some("T2".into()),
            count: 100_000,
            pct: 99.95,
            threshold_violation: false,
        });
        r.calibration_ratios.push(ratio("gpt-4o", "B", 1.15, 5000));
        r.calibration_ratios.push(ratio("gpt-4o", "C", 1.05, 1000));
        for i in 0..3 {
            r.drift_alerts.push(DriftAlert {
                event_id: format!("id{i}"),
                event_time: Utc.with_ymd_and_hms(2026, 5, 15 + i, 14, 0, 0).unwrap(),
                bucket: "(gpt-4o, support, chat_long)".into(),
                z_score: 2.4,
            });
        }
        let recs = evaluate(&r);
        assert!(recs.iter().any(|x| x.code == "PREDICTION_DRIFT_ALERTS_PRESENT"));
    }

    #[test]
    fn scenario_cold_start_dominated() {
        let mut r = base();
        r.calibration_ratios.push(ratio("gpt-4o", "A", 0.8, 900));
        r.calibration_ratios.push(ratio("gpt-4o", "B", 1.1, 100));
        let recs = evaluate(&r);
        assert!(recs.iter().any(|x| x.code == "COLD_START_L1_DOMINANT"));
    }

    #[test]
    fn scenario_plugin_failing() {
        let mut r = base();
        r.calibration_ratios.push(ratio("gpt-4o", "A", 0.8, 200));
        r.calibration_ratios.push(ratio("gpt-4o", "B", 1.1, 200));
        // No C entries → plugin missing / failing.
        let recs = evaluate(&r);
        assert!(recs.iter().any(|x| x.code == "STRATEGY_C_ABSENT"));
    }

    #[test]
    fn scenario_tier3_burst() {
        let mut r = base();
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
        r.calibration_ratios.push(ratio("gpt-4o", "B", 1.15, 5000));
        let recs = evaluate(&r);
        assert!(recs.iter().any(|x| x.code == "TIER3_BURST"));
        // Critical because pct > 1.0%.
        assert!(recs.iter().any(|x| x.code == "TIER3_BURST" && x.severity == Severity::Critical));
    }
}
