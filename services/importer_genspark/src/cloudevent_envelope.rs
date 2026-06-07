//! D16 COV_86 — CloudEvent 1.0 envelope builder for
//! `spendguard.audit.import.genspark_credit`.
//!
//! Sibling pattern to D14's `spendguard.audit.import.devin_acu`. The
//! envelope structure is identical; only the per-vendor fields differ
//! (`credits_consumed` / `usd_per_credit` / `workspace_id` / `task_id`
//! instead of ACU + team + session).
//!
//! ## Locked constants (review-standards X3)
//!
//! * `type   = "spendguard.audit.import.genspark_credit"`
//! * `source = "spendguard-importer-genspark"`
//! * `data.schema_version = "v1alpha1"`
//! * `data.reservation_source = "subscription_meter"`
//! * `data.import_source      = "genspark_team_api"`
//!
//! ## Determinism boundary
//!
//! `build_with` takes a `Uuid` (event_id) and a `DateTime<Utc>` (time)
//! as explicit parameters so the builder is pure / golden-testable.
//! The convenience `build` impl reads `deterministic_event_id` +
//! `Utc::now` for production use.

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::credit_price_table::CreditPriceTable;
use crate::import_record::{deterministic_event_id, AuditRowDraft, ImportRecord, IngestionMode};

/// CloudEvent `type` constant.
pub const EVENT_TYPE: &str = "spendguard.audit.import.genspark_credit";

/// CloudEvent `source` constant. Matches the crate name.
pub const EVENT_SOURCE: &str = "spendguard-importer-genspark";

/// `data.schema_version`. Bumped to `v1alpha2` only on additive evolution.
pub const SCHEMA_VERSION: &str = "v1alpha1";

/// CloudEvent 1.0 envelope.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CloudEventEnvelope {
    /// CloudEvents spec version. Always `"1.0"`.
    pub specversion: String,
    /// Event type — `spendguard.audit.import.genspark_credit`.
    #[serde(rename = "type")]
    pub event_type: String,
    /// Event source — `spendguard-importer-genspark`.
    pub source: String,
    /// Deterministic UUIDv5 from `(workspace_id, task_id, window_end)`.
    pub id: String,
    /// RFC 3339 timestamp.
    pub time: String,
    /// Always `"application/json"`.
    pub datacontenttype: String,
    /// `tenant/<tid>/genspark/workspace/<ws>/task/<task>`. Never contains
    /// the bearer token / customer email (review-standards T1 / T9).
    pub subject: String,
    /// Payload shape.
    pub data: CloudEventData,
}

/// CloudEvent `data` body. Mirrors the schema doc field-for-field.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CloudEventData {
    /// `v1alpha1`. Bumps land additively as `v1alpha2`.
    pub schema_version: String,
    /// SpendGuard tenant.
    pub tenant_id: String,
    /// SpendGuard budget.
    pub budget_id: String,
    /// Genspark workspace identifier.
    pub workspace_id: String,
    /// Genspark task identifier.
    pub task_id: String,
    /// Genspark plan slug (`"plus"` / `"pro"` / `"premium"`, or any
    /// slug the vendor reports). Carried through so downstream
    /// consumers (dashboard, demo verifier) don't have to re-derive
    /// it from `model`.
    pub plan: String,
    /// Raw credit value from the Genspark admin API.
    pub credits_consumed: f64,
    /// `Some(rate)` for known plans; `None` for unknown plans.
    pub usd_per_credit: Option<f64>,
    /// Micro-USD value. `0` when the plan slug is unknown (paired with
    /// `reason_code = "genspark_plan_unknown"`).
    pub amount_micro_usd: i64,
    /// `Some("genspark_plan_unknown")` when the plan slug was absent
    /// from the price table; `None` otherwise.
    pub reason_code: Option<String>,
    /// Stamped from the price table at the moment of conversion.
    pub pricing_version: String,
    /// Billing-window start (RFC 3339).
    pub window_start: String,
    /// Billing-window end (RFC 3339).
    pub window_end: String,
    /// Always `"subscription_meter"`.
    pub reservation_source: String,
    /// Always `"genspark_team_api"`.
    pub import_source: String,
    /// `"fixture"` or `"live"`.
    pub ingestion_mode: String,
    /// `Some(64 hex)` when ingestion_mode is fixture; `None` when live.
    pub fixture_provenance_sha256: Option<String>,
    /// Pass-through task category (e.g. `"research"`).
    pub task_category: Option<String>,
}

