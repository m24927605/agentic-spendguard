//! D15 COV_74 — CloudEvent 1.0 envelope builder for
//! `spendguard.audit.import.manus_credit`.
//!
//! Family pattern `spendguard.audit.import.<vendor>_<unit>` —
//! siblings:
//!
//!   * D14: `spendguard.audit.import.devin_acu`
//!   * D15: `spendguard.audit.import.manus_credit`
//!   * D16 (future): `spendguard.audit.import.genspark_credit`
//!
//! ## Locked constants (review-standards §6 + design §5 #3)
//!
//! * `type   = "spendguard.audit.import.manus_credit"`   (C1)
//! * `source = "spendguard-importer-manus"`              (S2-mirror)
//! * `data.schema_version = "v1alpha1"`                  (S3-mirror)
//! * `data.reservation_source = "subscription_meter"`    (D14/D16-aligned)
//! * `data.import_source      = "manus_team_api"`        (D14/D16 family)
//!
//! ## Determinism boundary
//!
//! `build_with` takes an explicit `Uuid` + `DateTime<Utc>` so the
//! builder is pure / golden-testable. The convenience `build` impl
//! reads `Utc::now()` for production use; the id is the same
//! deterministic UUIDv5 in both paths so dedup never depends on the
//! wall clock.

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::audit::{
    deterministic_event_id, import_record_to_audit_row, AuditRowDraft, CLOUDEVENT_TYPE, MODEL_SLUG,
};
use crate::pricing::PriceTable;
use crate::record::{ImportRecord, IngestionMode};

/// CloudEvent `type` constant. Mirrors `audit::CLOUDEVENT_TYPE`.
pub const EVENT_TYPE: &str = CLOUDEVENT_TYPE;

/// CloudEvent `source` constant. Matches the crate name.
pub const EVENT_SOURCE: &str = "spendguard-importer-manus";

/// `data.schema_version`. Bumped to `v1alpha2` only on additive evolution.
pub const SCHEMA_VERSION: &str = "v1alpha1";

/// CloudEvent 1.0 envelope.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CloudEventEnvelope {
    /// CloudEvents spec version. Always `"1.0"`.
    pub specversion: String,
    /// Event type — `spendguard.audit.import.manus_credit`.
    #[serde(rename = "type")]
    pub event_type: String,
    /// Event source — `spendguard-importer-manus`.
    pub source: String,
    /// Deterministic UUIDv5 from `(workspace, session, window_end)`.
    pub id: String,
    /// RFC 3339 timestamp.
    pub time: String,
    /// Always `"application/json"`.
    pub datacontenttype: String,
    /// `tenant/<wid>/manus/workspace/<wid>/session/<sid>`. NEVER
    /// contains the bearer token / customer email (review-standards T1
    /// / T8).
    pub subject: String,
    /// Payload shape.
    pub data: CloudEventData,
}

/// CloudEvent `data` body. Mirrors the audit-row contract field-for-field.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CloudEventData {
    /// `v1alpha1`. Bumps land additively as `v1alpha2`.
    pub schema_version: String,
    /// Workspace ID surfaced as tenant scope.
    pub tenant_id: String,
    /// Vendor workspace ID. Opaque.
    pub workspace_id: String,
    /// Vendor session ID. Opaque.
    pub session_id: String,
    /// Vendor tier slug (`"team_plan"` / `"enterprise"` / ...).
    pub tier: String,
    /// Vendor status slug (`"completed"` / `"failed"` / ...).
    pub status: String,
    /// Credits consumed (raw vendor signal).
    pub credits_consumed: i64,
    /// Per-credit cost from the price table at conversion time.
    pub credit_cost_micro_usd: i64,
    /// `credits_consumed * credit_cost_micro_usd` (saturating).
    pub amount_micro_usd: i64,
    /// Stamped from the price table at the moment of conversion.
    pub pricing_version: String,
    /// Synthetic `manus.session/credit` slug — dashboards group by it.
    pub model: String,
    /// Always 0 (review-standards E5).
    pub input_tokens: i64,
    /// As above.
    pub output_tokens: i64,
    /// Billing window start (RFC 3339).
    pub window_start: String,
    /// Billing window end (RFC 3339).
    pub window_end: String,
    /// Always `"subscription_meter"` (D14/D16-aligned).
    pub reservation_source: String,
    /// Always `"manus_team_api"` (D14/D16 family pattern).
    pub import_source: String,
    /// `"fixture"` or `"live"`.
    pub ingestion_mode: String,
    /// `Some(64 hex)` for fixture mode; `None` for live mode.
    pub fixture_provenance_sha256: Option<String>,
    /// Vendor-prefixed dedupe key `"manus:<session_id>"`.
    pub dedupe_key: String,
}

