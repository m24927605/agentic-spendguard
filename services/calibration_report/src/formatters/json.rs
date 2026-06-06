//! JSON formatter — structured output for SIEM / data warehouse
//! consumption.
//!
//! Spec ancestor: `docs/calibration-report-spec-v1alpha1.md` §4.2.
//!
//! ## Schema
//!
//! The shape is a strict 1:1 of the `Report` struct (see `report.rs`).
//! It deliberately deviates from a `serde_json::to_string_pretty(report)`
//! direct dump on two points:
//!
//!   1. Tier distribution is keyed-by-tier (`T1`/`T2`/`T3`) so a SIEM
//!      query can pluck `tier_distribution.T3.pct` directly. The raw
//!      `Vec<TierDistribution>` is order-dependent.
//!   2. The `include_recommendations` flag controls whether the
//!      `recommendations` field appears (SIEM consumers usually want
//!      raw signal; operators piping to file want the heuristics).
//!
//! This matches the spec §4.2 example which shows `tier_distribution`
//! as an object keyed by tier label.
//!
//! ## Stability commitment (spec §0.3 GA prereq, not v1alpha1)
//!
//! SLICE_13 ships this as a v1alpha1 shape; GA prereq from spec §0.3
//! #2 (stable JSON schema for SIEM) is OUT-OF-SCOPE per the slice
//! constraint. We add a `schema_version` field so downstream consumers
//! can detect future renames.

use crate::formatters::FormatOptions;
use crate::report::Report;
use serde::Serialize;
use serde_json::json;

/// Stable JSON schema version. Bump on breaking change.
pub const JSON_SCHEMA_VERSION: &str = "v1alpha1";

pub fn render(report: &Report, opts: &FormatOptions) -> String {
    let tier_obj = report
        .tier_distribution
        .iter()
        .map(|t| {
            let key = t.tier.clone().unwrap_or_else(|| "unspecified".to_string());
            (
                key,
                json!({
                    "pct": t.pct,
                    "count": t.count,
                    "threshold_violation": t.threshold_violation,
                }),
            )
        })
        .collect::<serde_json::Map<_, _>>();

    let mut payload = json!({
        "schema_version": JSON_SCHEMA_VERSION,
        "tenant_id": report.tenant_id,
        "window": {
            "from": report.window.from.to_rfc3339(),
            "to": report.window.to.to_rfc3339(),
        },
        "proof_mode": report.proof_mode,
        "tier_distribution": tier_obj,
        "calibration_ratios": report.calibration_ratios,
        "drift_alerts": report.drift_alerts,
        "run_summary": {
            "run_budget_projection_exceeded_count": report.run_budget_projection_exceeded_count,
            "run_drift_detected_count": report.run_drift_detected_count,
            "run_total_count": report.run_total_count,
        },
        "verify_chain_run": report.verify_chain_run,
        "verify_chain_failure": report.verify_chain_failure,
        "exit_code": report.exit_code() as u8,
    });

    if opts.include_recommendations {
        payload
            .as_object_mut()
            .expect("JSON payload is an object")
            .insert(
                "recommendations".to_string(),
                serde_json::to_value(&report.recommendations).expect("Recommendation serializes"),
            );
    }

    serde_json::to_string_pretty(&payload).expect("payload serializes")
}

/// Helper exported for tests / downstream — parses a rendered JSON
/// blob back to a `serde_json::Value` for assertion.
pub fn parse_for_assertion(s: &str) -> serde_json::Value {
    serde_json::from_str(s).expect("rendered JSON is valid")
}

