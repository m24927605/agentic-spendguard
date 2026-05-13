//! P1 runtime: rule registry + evaluation + cost_findings UPSERT.
//!
//! Orchestrates rule execution for a (tenant, date) bucket:
//!   1. Build registry from compiled-in rules (registers only those
//!      whose `is_ready()` returns true so placeholders never run).
//!   2. For each ready rule, execute its SQL against the ledger DB.
//!   3. Decode result into [`FindingEvidence`] proto + JSONB shape.
//!   4. UPSERT via `cost_findings_upsert()` SP on the canonical DB
//!      (spec §11.5 A1 idempotency).
//!
//! P1 ships exactly one rule (`idle_reservation_rate_v1`). The
//! registry pattern lets P1.5 add the other 3 rules without touching
//! this file.

use anyhow::{anyhow, Context, Result};
use bigdecimal::BigDecimal;
use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::{postgres::PgRow, PgPool, Row};
use uuid::Uuid;

use crate::fingerprint;
use crate::proto::cost_advisor::v1::{
    FindingCategory, FindingEvidence, FindingScope, Metric, MetricUnit, PiiClassification,
    ScopeType, Severity, WasteConfidence, WasteEstimate, WasteMethod,
};
use crate::rule::CostRule;
use crate::rules::idle_reservation_rate;
use crate::sql_rule::SqlCostRule;

/// Single emitted finding row returned to a CLI / future REST caller.
#[derive(Debug, Serialize)]
pub struct EmittedFinding {
    pub outcome: String,
    pub finding_id: String,
    pub rule_id: String,
    pub severity: String,
    pub confidence: f64,
    pub estimated_waste_micros_usd: i64,
    pub evidence: serde_json::Value,
    pub proposed_dsl_patch: Option<serde_json::Value>,
}

/// Build the P1 rule registry. Returns only rules whose
/// `is_ready()` returns true so the runtime never invokes a
/// placeholder.
pub fn build_registry() -> Vec<SqlCostRule> {
    [idle_reservation_rate::descriptor()]
        .into_iter()
        .filter(|r| r.is_ready())
        .collect()
}

/// Single-tenant evaluation entry point. Runs every registered rule
/// against the (tenant, date) bucket, UPSERTs findings, optionally
/// emits proposed DSL patches.
///
/// `propose_patches=true` returns the RFC-6902 patch in the result
/// row so a CLI caller can show it; it does NOT write an
/// `approval_requests` row in P1 (gated on owner-ack #53/#54).
pub async fn evaluate_tenant_day(
    ledger: &PgPool,
    canonical: &PgPool,
    tenant_id: Uuid,
    bucket_date: NaiveDate,
    propose_patches: bool,
) -> Result<Vec<EmittedFinding>> {
    let mut emitted = Vec::new();

    for rule in build_registry() {
        let Some(finding) = run_rule(ledger, &rule, tenant_id, bucket_date).await? else {
            continue;
        };

        let outcome = upsert_finding(canonical, tenant_id, &finding).await?;

        let proposed_patch = if propose_patches {
            Some(build_proposed_patch_for_rule(rule.rule_id(), &finding))
        } else {
            None
        };

        emitted.push(EmittedFinding {
            outcome: outcome.outcome,
            finding_id: outcome.finding_id.to_string(),
            rule_id: finding.proto.rule_id.clone(),
            severity: severity_str(finding.proto.severity()).to_string(),
            confidence: finding.confidence,
            estimated_waste_micros_usd: finding
                .proto
                .waste_estimate
                .as_ref()
                .map(|w| w.micros_usd)
                .unwrap_or(0),
            evidence: finding.proto_json.clone(),
            proposed_dsl_patch: proposed_patch,
        });
    }

    Ok(emitted)
}

struct DecodedFinding {
    proto: FindingEvidence,
    proto_json: serde_json::Value,
    confidence: f64,
    finding_id: Uuid,
    detected_at: DateTime<Utc>,
    sample_decision_ids: Vec<Uuid>,
}