/// All ways envelope construction can fail.
#[derive(Debug, thiserror::Error)]
pub enum EnvelopeBuildError {
    /// Underlying conversion error from `import_record_to_audit_row`.
    #[error(transparent)]
    Meter(#[from] crate::error::MeterError),
}

/// Build a CloudEvent envelope from an `ImportRecord` using
/// `deterministic_event_id` + `Utc::now()`. Production-emit path.
pub fn build(
    rec: &ImportRecord,
    table: &PriceTable,
) -> Result<CloudEventEnvelope, EnvelopeBuildError> {
    let event_id = deterministic_event_id(&rec.workspace_id, &rec.session_id, rec.window_end);
    build_with(rec, table, event_id, Utc::now())
}

/// Pure variant — caller supplies `id` + `time`. Golden-tested.
pub fn build_with(
    rec: &ImportRecord,
    table: &PriceTable,
    event_id: Uuid,
    time: DateTime<Utc>,
) -> Result<CloudEventEnvelope, EnvelopeBuildError> {
    // Drive through the canonical import_record_to_audit_row so the
    // envelope and the audit row are derived from the same arithmetic.
    let draft: AuditRowDraft = import_record_to_audit_row(rec, table)?;
    let credit_cost_micro_usd = table.credit_cost(rec.tier).unwrap_or(0);

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
            tenant_id: rec.workspace_id.clone(),
            workspace_id: rec.workspace_id.clone(),
            session_id: rec.session_id.clone(),
            tier: rec.tier.as_str().to_string(),
            status: rec.status.as_str().to_string(),
            credits_consumed: rec.credits_consumed,
            credit_cost_micro_usd,
            amount_micro_usd: draft.amount_micro_usd,
            pricing_version: table.pricing_version.clone(),
            model: MODEL_SLUG.to_string(),
            input_tokens: 0,
            output_tokens: 0,
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
            dedupe_key: draft.dedupe_key,
        },
    })
}

fn subject_for(rec: &ImportRecord) -> String {
    // Review-standards T1 / T8: tenant + synthetic workspace / session only.
    format!(
        "tenant/{}/manus/workspace/{}/session/{}",
        rec.workspace_id, rec.workspace_id, rec.session_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{IngestionMode, SessionStatus, Tier};
    use chrono::TimeZone;
    use pretty_assertions::assert_eq;

    fn prices() -> PriceTable {
        PriceTable::load_embedded()
    }

    fn t(y: i32, mo: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, 0, 0).unwrap()
    }

    fn rec_team_fixture() -> ImportRecord {
        ImportRecord {
            session_id: "ses_FAKE_team_completed_001".into(),
            workspace_id: "ws_FAKE_team_001".into(),
            tier: Tier::TeamPlan,
            credits_consumed: 47,
            status: SessionStatus::Completed,
            window_start: t(2026, 6, 5, 14),
            window_end: t(2026, 6, 5, 15),
            ingestion_mode: IngestionMode::Fixture,
            fixture_provenance_sha256: Some("a".repeat(64)),
        }
    }

    #[test]
    fn constants_are_pinned() {
        assert_eq!(EVENT_TYPE, "spendguard.audit.import.manus_credit");
        assert_eq!(EVENT_SOURCE, "spendguard-importer-manus");
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
            "tenant/ws_FAKE_team_001/manus/workspace/ws_FAKE_team_001/session/ses_FAKE_team_completed_001",
        );
        assert!(!env.subject.contains("Bearer"));
        assert!(!env.subject.to_lowercase().contains("authorization"));
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
        assert_eq!(env.data.amount_micro_usd, 964_722);
        assert_eq!(env.data.credit_cost_micro_usd, 20_526);
        assert_eq!(env.data.pricing_version, "manus-credit-v1-2026-06");
    }

    #[test]
    fn data_carries_zero_amount_for_byok() {
        let mut rec = rec_team_fixture();
        rec.tier = Tier::EnterpriseByok;
        rec.credits_consumed = 1024;
        let env = build_with(&rec, &prices(), Uuid::nil(), t(2026, 6, 8, 12)).unwrap();
        assert_eq!(env.data.amount_micro_usd, 0);
        assert_eq!(env.data.credit_cost_micro_usd, 0);
        assert_eq!(env.data.tier, "enterprise_byok");
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
    fn data_dedupe_key_has_manus_prefix() {
        // X3 mirror: dedupe_key vendor-prefixed.
        let env = build_with(
            &rec_team_fixture(),
            &prices(),
            Uuid::nil(),
            t(2026, 6, 8, 12),
        )
        .unwrap();
        assert!(env.data.dedupe_key.starts_with("manus:"));
        assert_eq!(env.data.dedupe_key, "manus:ses_FAKE_team_completed_001");
    }

    #[test]
    fn data_tokens_are_zero() {
        // E5 mirror.
        let env = build_with(
            &rec_team_fixture(),
            &prices(),
            Uuid::nil(),
            t(2026, 6, 8, 12),
        )
        .unwrap();
        assert_eq!(env.data.input_tokens, 0);
        assert_eq!(env.data.output_tokens, 0);
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
        assert!(s.contains("\"type\":\"spendguard.audit.import.manus_credit\""));
        assert!(!s.contains("event_type"));
    }
}