/// All ways envelope construction can fail.
#[derive(Debug, thiserror::Error)]
pub enum EnvelopeBuildError {
    /// Underlying ImportRecord conversion error.
    #[error(transparent)]
    Record(#[from] crate::import_record::ImportRecordError),
}

/// Build a CloudEvent envelope from an `ImportRecord` using
/// `Uuid::now_v7()`-based id + `Utc::now()` time. Use for production
/// emission. Tests use [`build_with`] for determinism.
pub fn build(
    rec: &ImportRecord,
    prices: &CreditPriceTable,
) -> Result<CloudEventEnvelope, EnvelopeBuildError> {
    // Deterministic id; the time stamp still uses now() because the
    // CloudEvent `time` is "when the envelope was constructed", not
    // "when the underlying event occurred" — that's window_end and is
    // already in data.
    let event_id = deterministic_event_id(&rec.workspace_id, &rec.task_id, rec.window_end);
    build_with(rec, prices, event_id, Utc::now())
}

/// Pure variant — caller supplies the `id` + `time`. Golden-tested.
pub fn build_with(
    rec: &ImportRecord,
    prices: &CreditPriceTable,
    event_id: Uuid,
    time: DateTime<Utc>,
) -> Result<CloudEventEnvelope, EnvelopeBuildError> {
    // Drive through the canonical import_record_to_audit_row so the
    // envelope and the audit row are derived from the same arithmetic.
    let draft: AuditRowDraft = crate::import_record::import_record_to_audit_row(rec, prices)?;
    // Look up rate for usd_per_credit; unknown plan → None.
    let usd_per_credit = prices.lookup(&rec.plan).ok().map(|r| r.usd_per_credit);

    Ok(CloudEventEnvelope {
        specversion: "1.0".to_string(),
        event_type: EVENT_TYPE.to_string(),
        source: EVENT_SOURCE.to_string(),
        id: event_id.to_string(),
        time: time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        datacontenttype: "application/json".to_string(),
        subject: subject_for(rec),
        data: CloudEventData {
            schema_version: SCHEMA_VERSION.to_string(),
            tenant_id: rec.tenant_id.clone(),
            budget_id: rec.budget_id.clone(),
            workspace_id: rec.workspace_id.clone(),
            task_id: rec.task_id.clone(),
            plan: rec.plan.clone(),
            credits_consumed: rec.credits_consumed,
            usd_per_credit,
            amount_micro_usd: draft.amount_micro_usd,
            reason_code: draft.reason_code.map(|s| s.to_string()),
            pricing_version: prices.pricing_version.clone(),
            window_start: rec
                .window_start
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            window_end: rec
                .window_end
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            reservation_source: draft.reservation_source.to_string(),
            import_source: draft.import_source.to_string(),
            ingestion_mode: rec.ingestion_mode.as_str().to_string(),
            fixture_provenance_sha256: match rec.ingestion_mode {
                IngestionMode::Fixture => rec.fixture_provenance_sha256.clone(),
                IngestionMode::Live => None,
            },
            task_category: rec.task_category.clone(),
        },
    })
}

fn subject_for(rec: &ImportRecord) -> String {
    // Review-standards T1 / T9: tenant + synthetic workspace / task only.
    format!(
        "tenant/{}/genspark/workspace/{}/task/{}",
        rec.tenant_id, rec.workspace_id, rec.task_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use pretty_assertions::assert_eq;

    fn prices() -> CreditPriceTable {
        CreditPriceTable::load_from_embedded()
    }

    fn t(y: i32, mo: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, 0, 0).unwrap()
    }

    fn rec_plus_fixture() -> ImportRecord {
        ImportRecord {
            tenant_id: "demo".into(),
            budget_id: "genspark-budget".into(),
            workspace_id: "FAKE_ws_001".into(),
            task_id: "FAKE_task_001".into(),
            credits_consumed: 3200.0,
            plan: "plus".into(),
            task_category: Some("research".into()),
            window_start: t(2026, 6, 1, 0),
            window_end: t(2026, 6, 1, 1),
            ingestion_mode: IngestionMode::Fixture,
            fixture_provenance_sha256: Some("a".repeat(64)),
        }
    }

    #[test]
    fn constants_are_pinned() {
        assert_eq!(EVENT_TYPE, "spendguard.audit.import.genspark_credit");
        assert_eq!(EVENT_SOURCE, "spendguard-importer-genspark");
        assert_eq!(SCHEMA_VERSION, "v1alpha1");
    }

    #[test]
    fn build_with_is_pure_same_args_same_envelope() {
        let p = prices();
        let rec = rec_plus_fixture();
        let id = Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, b"deterministic-test-id");
        let now = t(2026, 6, 8, 12);
        let a = build_with(&rec, &p, id, now).unwrap();
        let b = build_with(&rec, &p, id, now).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn subject_format_synthetic_only() {
        let env = build_with(
            &rec_plus_fixture(),
            &prices(),
            Uuid::nil(),
            t(2026, 6, 8, 12),
        )
        .unwrap();
        assert_eq!(
            env.subject,
            "tenant/demo/genspark/workspace/FAKE_ws_001/task/FAKE_task_001",
        );
        assert!(!env.subject.contains("Bearer"));
        assert!(!env.subject.contains("FAKE_GENSPARK_TOKEN"));
    }

    #[test]
    fn data_carries_amount_for_plus_plan() {
        let env = build_with(
            &rec_plus_fixture(),
            &prices(),
            Uuid::nil(),
            t(2026, 6, 8, 12),
        )
        .unwrap();
        // 3200 × 0.001999 × 1e6 = 6_396_800
        assert_eq!(env.data.amount_micro_usd, 6_396_800);
        assert!((env.data.usd_per_credit.unwrap() - 0.001999).abs() < 1e-12);
        assert_eq!(env.data.pricing_version, "genspark-credit-v1-2026-06");
        assert_eq!(env.data.reason_code, None);
    }

    #[test]
    fn data_unknown_plan_zeros_amount_and_sets_reason() {
        let mut rec = rec_plus_fixture();
        rec.plan = "enterprise".into();
        let env = build_with(&rec, &prices(), Uuid::nil(), t(2026, 6, 8, 12)).unwrap();
        assert_eq!(env.data.amount_micro_usd, 0);
        assert_eq!(env.data.usd_per_credit, None);
        assert_eq!(
            env.data.reason_code.as_deref(),
            Some("genspark_plan_unknown"),
        );
    }

    #[test]
    fn data_provenance_mode_invariant_fixture() {
        let env = build_with(
            &rec_plus_fixture(),
            &prices(),
            Uuid::nil(),
            t(2026, 6, 8, 12),
        )
        .unwrap();
        assert_eq!(env.data.ingestion_mode, "fixture");
        assert!(env.data.fixture_provenance_sha256.is_some());
    }

    #[test]
    fn data_provenance_mode_invariant_live() {
        let mut rec = rec_plus_fixture();
        rec.ingestion_mode = IngestionMode::Live;
        rec.fixture_provenance_sha256 = None;
        let env = build_with(&rec, &prices(), Uuid::nil(), t(2026, 6, 8, 12)).unwrap();
        assert_eq!(env.data.ingestion_mode, "live");
        assert_eq!(env.data.fixture_provenance_sha256, None);
    }

    #[test]
    fn data_task_category_propagates() {
        let env = build_with(
            &rec_plus_fixture(),
            &prices(),
            Uuid::nil(),
            t(2026, 6, 8, 12),
        )
        .unwrap();
        assert_eq!(env.data.task_category.as_deref(), Some("research"));
    }

    #[test]
    fn data_task_category_absent_serializes_as_null() {
        let mut rec = rec_plus_fixture();
        rec.task_category = None;
        let env = build_with(&rec, &prices(), Uuid::nil(), t(2026, 6, 8, 12)).unwrap();
        assert_eq!(env.data.task_category, None);
        let s = serde_json::to_string(&env).unwrap();
        assert!(s.contains("\"task_category\":null"));
    }

    #[test]
    fn serializes_to_json_with_type_renamed() {
        let env = build_with(
            &rec_plus_fixture(),
            &prices(),
            Uuid::nil(),
            t(2026, 6, 8, 12),
        )
        .unwrap();
        let s = serde_json::to_string(&env).unwrap();
        // serde rename = "type" → key is "type", not "event_type".
        assert!(s.contains("\"type\":\"spendguard.audit.import.genspark_credit\""));
        assert!(!s.contains("event_type"));
    }

    #[test]
    fn reservation_source_is_subscription_meter() {
        let env = build_with(
            &rec_plus_fixture(),
            &prices(),
            Uuid::nil(),
            t(2026, 6, 8, 12),
        )
        .unwrap();
        assert_eq!(env.data.reservation_source, "subscription_meter");
    }

    #[test]
    fn import_source_is_genspark_team_api() {
        let env = build_with(
            &rec_plus_fixture(),
            &prices(),
            Uuid::nil(),
            t(2026, 6, 8, 12),
        )
        .unwrap();
        assert_eq!(env.data.import_source, "genspark_team_api");
    }

    #[test]
    fn premium_plan_uses_premium_per_credit_rate() {
        let mut rec = rec_plus_fixture();
        rec.plan = "premium".into();
        rec.credits_consumed = 50_000.0;
        let env = build_with(&rec, &prices(), Uuid::nil(), t(2026, 6, 8, 12)).unwrap();
        // 50_000 × 0.00199992 × 1e6 ≈ 99_996_000
        assert!(env.data.amount_micro_usd > 99_000_000);
        assert!(env.data.amount_micro_usd < 101_000_000);
    }
}
