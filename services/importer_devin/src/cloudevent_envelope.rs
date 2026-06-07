//! D14 COV_70 — CloudEvent 1.0 envelope builder for
//! `spendguard.audit.import.devin_acu`.
//!
//! The schema is committed at the sibling doc
//! [`cloudevent-schema.md`](../../../docs/specs/coverage/D14_devin_importer/cloudevent-schema.md).
//! Any drift between this struct and that doc is a review Blocker
//! (review-standards S4 / S5).
//!
//! ## Locked constants (review-standards §2)
//!
//! * `type   = "spendguard.audit.import.devin_acu"`  (S1)
//! * `source = "spendguard-importer-devin"`          (S2)
//! * `data.schema_version = "v1alpha1"`              (S3)
//! * `data.reservation_source = "subscription_meter"` (S6)
//! * `data.import_source      = "devin_team_api"`     (S7)
//!
//! ## Determinism boundary
//!
//! `CloudEventBuilder::build_with` takes a `Uuid` (event_id) and a
//! `DateTime<Utc>` (time) as explicit parameters so the builder is
//! pure / golden-testable. The convenience `build` impl reads
//! `Uuid::now_v7` + `Utc::now` for production use.

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::acu_price_table::AcuPriceTable;
use crate::import_record::{deterministic_event_id, AuditRowDraft, ImportRecord, IngestionMode};

/// CloudEvent `type` constant.
pub const EVENT_TYPE: &str = "spendguard.audit.import.devin_acu";

/// CloudEvent `source` constant. Matches the crate name.
pub const EVENT_SOURCE: &str = "spendguard-importer-devin";

/// `data.schema_version`. Bumped to `v1alpha2` only on additive evolution.
pub const SCHEMA_VERSION: &str = "v1alpha1";

/// CloudEvent 1.0 envelope.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CloudEventEnvelope {
    /// CloudEvents spec version. Always `"1.0"`.
    pub specversion: String,
    /// Event type — `spendguard.audit.import.devin_acu`.
    #[serde(rename = "type")]
    pub event_type: String,
    /// Event source — `spendguard-importer-devin`.
    pub source: String,
    /// Deterministic UUIDv5 from `(team, session, window_end)`.
    pub id: String,
    /// RFC 3339 timestamp.
    pub time: String,
    /// Always `"application/json"`.
    pub datacontenttype: String,
    /// `tenant/<tid>/devin/team/<dt>/session/<ds>`. Never contains the
    /// bearer token / customer email (review-standards T7).
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
    /// Devin team identifier.
    pub devin_team_id: String,
    /// Devin session identifier.
    pub devin_session_id: String,
    /// Raw ACU value from the Devin Team API.
    pub acu_consumed: f64,
    /// `Some(rate)` for public plans; `None` for enterprise.
    pub usd_per_acu: Option<f64>,
    /// `Some(micro_usd)` for public plans; `None` for enterprise
    /// negotiated rate (in which case `reason_code = "devin_enterprise_negotiated_rate"`).
    pub amount_micro_usd: Option<i64>,
    /// Stamped from the price table at the moment of conversion.
    pub pricing_version: String,
    /// Billing-window start (RFC 3339).
    pub window_start: String,
    /// Billing-window end (RFC 3339).
    pub window_end: String,
    /// Always `"subscription_meter"`.
    pub reservation_source: String,
    /// Always `"devin_team_api"`.
    pub import_source: String,
    /// `"fixture"` or `"live"`.
    pub ingestion_mode: String,
    /// `Some(64 hex)` when ingestion_mode is fixture; `None` when live.
    pub fixture_provenance_sha256: Option<String>,
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
    prices: &AcuPriceTable,
) -> Result<CloudEventEnvelope, EnvelopeBuildError> {
    // Deterministic id; the time stamp still uses now() because the
    // CloudEvent `time` is "when the envelope was constructed", not
    // "when the underlying event occurred" — that's window_end and is
    // already in data.
    let event_id =
        deterministic_event_id(&rec.devin_team_id, &rec.devin_session_id, rec.window_end);
    build_with(rec, prices, event_id, Utc::now())
}

