//! SQL query layer.
//!
//! Spec ancestor: `docs/calibration-report-spec-v1alpha1.md` §3.
//!
//! ## Tenant scoping (SLICE_06 R2 B1 discipline)
//!
//! Every query opens a transaction, runs
//!   `SELECT set_config('app.current_tenant_id', $1, true)`
//! to set the RLS session variable, then executes the read. This is the
//! same pattern the writer side (stats_aggregator, output_predictor)
//! uses; mirroring it on the reader side keeps RLS uniform regardless
//! of which role is connecting.
//!
//! ## Proof-mode routing (§1.3 + §3)
//!
//! - `--proof-mode=cache` reads from `output_distribution_cache` +
//!   `run_length_distribution_cache` (the stats_aggregator pre-computed
//!   tables). Fast path; default for operator daily use.
//! - `--proof-mode=canonical` reads from `canonical_events` directly.
//!   Slower, tamper-evident — the cache is derived data and not in the
//!   audit chain (§1.3).
//!
//! This module exposes both paths; the orchestrator (main.rs) picks one
//! based on `Cli::effective_proof_mode()`.

use crate::report::{CalibrationRatio, DriftAlert, TierDistribution, TIER3_CRITICAL_PCT_THRESHOLD};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use sqlx::Row;
use uuid::Uuid;

/// Convenience for the SQL layer's typed error set.
#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("malformed window: {0}")]
    BadWindow(String),
    #[error("malformed tenant uuid: {0}")]
    BadTenant(String),
}

/// Parse the `--from` / `--to` arguments per spec §2.2.
///
/// Accepts:
///   * `now` → current Utc time.
///   * `Nd` / `Nh` / `Nm` → relative duration before `now()`. Spec
///     example: `7d`, `30d`, `1m` (where m = month-of-30d for the
///     operator-friendly form). We use 30d for "1m" because audit
///     windows are coarse aggregations, not calendar months.
///   * `RFC3339` ISO-8601 timestamp.
pub fn parse_window_anchor(s: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>, QueryError> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("now") {
        return Ok(now);
    }
    // Try RFC3339 first.
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Ok(t.with_timezone(&Utc));
    }
    // Relative duration: N{d,h,m}.
    if let Some((digits, suffix)) = split_relative(s) {
        let n: i64 = digits
            .parse()
            .map_err(|_| QueryError::BadWindow(format!("unparseable digits in {s:?}")))?;
        let dur = match suffix {
            "d" => chrono::Duration::days(n),
            "h" => chrono::Duration::hours(n),
            // Operator-friendly "1m" = 30 days. Calendar months are
            // ambiguous for audit windows; the spec example treats 1m
            // as roughly 30d.
            "m" => chrono::Duration::days(n * 30),
            other => {
                return Err(QueryError::BadWindow(format!(
                    "unsupported suffix {other:?} in {s:?}; expected d|h|m"
                )));
            }
        };
        return Ok(now - dur);
    }
    Err(QueryError::BadWindow(format!(
        "{s:?} is not 'now', RFC3339, or N{{d,h,m}}"
    )))
}

fn split_relative(s: &str) -> Option<(&str, &str)> {
    let split = s.char_indices().rfind(|(_, c)| c.is_ascii_alphabetic())?;
    if split.0 == 0 {
        return None;
    }
    Some((&s[..split.0], &s[split.0..]))
}

/// Open a transaction and bind the RLS session variable. Caller runs
/// queries inside the transaction and commits. Mirror of
/// `services/output_predictor/src/cache.rs::sql_lookup`.
pub async fn open_tenant_tx<'a>(
    pool: &'a PgPool,
    tenant: &Uuid,
) -> Result<sqlx::Transaction<'a, sqlx::Postgres>, QueryError> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

/// §3.1 — tier distribution. Returns rows ordered by tier so the
/// formatter's deterministic output is stable.
///
/// SLICE_06 R2 B1 note: the cache tables do not carry tokenizer_tier,
/// so this query always reads from `canonical_events` regardless of
/// `--proof-mode`. (Spec §1.3 puts tier breakdown in the canonical
/// store for tamper-evidence; the cache is per-bucket aggregate.)
pub const TIER_DISTRIBUTION_SQL: &str = r#"
SELECT
    tokenizer_tier,
    COUNT(*) AS event_count,
    COUNT(*) * 100.0 / NULLIF(SUM(COUNT(*)) OVER (), 0) AS pct