async fn run_rule(
    ledger: &PgPool,
    rule: &SqlCostRule,
    tenant_id: Uuid,
    bucket_date: NaiveDate,
) -> Result<Option<DecodedFinding>> {
    let row_opt: Option<PgRow> = sqlx::query(rule.sql())
        .bind(tenant_id)
        .bind(bucket_date)
        .fetch_optional(ledger)
        .await
        .with_context(|| format!("execute rule {}", rule.rule_id()))?;

    let Some(row) = row_opt else {
        return Ok(None);
    };

    match rule.rule_id() {
        "idle_reservation_rate_v1" => decode_idle_reservation_rate(rule, row, bucket_date).map(Some),
        other => Err(anyhow!("no decoder registered for rule {}", other)),
    }
}

fn decode_idle_reservation_rate(
    rule: &SqlCostRule,
    row: PgRow,
    bucket_date: NaiveDate,
) -> Result<DecodedFinding> {
    let total: i64 = row.try_get("total_reservations")?;
    let ttl_expired: i64 = row.try_get("ttl_expired_count")?;
    let median_ttl: i32 = row.try_get("median_ttl_seconds")?;
    let p95_ttl: i32 = row.try_get("p95_ttl_seconds")?;
    let sample_ids: Vec<Uuid> = row
        .try_get::<Option<Vec<Uuid>>, _>("sample_reservation_ids")?
        .unwrap_or_default();
    let waste_micros: i64 = row.try_get("estimated_waste_micros_usd")?;

    let idle_ratio = if total > 0 {
        ttl_expired as f64 / total as f64
    } else {
        0.0
    };
    let time_bucket = bucket_date.format("%Y-%m-%d").to_string();

    let scope = FindingScope {
        scope_type: ScopeType::TenantGlobal as i32,
        agent_id: String::new(),
        run_id: String::new(),
        tool_name: String::new(),
        model_family: String::new(),
    };
    let fingerprint_hex = fingerprint::compute(rule.rule_id(), &scope, &time_bucket);

    let metrics = vec![
        metric("total_reservations", total as f64, MetricUnit::Count, "reservations_with_ttl_status_v1.reservation_id"),
        metric("ttl_expired_count", ttl_expired as f64, MetricUnit::Count, "reservations_with_ttl_status_v1.derived_state"),
        Metric {
            name: "idle_ratio".into(),
            value: idle_ratio,
            unit: MetricUnit::Ratio as i32,
            source_field: "derived: ttl_expired_count / total_reservations".into(),
            pii_classification: PiiClassification::None as i32,
            derivation: "ttl_expired_count / total_reservations".into(),
            ci95_low: None,
            ci95_high: None,
        },
        metric("median_ttl_seconds", median_ttl as f64, MetricUnit::Seconds, "derived: PERCENTILE_CONT(0.5) of ttl_seconds"),
        metric("p95_ttl_seconds", p95_ttl as f64, MetricUnit::Seconds, "derived: PERCENTILE_CONT(0.95) of ttl_seconds"),
    ];

    let waste_estimate = Some(WasteEstimate {
        micros_usd: waste_micros,
        method: WasteMethod::BaselineExcess as i32,
        confidence: WasteConfidence::Medium as i32,
        explanation: format!(
            "{} of {} reservations TTL'd (idle ratio {:.0}%); median TTL {}s — contract reservation TTL is held longer than the workload's typical commit latency.",
            ttl_expired, total, idle_ratio * 100.0, median_ttl
        ),
    });

    let severity = Severity::Warn;
    let finding_id = Uuid::now_v7();
    let detected_at = Utc::now();
    let decision_refs: Vec<String> = sample_ids.iter().map(|u| u.to_string()).collect();

    let proto = FindingEvidence {
        rule_id: rule.rule_id().into(),
        rule_version: rule.rule_version(),
        fingerprint: fingerprint_hex,
        category: FindingCategory::DetectedWaste as i32,
        scope: Some(scope),
        metrics,
        decision_refs,
        waste_estimate,
        severity: severity as i32,
        time_bucket,
        co_observed_rules: Vec::new(),
    };

    let proto_json = serde_json::json!({
        "rule_id": proto.rule_id,
        "rule_version": proto.rule_version,
        "fingerprint": proto.fingerprint,
        "category": "DETECTED_WASTE",
        "scope": { "scope_type": "TENANT_GLOBAL" },
        "metrics": proto.metrics.iter().map(|m| serde_json::json!({
            "name": m.name,
            "value": m.value,
            "unit": metric_unit_str(m.unit),
            "source_field": m.source_field,
            "pii_classification": "none",
            "derivation": m.derivation,
        })).collect::<Vec<_>>(),
        "decision_refs": proto.decision_refs,
        "waste_estimate": proto.waste_estimate.as_ref().map(|w| serde_json::json!({
            "micros_usd": w.micros_usd,
            "method": "baseline_excess",
            "confidence": "medium",
            "explanation": w.explanation,
        })),
        "severity": severity_str(severity),
        "time_bucket": proto.time_bucket,
    });

    Ok(DecodedFinding {
        proto,
        proto_json,
        confidence: 0.75,
        finding_id,
        detected_at,
        sample_decision_ids: sample_ids,
    })
}