/// Pure variant — caller supplies the `id` + `time`. Golden-tested.
pub fn build_with(
    rec: &ImportRecord,
    prices: &AcuPriceTable,
    event_id: Uuid,
    time: DateTime<Utc>,
) -> Result<CloudEventEnvelope, EnvelopeBuildError> {
    // Drive through the canonical import_record_to_audit_row so the
    // envelope and the audit row are derived from the same arithmetic.
    let draft: AuditRowDraft = crate::import_record::import_record_to_audit_row(rec, prices)?;
    let rate = prices
        .lookup(&rec.plan)
        .map_err(crate::import_record::ImportRecordError::Lookup)?;

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
            devin_team_id: rec.devin_team_id.clone(),
            devin_session_id: rec.devin_session_id.clone(),
            acu_consumed: rec.acu_consumed,
            usd_per_acu: rate.usd_per_acu,
            amount_micro_usd: draft.amount_micro_usd,
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
        },
    })
}

fn subject_for(rec: &ImportRecord) -> String {
    // Review-standards T7: tenant + synthetic team / session only.
    format!(
        "tenant/{}/devin/team/{}/session/{}",
        rec.tenant_id, rec.devin_team_id, rec.devin_session_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use pretty_assertions::assert_eq;

    fn prices() -> AcuPriceTable {
        AcuPriceTable::load_from_embedded()
    }

    fn t(y: i32, mo: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, 0, 0).unwrap()
    }

    fn rec_team_fixture() -> ImportRecord {
        ImportRecord {
            tenant_id: "demo".into(),
            budget_id: "devin-budget".into(),
            devin_team_id: "TEAM_FIXTURE_001".into(),
            devin_session_id: "SESSION_FIXTURE_001".into(),
            acu_consumed: 12.5,
            plan: "team".into(),
            window_start: t(2026, 6, 1, 0),
            window_end: t(2026, 6, 1, 1),
            ingestion_mode: IngestionMode::Fixture,
            fixture_provenance_sha256: Some("a".repeat(64)),
        }
    }

    #[test]
    fn constants_are_pinned() {
        assert_eq!(EVENT_TYPE, "spendguard.audit.import.devin_acu");
        assert_eq!(EVENT_SOURCE, "spendguard-importer-devin");
        assert_eq!(SCHEMA_VERSION, "v1alpha1");
    }

    #[test]
    fn build_with_is_pure_same_args_same_envelope() {
        let p = prices();
        let rec = rec_team_fixture();
        let id = Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, b"deterministic-test-id");
        let now = t(2026, 6, 8, 12);
        let a = build_with(&rec, &p, id, now).unwrap();
        let b = build_with(&rec, &p, id, now).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn subject_format_synthetic_only() {
        let env = build_with(
            &rec_team_fixture(),
            &prices(),
            Uuid::nil(),
            t(2026, 6, 8, 12),
        )
        .unwrap();
        assert_eq!(
            env.subject,
            "tenant/demo/devin/team/TEAM_FIXTURE_001/session/SESSION_FIXTURE_001",
        );
        assert!(!env.subject.contains("Bearer"));
    }

    #[test]
    fn data_carries_amount_for_team_plan() {
        let env = build_with(
            &rec_team_fixture(),
            &prices(),
            Uuid::nil(),
            t(2026, 6, 8, 12),
        )
        .unwrap();
        assert_eq!(env.data.amount_micro_usd, Some(28_125_000));
        assert_eq!(env.data.usd_per_acu, Some(2.25));
        assert_eq!(env.data.pricing_version, "devin-acu-v1-2026-06");
    }

    #[test]
    fn data_nulls_amount_for_enterprise_plan() {
        let mut rec = rec_team_fixture();
        rec.plan = "enterprise".into();
        let env = build_with(&rec, &prices(), Uuid::nil(), t(2026, 6, 8, 12)).unwrap();
        assert_eq!(env.data.amount_micro_usd, None);
        assert_eq!(env.data.usd_per_acu, None);
    }

    #[test]
    fn data_provenance_mode_invariant_fixture() {
        let env = build_with(
            &rec_team_fixture(),
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
        let mut rec = rec_team_fixture();
        rec.ingestion_mode = IngestionMode::Live;
        rec.fixture_provenance_sha256 = None;
        let env = build_with(&rec, &prices(), Uuid::nil(), t(2026, 6, 8, 12)).unwrap();
        assert_eq!(env.data.ingestion_mode, "live");
        assert_eq!(env.data.fixture_provenance_sha256, None);
    }

    #[test]
    fn serializes_to_json_with_type_renamed() {
        let env = build_with(
            &rec_team_fixture(),
            &prices(),
            Uuid::nil(),
            t(2026, 6, 8, 12),
        )
        .unwrap();
        let s = serde_json::to_string(&env).unwrap();
        // serde rename = "type" → key is "type", not "event_type".
        assert!(s.contains("\"type\":\"spendguard.audit.import.devin_acu\""));
        // No accidental Rust-side fields leaked.
        assert!(!s.contains("event_type"));
    }
}