#[allow(dead_code)]
#[derive(Serialize)]
struct DebugMarker;

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
            calibration_ratios: vec![CalibrationRatio {
                model: "gpt-4o".into(),
                strategy: "B".into(),
                p50: 1.04,
                p95: 1.18,
                p99: 1.34,
                sample_size: 50_000,
            }],
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
                headline: "T3 hit rate exceeds 0.1%".into(),
                possible_cause: "Unknown model fingerprints".into(),
                suggested_action: "Add to dispatch table".into(),
                details: json!({ "tier3_pct": 1.5 }),
            }],
            verify_chain_run: false,
            verify_chain_failure: None,
        }
    }

    fn opts(include_recs: bool) -> FormatOptions {
        FormatOptions {
            include_recommendations: include_recs,
            verify_chain_run: false,
        }
    }

    #[test]
    fn renders_schema_version_field() {
        // Spec §4.2 + this module's GA-prereq note: stable JSON consumers
        // detect future renames via schema_version.
        let r = fixture();
        let s = render(&r, &opts(true));
        let v = parse_for_assertion(&s);
        assert_eq!(v["schema_version"], "v1alpha1");
    }

    #[test]
    fn tier_distribution_keyed_by_tier_label() {
        // Spec §4.2 example shows `tier_distribution.T3` — so the
        // formatter MUST emit an object, not a list.
        let r = fixture();
        let s = render(&r, &opts(true));
        let v = parse_for_assertion(&s);
        assert_eq!(v["tier_distribution"]["T2"]["pct"], 98.5);
        assert_eq!(v["tier_distribution"]["T3"]["pct"], 1.5);
        assert_eq!(v["tier_distribution"]["T3"]["threshold_violation"], true);
        assert_eq!(v["tier_distribution"]["T3"]["count"], 15_000);
    }

    #[test]
    fn calibration_ratios_is_list_of_typed_rows() {
        let r = fixture();
        let s = render(&r, &opts(true));
        let v = parse_for_assertion(&s);
        let arr = v["calibration_ratios"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["model"], "gpt-4o");
        assert_eq!(arr[0]["strategy"], "B");
        assert_eq!(arr[0]["p95"], 1.18);
    }

    #[test]
    fn recommendations_omitted_when_disabled() {
        // Spec §2.2: JSON default for include_recommendations is false.
        // The formatter MUST omit the field entirely (not emit null /
        // empty list) when disabled, so downstream SIEM queries fail
        // closed if they expect the field.
        let r = fixture();
        let s = render(&r, &opts(false));
        let v = parse_for_assertion(&s);
        assert!(v.get("recommendations").is_none());
    }

    #[test]
    fn recommendations_present_when_enabled() {
        let r = fixture();
        let s = render(&r, &opts(true));
        let v = parse_for_assertion(&s);
        assert!(v["recommendations"].is_array());
        assert_eq!(v["recommendations"][0]["code"], "TIER3_BURST");
        assert_eq!(v["recommendations"][0]["severity"], "warning");
    }

    #[test]
    fn drift_alerts_rendered_with_iso_timestamps() {
        let r = fixture();
        let s = render(&r, &opts(true));
        let v = parse_for_assertion(&s);
        let arr = v["drift_alerts"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        // RFC3339 timestamp must be present (chrono serde default).
        assert!(arr[0]["event_time"]
            .as_str()
            .unwrap()
            .contains("2026-05-15"));
        assert_eq!(arr[0]["z_score"], 2.4);
    }

    #[test]
    fn run_summary_present() {
        let r = fixture();
        let s = render(&r, &opts(true));
        let v = parse_for_assertion(&s);
        assert_eq!(v["run_summary"]["run_budget_projection_exceeded_count"], 12);
        assert_eq!(v["run_summary"]["run_total_count"], 240);
    }

    #[test]
    fn exit_code_field_present_for_ci_consumption() {
        // CI / monitoring uses JSON output for batch ingestion; the
        // exit code being inside the JSON blob lets a Splunk/Datadog
        // dashboard surface the per-tenant health without parsing the
        // process exit code.
        let r = fixture();
        let s = render(&r, &opts(true));
        let v = parse_for_assertion(&s);
        // The fixture has T3 threshold_violation -> exit code 1.
        assert_eq!(v["exit_code"], 1);
    }

    #[test]
    fn verify_chain_failure_serialized_as_object() {
        let mut r = fixture();
        r.verify_chain_failure = Some(crate::report::VerifyChainFailure {
            event_id: "deadbeef".into(),
            reason: "signature mismatch".into(),
        });
        let s = render(&r, &opts(false));
        let v = parse_for_assertion(&s);
        assert_eq!(v["verify_chain_failure"]["event_id"], "deadbeef");
        assert_eq!(v["verify_chain_failure"]["reason"], "signature mismatch");
        // Exit code reflects the failure.
        assert_eq!(v["exit_code"], 3);
    }
}