fn metric(name: &str, value: f64, unit: MetricUnit, source_field: &str) -> Metric {
    Metric {
        name: name.into(),
        value,
        unit: unit as i32,
        source_field: source_field.into(),
        pii_classification: PiiClassification::None as i32,
        derivation: String::new(),
        ci95_low: None,
        ci95_high: None,
    }
}

struct UpsertOutcome {
    outcome: String,
    finding_id: Uuid,
}

async fn upsert_finding(
    canonical: &PgPool,
    tenant_id: Uuid,
    finding: &DecodedFinding,
) -> Result<UpsertOutcome> {
    let confidence_bd: BigDecimal = finding
        .confidence
        .to_string()
        .parse()
        .context("convert confidence to BigDecimal")?;

    let row: PgRow = sqlx::query(
        r#"
        SELECT outcome, finding_id, finding_detected_at
          FROM cost_findings_upsert(
            $1::uuid, $2::char(64), $3::uuid, $4::timestamptz,
            $5::text, $6::int, $7::text, $8::text, $9::numeric,
            $10::text, $11::text, $12::text,
            $13::jsonb, $14::bigint, $15::uuid[]
          )
        "#,
    )
    .bind(finding.finding_id)
    .bind(&finding.proto.fingerprint)
    .bind(tenant_id)
    .bind(finding.detected_at)
    .bind(&finding.proto.rule_id)
    .bind(finding.proto.rule_version as i32)
    .bind("detected_waste")
    .bind(severity_str(finding.proto.severity()))
    .bind(confidence_bd)
    .bind::<Option<String>>(None)
    .bind::<Option<String>>(None)
    .bind::<Option<String>>(None)
    .bind(&finding.proto_json)
    .bind(
        finding
            .proto
            .waste_estimate
            .as_ref()
            .map(|w| w.micros_usd)
            .unwrap_or(0),
    )
    .bind(&finding.sample_decision_ids)
    .fetch_one(canonical)
    .await
    .context("cost_findings_upsert SP call")?;

    Ok(UpsertOutcome {
        outcome: row.try_get("outcome")?,
        finding_id: row.try_get("finding_id")?,
    })
}

fn build_proposed_patch_for_rule(rule_id: &str, finding: &DecodedFinding) -> serde_json::Value {
    match rule_id {
        "idle_reservation_rate_v1" => {
            // Tighten the contract's reservation TTL to 1.5× the
            // observed median (per the rule SQL header comment).
            let median = finding
                .proto
                .metrics
                .iter()
                .find(|m| m.name == "median_ttl_seconds")
                .map(|m| m.value)
                .unwrap_or(60.0);
            let proposed_ttl = (median * 1.5).round() as i64;
            serde_json::json!([{
                "op": "replace",
                "path": "/budgets/0/rules/reservation_ttl_seconds",
                "value": proposed_ttl
            }])
        }
        _ => serde_json::Value::Null,
    }
}

// ---- enum → string helpers --------------------------------------

fn metric_unit_str(unit: i32) -> &'static str {
    match MetricUnit::try_from(unit).unwrap_or(MetricUnit::Unspecified) {
        MetricUnit::Unspecified => "METRIC_UNIT_UNSPECIFIED",
        MetricUnit::MicrosUsd => "micros_usd",
        MetricUnit::Usd => "usd",
        MetricUnit::Tokens => "tokens",
        MetricUnit::Calls => "calls",
        MetricUnit::Seconds => "seconds",
        MetricUnit::Ratio => "ratio",
        MetricUnit::Count => "count",
        MetricUnit::Percent => "percent",
        MetricUnit::Multiplier => "multiplier",
    }
}

fn severity_str(s: Severity) -> &'static str {
    match s {
        Severity::Critical => "critical",
        Severity::Warn => "warn",
        Severity::Info => "info",
        Severity::Unspecified => "warn",
    }
}