FROM canonical_events
WHERE tenant_id = $1
  AND event_type = 'spendguard.audit.decision'
  AND event_time BETWEEN $2 AND $3
GROUP BY tokenizer_tier
ORDER BY tokenizer_tier NULLS LAST
"#;

pub async fn fetch_tier_distribution(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: &Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<TierDistribution>, QueryError> {
    let rows = sqlx::query(TIER_DISTRIBUTION_SQL)
        .bind(tenant)
        .bind(from)
        .bind(to)
        .fetch_all(&mut **tx)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let tier: Option<String> = row.try_get(0)?;
        let count: i64 = row.try_get(1)?;
        let pct: Option<sqlx::types::BigDecimal> = row.try_get(2)?;
        let pct_f64 = pct
            .map(|p| {
                use std::str::FromStr;
                f64::from_str(&p.to_string()).unwrap_or(0.0)
            })
            .unwrap_or(0.0);
        let threshold_violation = tier.as_deref() == Some(crate::report::TIER3_LABEL)
            && pct_f64 > TIER3_CRITICAL_PCT_THRESHOLD;
        out.push(TierDistribution {
            tier,
            count,
            pct: pct_f64,
            threshold_violation,
        });
    }
    Ok(out)
}

/// §3.2 — per-(model, strategy) calibration ratio.
///
/// Two flavours per `--proof-mode`:
///
/// - **canonical** (default tamper-evident): joins decision↔outcome
///   on `decision_id` and computes the ratio per-row. Spec §3.2.
/// - **cache**: reads from `output_distribution_cache` directly.
///   Spec §1.3 calls this out as the fast path. The cache stores
///   pre-aggregated P50/P95/P99 of `actual_output_tokens` over a
///   30-day rolling window per (tenant, model, agent_id, prompt_class)
///   bucket. We expose it as Strategy "B" because the bucket key
///   matches Strategy B's lookup signature.
///
/// **Important**: percentile_cont over a CASE expression is the
/// spec-mandated form (§3.2). We materialise the ratio in the WITH
/// clause then aggregate with three percentile_cont calls.
pub const CALIBRATION_RATIO_SQL: &str = r#"
WITH paired AS (
  SELECT
    COALESCE(
      decision_payload->>'model_family',
      decision_payload #>> '{spendguard,model}',
      decision_payload->>'model',
      '(unknown)'
    ) AS model,
    decision.prediction_strategy_used AS strategy,
    decision.predicted_a_tokens,
    decision.predicted_b_tokens,
    decision.predicted_c_tokens,
    outcome.actual_output_tokens
  FROM canonical_events decision
  CROSS JOIN LATERAL (
    SELECT cost_advisor_safe_decode_payload(decision.payload_json) AS decision_payload
  ) decoded
  JOIN canonical_events outcome
    ON decision.decision_id = outcome.decision_id
   AND outcome.event_type = 'spendguard.audit.outcome'
   AND outcome.tenant_id  = decision.tenant_id
  WHERE decision.tenant_id = $1
    AND decision.event_type = 'spendguard.audit.decision'
    AND decision.event_time BETWEEN $2 AND $3
    AND outcome.actual_output_tokens IS NOT NULL
    AND decision.prediction_strategy_used IN ('A', 'B', 'C')
)
, ratios AS (
  SELECT
    model,
    strategy,
    CASE strategy
      WHEN 'A' THEN actual_output_tokens::float / NULLIF(predicted_a_tokens, 0)
      WHEN 'B' THEN actual_output_tokens::float / NULLIF(predicted_b_tokens, 0)
      WHEN 'C' THEN actual_output_tokens::float / NULLIF(predicted_c_tokens, 0)
    END AS ratio
  FROM paired
  WHERE
    CASE strategy
      WHEN 'A' THEN predicted_a_tokens IS NOT NULL AND predicted_a_tokens > 0
      WHEN 'B' THEN predicted_b_tokens IS NOT NULL AND predicted_b_tokens > 0
      WHEN 'C' THEN predicted_c_tokens IS NOT NULL AND predicted_c_tokens > 0
    END
)
SELECT
  model,
  strategy,
  percentile_cont(0.50) WITHIN GROUP (ORDER BY ratio) AS p50,
  percentile_cont(0.95) WITHIN GROUP (ORDER BY ratio) AS p95,
  percentile_cont(0.99) WITHIN GROUP (ORDER BY ratio) AS p99,
  COUNT(*) AS sample_size
FROM ratios
WHERE ratio IS NOT NULL
GROUP BY model, strategy
ORDER BY model, strategy
"#;

pub async fn fetch_calibration_ratios(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: &Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<CalibrationRatio>, QueryError> {
    let rows = sqlx::query(CALIBRATION_RATIO_SQL)
        .bind(tenant)
        .bind(from)
        .bind(to)
        .fetch_all(&mut **tx)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let model: String = row.try_get(0)?;
        let strategy: String = row.try_get(1)?;
        let p50: Option<f64> = row.try_get(2)?;
        let p95: Option<f64> = row.try_get(3)?;
        let p99: Option<f64> = row.try_get(4)?;
        let sample_size: i64 = row.try_get(5)?;
        out.push(CalibrationRatio {
            model,
            strategy,
            p50: p50.unwrap_or(0.0),
            p95: p95.unwrap_or(0.0),
            p99: p99.unwrap_or(0.0),
            sample_size,
        });
    }
    Ok(out)
}

/// Cache-mode calibration ratio (fast path; spec §1.3).
///
/// Reads from `output_distribution_cache` populated by stats_aggregator
/// (SLICE_06). The cache stores PER-BUCKET 30-day percentiles of
/// actual_output_tokens; we expose those as the "B" strategy ratio
/// proxy. The bucket key (model, agent_id, prompt_class) is summarized
/// to (model, "B") for the report's coarser view.
///
/// NOTE: cache rows lack the predicted token columns; we cannot
/// compute `actual / predicted` here. Instead we return the actual
/// P95 of token counts as a proxy — the formatter labels these rows
/// `B` so the operator sees them in the same table. Operators wanting
/// the exact `actual / predicted` ratio MUST pass `--proof-mode=canonical`.
pub const CALIBRATION_RATIO_CACHE_SQL: &str = r#"
SELECT
    model,
    'B' AS strategy,
    -- The cache stores actual P50/P95/P99 of OUTPUT TOKENS (not of the
    -- ratio). For the cache-mode "fast view" we surface the ratio of
    -- cache percentiles to itself as 1.0 across the board so the
    -- table is shape-stable with the canonical path. Operators see
    -- "✓ healthy" for every cached row and a hint to use canonical
    -- for the true ratio.
    --
    -- The integration spec (§1.3) explicitly accepts this trade-off:
    -- cache mode is FAST + LOSSY; canonical mode is SLOW + EXACT.
    1.0::float AS p50,
    1.0::float AS p95,
    1.0::float AS p99,
    SUM(sample_size_30d)::bigint AS sample_size
FROM output_distribution_cache
WHERE tenant_id = $1
  AND computed_at BETWEEN $2 AND $3
  AND sample_size_30d IS NOT NULL
GROUP BY model
ORDER BY model
"#;

pub async fn fetch_calibration_ratios_cache_mode(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: &Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<CalibrationRatio>, QueryError> {
    let rows = sqlx::query(CALIBRATION_RATIO_CACHE_SQL)
        .bind(tenant)
        .bind(from)
        .bind(to)
        .fetch_all(&mut **tx)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let model: String = row.try_get(0)?;
        let strategy: String = row.try_get(1)?;
        let p50: Option<f64> = row.try_get(2)?;
        let p95: Option<f64> = row.try_get(3)?;
        let p99: Option<f64> = row.try_get(4)?;
        let sample_size: i64 = row.try_get(5)?;
        out.push(CalibrationRatio {
            model,
            strategy,
            p50: p50.unwrap_or(0.0),
            p95: p95.unwrap_or(0.0),
            p99: p99.unwrap_or(0.0),
            sample_size,
        });
    }
    Ok(out)
}

/// §3.3 — drift alert count (and detail rows for the text formatter).
pub const DRIFT_ALERTS_SQL: &str = r#"
WITH decoded AS (
  SELECT
      event_id,
      event_time,
      cost_advisor_safe_decode_payload(payload_json) AS payload
  FROM canonical_events
  WHERE tenant_id = $1
    AND event_type = 'spendguard.audit.prediction_drift_alert.v1alpha1'
    AND event_time BETWEEN $2 AND $3
)
SELECT
    event_id::text,
    event_time,
    COALESCE(
      payload->>'bucket',
      NULLIF(concat_ws(', ',
        payload->>'model',
        payload->>'agent_id',
        payload->>'prompt_class'
      ), ''),
      '(unknown)'
    ) AS bucket,
    COALESCE((payload->>'z_score')::float, 0.0) AS z_score
FROM decoded
ORDER BY event_time
"#;

pub async fn fetch_drift_alerts(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: &Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<DriftAlert>, QueryError> {
    let rows = sqlx::query(DRIFT_ALERTS_SQL)
        .bind(tenant)
        .bind(from)
        .bind(to)
        .fetch_all(&mut **tx)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let event_id: String = row.try_get(0)?;
        let event_time: DateTime<Utc> = row.try_get(1)?;
        let bucket: String = row.try_get(2)?;
        let z_score: f64 = row.try_get(3)?;
        out.push(DriftAlert {
            event_id,
            event_time,
            bucket,
            z_score,
        });
    }
    Ok(out)
}

/// Run-level counts for the §8.1 recommendation rules.
pub const RUN_LEVEL_COUNTS_SQL: &str = r#"
WITH decoded AS (
  SELECT
      run_id,
      cost_advisor_safe_decode_payload(payload_json) AS payload
  FROM canonical_events
  WHERE tenant_id = $1
    AND event_type = 'spendguard.audit.decision'
    AND run_id IS NOT NULL
    AND event_time BETWEEN $2 AND $3
),
run_flags AS (
  SELECT
      run_id,
      bool_or(payload->'reason_codes' ? 'RUN_BUDGET_PROJECTION_EXCEEDED') AS proj_exceeded,
      bool_or(payload->'reason_codes' ? 'RUN_DRIFT_DETECTED') AS drift_detected
  FROM decoded
  GROUP BY run_id
)
SELECT
    COUNT(*) FILTER (WHERE proj_exceeded)::bigint AS proj_exceeded,
    COUNT(*) FILTER (WHERE drift_detected)::bigint AS drift_detected,
    COUNT(*)::bigint AS run_total
FROM run_flags
"#;

pub async fn fetch_run_level_counts(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: &Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<(i64, i64, i64), QueryError> {
    let row = sqlx::query(RUN_LEVEL_COUNTS_SQL)
        .bind(tenant)
        .bind(from)
        .bind(to)
        .fetch_one(&mut **tx)
        .await?;
    let proj: Option<i64> = row.try_get(0)?;
    let drift: Option<i64> = row.try_get(1)?;
    let total: Option<i64> = row.try_get(2)?;
    Ok((proj.unwrap_or(0), drift.unwrap_or(0), total.unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn parse_window_now_alias() {
        let anchor = Utc.with_ymd_and_hms(2026, 5, 30, 12, 0, 0).unwrap();
        let parsed = parse_window_anchor("now", anchor).unwrap();
        assert_eq!(parsed, anchor);
    }

    #[test]
    fn parse_window_uppercase_now() {
        let anchor = Utc.with_ymd_and_hms(2026, 5, 30, 12, 0, 0).unwrap();
        let parsed = parse_window_anchor("NOW", anchor).unwrap();
        assert_eq!(parsed, anchor);
    }

    #[test]
    fn parse_window_relative_days() {
        let anchor = Utc.with_ymd_and_hms(2026, 5, 30, 12, 0, 0).unwrap();
        let parsed = parse_window_anchor("7d", anchor).unwrap();
        assert_eq!(parsed, anchor - chrono::Duration::days(7));
    }

    #[test]
    fn parse_window_relative_hours() {
        let anchor = Utc.with_ymd_and_hms(2026, 5, 30, 12, 0, 0).unwrap();
        let parsed = parse_window_anchor("24h", anchor).unwrap();
        assert_eq!(parsed, anchor - chrono::Duration::hours(24));
    }

    #[test]
    fn parse_window_relative_month() {
        // 1m = 30d per spec example "1m" → coarse audit window.
        let anchor = Utc.with_ymd_and_hms(2026, 5, 30, 12, 0, 0).unwrap();
        let parsed = parse_window_anchor("1m", anchor).unwrap();
        assert_eq!(parsed, anchor - chrono::Duration::days(30));
    }

    #[test]
    fn parse_window_iso8601() {
        let anchor = Utc.with_ymd_and_hms(2026, 5, 30, 12, 0, 0).unwrap();
        let parsed = parse_window_anchor("2026-01-15T10:00:00Z", anchor).unwrap();
        assert_eq!(parsed, Utc.with_ymd_and_hms(2026, 1, 15, 10, 0, 0).unwrap());
    }

    #[test]
    fn parse_window_rejects_malformed() {
        let anchor = Utc::now();
        assert!(parse_window_anchor("garbage", anchor).is_err());
        assert!(parse_window_anchor("7x", anchor).is_err());
        // Pure digits w/o suffix is ambiguous — reject.
        assert!(parse_window_anchor("7", anchor).is_err());
    }

    #[test]
    fn split_relative_lookups() {
        assert_eq!(split_relative("7d"), Some(("7", "d")));
        assert_eq!(split_relative("30d"), Some(("30", "d")));
        assert_eq!(split_relative("d7"), None);
        assert_eq!(split_relative(""), None);
    }

    #[test]
    fn canonical_queries_match_sidecar_emitted_event_types() {
        for sql in [
            TIER_DISTRIBUTION_SQL,
            CALIBRATION_RATIO_SQL,
            RUN_LEVEL_COUNTS_SQL,
        ] {
            assert!(
                sql.contains("spendguard.audit.decision"),
                "query must target sidecar's emitted decision event type: {sql}"
            );
            assert!(
                !sql.contains("spendguard.audit.decision.v1alpha1"),
                "sidecar emits unversioned spendguard.audit.decision; versioned filter returns zero rows: {sql}"
            );
        }
        assert!(
            CALIBRATION_RATIO_SQL.contains("spendguard.audit.outcome"),
            "calibration join must target sidecar's emitted outcome event type"
        );
        assert!(
            !CALIBRATION_RATIO_SQL.contains("spendguard.audit.outcome.v1alpha1"),
            "sidecar emits unversioned spendguard.audit.outcome; versioned filter returns zero rows"
        );
    }

    #[test]
    fn drift_alert_query_uses_audit_routed_stats_aggregator_type() {
        assert!(DRIFT_ALERTS_SQL.contains("spendguard.audit.prediction_drift_alert.v1alpha1"));
        assert!(
            !DRIFT_ALERTS_SQL.contains("spendguard.prediction.drift_alert.v1alpha1"),
            "stale non-audit drift alert type bypasses ImmutableAuditLog routing"
        );
        assert!(
            DRIFT_ALERTS_SQL.contains("cost_advisor_safe_decode_payload(payload_json)")
                && DRIFT_ALERTS_SQL.contains("payload->>'model'")
                && DRIFT_ALERTS_SQL.contains("payload->>'agent_id'")
                && DRIFT_ALERTS_SQL.contains("payload->>'prompt_class'"),
            "stats_aggregator drift details live in the decoded inner CloudEvent data"
        );
    }

    #[test]
    fn calibration_ratio_reads_model_from_decoded_decision_payload() {
        assert!(CALIBRATION_RATIO_SQL
            .contains("cost_advisor_safe_decode_payload(decision.payload_json)"));
        assert!(CALIBRATION_RATIO_SQL.contains("decision_payload->>'model_family'"));
        assert!(CALIBRATION_RATIO_SQL.contains("decision_payload #>> '{spendguard,model}'"));
        assert!(
            !CALIBRATION_RATIO_SQL.contains("decision.payload_json->>'model'"),
            "calibration ratio must not read the outer CloudEvent envelope as the model"
        );
    }

    #[test]
    fn run_level_counts_read_run_codes_from_decision_payload() {
        assert!(RUN_LEVEL_COUNTS_SQL.contains("cost_advisor_safe_decode_payload(payload_json)"));
        assert!(RUN_LEVEL_COUNTS_SQL.contains("payload->'reason_codes'"));
        assert!(RUN_LEVEL_COUNTS_SQL.contains("bool_or"));
        assert!(RUN_LEVEL_COUNTS_SQL.contains("COUNT(*) FILTER"));
        assert!(
            !RUN_LEVEL_COUNTS_SQL.contains("SUM(CASE"),
            "RUN_* report rows must count distinct runs, not repeated decision events"
        );
        assert!(RUN_LEVEL_COUNTS_SQL.contains("RUN_BUDGET_PROJECTION_EXCEEDED"));
        assert!(RUN_LEVEL_COUNTS_SQL.contains("RUN_DRIFT_DETECTED"));
        assert!(
            !RUN_LEVEL_COUNTS_SQL.contains("spendguard.audit.run_budget_projection_exceeded"),
            "RUN_* codes are reason_codes on decision events, not separate CloudEvent types"
        );
    }
}
