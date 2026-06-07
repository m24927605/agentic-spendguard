//! D14 COV_69 — `ImportRecord` shape + pure
//! `import_record_to_audit_row`.
//!
//! Mirrors the D13 importer contract
//! (`services/ledger/src/subscription_importer/stub.rs`) but with
//! Devin-specific fields (ACU consumed, plan tier, ingestion mode,
//! fixture provenance). The D13 `AuditRowDraft` shape is *not* reused
//! verbatim because the Devin row carries the ACU rate provenance —
//! the D13 stub focuses on `(input_tokens, output_tokens,
//! amount_atomic)` (token-billed providers). D14 ships its own
//! `AuditRowDraft` co-located with the importer; canonical_ingest
//! INSERT path treats both identically (same `import_source` column,
//! same `reservation_source` column).
//!
//! ## Purity (review-standards T8)
//!
//! `import_record_to_audit_row` is **pure**: no I/O, no global state,
//! no clock read. Same `(ImportRecord, AcuPriceTable)` ↦ same
//! `AuditRowDraft`. Trivially fuzz / property testable.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::acu_price_table::{acu_to_micro_usd, AcuPriceTable, ConversionError, PriceLookupError};

/// Sentinel string for `audit_outbox.reservation_source`.
///
/// Locked in D14 design §6 decision #2: every Devin importer row is
/// `subscription_meter`. NEVER `byok`. D13 §4.3 fork: skip the BYOK
/// ledger.
pub const RESERVATION_SOURCE_SUBSCRIPTION_METER: &str = "subscription_meter";

/// Sentinel string for `audit_outbox.import_source`.
///
/// Locked in D14 design §6 decision #8. Matches the value that mig
/// 0059 adds to the CHECK constraint.
pub const IMPORT_SOURCE_DEVIN_TEAM_API: &str = "devin_team_api";

/// `reason_code` stamped on enterprise-plan rows where
/// `amount_micro_usd = NULL` so dashboards distinguish "unknown rate"
/// from "zero spend".
pub const REASON_CODE_ENTERPRISE_NEGOTIATED: &str = "devin_enterprise_negotiated_rate";

/// UUIDv5 namespace for deterministic Devin event IDs.
///
/// Chosen at random and frozen here. Review-standards T12 requires
/// `event.id` to be deterministic so re-running the same window does
/// not double-emit (canonical_ingest dedups via existing
/// `event_replay_dedup`). Bumping this UUID would invalidate all
/// previously-emitted Devin event IDs — DO NOT EDIT.
pub const DEVIN_EVENT_NAMESPACE: Uuid = Uuid::from_bytes([
    0xde, 0x71, 0xa3, 0xc4, 0x1a, 0x73, 0x4b, 0x14, 0x91, 0xfe, 0xd0, 0x14, 0xde, 0x71, 0xac, 0xa0,
]);

/// Whether the ImportRecord was produced from a committed fixture
/// (default merge gate) or from the live Devin Team API (gated on
/// `live` Cargo feature + `DEVIN_API_TOKEN`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestionMode {
    /// Fixture replay — default merge gate. Carries
    /// `fixture_provenance_sha256` so audit rows are auditable.
    Fixture,
    /// Live Team API pull. `fixture_provenance_sha256` is `None`.
    Live,
}

impl IngestionMode {
    /// Wire string used in CloudEvent `data.ingestion_mode`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fixture => "fixture",
            Self::Live => "live",
        }
    }
}

/// One row of Devin Team API usage. The shape mirrors the Devin
/// `/teams/{id}/usage` response (design §4.1) plus the
/// SpendGuard-side bookkeeping fields.
///
/// All amounts are computed downstream; the import record carries the
/// raw vendor signal (`acu_consumed`, `plan`).
#[derive(Debug, Clone, PartialEq)]
pub struct ImportRecord {
    /// SpendGuard tenant ID (caller-supplied — the importer is a
    /// single-tenant job at a time).
    pub tenant_id: String,
    /// SpendGuard budget ID. Optional in some flows but D14 always
    /// requires one — Devin rows land on a budget the operator pre-
    /// provisioned.
    pub budget_id: String,
    /// Devin Team API team identifier. Opaque.
    pub devin_team_id: String,
    /// Devin session identifier. Opaque. Same `(team_id, session_id,
    /// window_end)` triple ↦ idempotent event.id.
    pub devin_session_id: String,
    /// Raw ACU value from the Devin Team API.
    pub acu_consumed: f64,
    /// Devin plan slug (`"team"` / `"enterprise"`). Looked up against
    /// the price table at conversion time.
    pub plan: String,
    /// Start of the billing window this record covers.
    pub window_start: DateTime<Utc>,
    /// End of the billing window. Used as `occurred_at` on the audit
    /// row so dashboards line up with the vendor's billing cutoff.
    pub window_end: DateTime<Utc>,
    /// Fixture vs live.
    pub ingestion_mode: IngestionMode,
    /// SHA-256 of the source fixture file. Required when
    /// `ingestion_mode == Fixture`; `None` when `Live`.
    pub fixture_provenance_sha256: Option<String>,
}

/// SpendGuard `audit_outbox` row draft built from an `ImportRecord`.
///
/// This is the canonical_ingest `AppendEvents` handoff — the actual
/// INSERT lives in canonical_ingest. Field names mirror the schema
/// (mig 0056/0058/0059) so reviewers can grep across stack.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditRowDraft {
    /// Deterministic UUIDv5 from `(team_id, session_id, window_end)`.
    /// Same record produces the same `event_id` so canonical_ingest's
    /// `event_replay_dedup` skips re-emission.
    pub event_id: Uuid,
    /// Tenant scope.
    pub tenant_id: String,
    /// Budget scope.
    pub budget_id: String,
    /// Always `"subscription_meter"`.
    pub reservation_source: &'static str,
    /// Always `"devin_team_api"`.
    pub import_source: &'static str,
    /// Pseudo-model slug stamped on the row so dashboards group by
    /// vendor: `"devin/acu/<plan>"`.
    pub model: String,
    /// Raw ACU value carried through to the audit row for
    /// observability / re-derivation.
    pub acu_consumed: f64,
    /// `Some(micro_usd)` for public plans; `None` for enterprise.
    pub amount_micro_usd: Option<i64>,
    /// Always `Some` for D14 rows — the price table's
    /// `pricing_version` at conversion time.
    pub pricing_version: Option<String>,
    /// `Some("devin_enterprise_negotiated_rate")` when
    /// `amount_micro_usd == None`. None otherwise.
    pub reason_code: Option<&'static str>,
    /// Window end — anchors dashboard timeline.
    pub occurred_at: DateTime<Utc>,
    /// Fixture / live.
    pub ingestion_mode: IngestionMode,
    /// Fixture SHA-256 provenance (None in live mode).
    pub fixture_provenance_sha256: Option<String>,
}

/// All ways `import_record_to_audit_row` can fail.
#[derive(Debug, thiserror::Error)]
pub enum ImportRecordError {
    /// The plan was not in the price table.
    #[error(transparent)]
    Lookup(#[from] PriceLookupError),
    /// The ACU value / rate failed validation.
    #[error(transparent)]
    Conversion(#[from] ConversionError),
    /// Required identifiers were empty.
    #[error("required field empty: {0}")]
    MissingField(&'static str),
    /// `ingestion_mode = Fixture` requires a SHA-256 provenance string;
    /// `Live` requires `None`. Mismatch.
    #[error("ingestion mode {mode:?} requires fixture_provenance_sha256 to be {expected}")]
    ProvenanceMismatch {
        /// The mode the record advertised.
        mode: IngestionMode,
        /// Human-readable expectation.
        expected: &'static str,
    },
}

/// Compute a deterministic `event_id` from the idempotency triple
/// `(devin_team_id, devin_session_id, window_end)`. Same record ↦ same
/// UUIDv5 ↦ canonical_ingest skips re-emission (D14 design §6 #10).
pub fn deterministic_event_id(
    devin_team_id: &str,
    devin_session_id: &str,
    window_end: DateTime<Utc>,
) -> Uuid {
    // RFC 3339 with the canonical Z suffix so the same timestamp
    // hashes identically across runtimes / serializers.
    let name = format!(
        "{}|{}|{}",
        devin_team_id,
        devin_session_id,
        window_end.to_rfc3339(),
    );
    Uuid::new_v5(&DEVIN_EVENT_NAMESPACE, name.as_bytes())
}

/// Pure conversion `ImportRecord -> AuditRowDraft`.
///
/// * No I/O.
/// * No global state.
/// * No clock read (`window_end` is supplied by the caller).
/// * No `unsafe`.
/// * Same `(ImportRecord, AcuPriceTable)` ↦ same `AuditRowDraft`.
///
/// Returns `Err(ImportRecordError)` on missing fields, an unknown
/// plan, a negative / non-finite ACU value, or a fixture/live
/// provenance mismatch.
pub fn import_record_to_audit_row(
    rec: &ImportRecord,
    prices: &AcuPriceTable,
) -> Result<AuditRowDraft, ImportRecordError> {
    // ── Required-field validation ────────────────────────────────────
    if rec.tenant_id.is_empty() {
        return Err(ImportRecordError::MissingField("tenant_id"));
    }
    if rec.budget_id.is_empty() {
        return Err(ImportRecordError::MissingField("budget_id"));
    }
    if rec.devin_team_id.is_empty() {
        return Err(ImportRecordError::MissingField("devin_team_id"));
    }
    if rec.devin_session_id.is_empty() {
        return Err(ImportRecordError::MissingField("devin_session_id"));
    }
    if rec.plan.is_empty() {
        return Err(ImportRecordError::MissingField("plan"));
    }

    // ── Provenance mode invariant (review-standards S9) ──────────────
    match (rec.ingestion_mode, rec.fixture_provenance_sha256.as_deref()) {
        (IngestionMode::Fixture, None) => {
            return Err(ImportRecordError::ProvenanceMismatch {
                mode: IngestionMode::Fixture,
                expected: "Some(<64 hex chars>)",
            });
        }
        (IngestionMode::Live, Some(_)) => {
            return Err(ImportRecordError::ProvenanceMismatch {
                mode: IngestionMode::Live,
                expected: "None",
            });
        }
        _ => {}
    }

    // ── Price lookup + conversion ────────────────────────────────────
    let rate = prices.lookup(&rec.plan)?;
    let amount = acu_to_micro_usd(rec.acu_consumed, rate.usd_per_acu)?;

    let reason_code = if amount.is_none() {
        Some(REASON_CODE_ENTERPRISE_NEGOTIATED)
    } else {
        None
    };

    let event_id =
        deterministic_event_id(&rec.devin_team_id, &rec.devin_session_id, rec.window_end);

    Ok(AuditRowDraft {
        event_id,
        tenant_id: rec.tenant_id.clone(),
        budget_id: rec.budget_id.clone(),
        reservation_source: RESERVATION_SOURCE_SUBSCRIPTION_METER,
        import_source: IMPORT_SOURCE_DEVIN_TEAM_API,
        model: format!("devin/acu/{}", rec.plan),
        acu_consumed: rec.acu_consumed,
        amount_micro_usd: amount,
        pricing_version: Some(prices.pricing_version.clone()),
        reason_code,
        occurred_at: rec.window_end,
        ingestion_mode: rec.ingestion_mode,
        fixture_provenance_sha256: rec.fixture_provenance_sha256.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use pretty_assertions::assert_eq;

    fn embedded_prices() -> AcuPriceTable {
        AcuPriceTable::load_from_embedded()
    }

    fn t(y: i32, mo: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, 0, 0).unwrap()
    }

    fn rec_team() -> ImportRecord {
        ImportRecord {
            tenant_id: "tenant-alpha".into(),
            budget_id: "devin-budget".into(),
            devin_team_id: "TEAM_FIXTURE_001".into(),
            devin_session_id: "SESSION_FIXTURE_001".into(),
            acu_consumed: 12.5,
            plan: "team".into(),
            window_start: t(2026, 6, 1, 0),
            window_end: t(2026, 6, 1, 1),
            ingestion_mode: IngestionMode::Fixture,
            fixture_provenance_sha256: Some("0".repeat(64)),
        }
    }

    fn rec_enterprise() -> ImportRecord {
        ImportRecord {
            tenant_id: "tenant-alpha".into(),
            budget_id: "devin-budget".into(),
            devin_team_id: "TEAM_FIXTURE_002".into(),
            devin_session_id: "SESSION_FIXTURE_002".into(),
            acu_consumed: 100.0,
            plan: "enterprise".into(),
            window_start: t(2026, 6, 1, 0),
            window_end: t(2026, 6, 1, 1),
            ingestion_mode: IngestionMode::Fixture,
            fixture_provenance_sha256: Some("0".repeat(64)),
        }
    }

    // ── Headline gate A10.3: 12.5 ACU × $2.25 → 28,125,000 micro-USD ─
    #[test]
    fn import_record_to_audit_row_amount_conversion() {
        let row = import_record_to_audit_row(&rec_team(), &embedded_prices()).unwrap();
        assert_eq!(row.amount_micro_usd, Some(28_125_000));
        assert_eq!(row.reason_code, None);
    }

    #[test]
    fn import_record_to_audit_row_sets_subscription_meter() {
        let row = import_record_to_audit_row(&rec_team(), &embedded_prices()).unwrap();
        assert_eq!(row.reservation_source, "subscription_meter");
    }

    #[test]
    fn import_record_to_audit_row_sets_import_source_devin_team_api() {
        let row = import_record_to_audit_row(&rec_team(), &embedded_prices()).unwrap();
        assert_eq!(row.import_source, "devin_team_api");
    }

    #[test]
    fn import_record_to_audit_row_enterprise_plan_nulls_amount() {
        let row = import_record_to_audit_row(&rec_enterprise(), &embedded_prices()).unwrap();
        assert_eq!(row.amount_micro_usd, None);
        assert_eq!(row.reason_code, Some("devin_enterprise_negotiated_rate"));
    }

    #[test]
    fn import_record_to_audit_row_stamps_pricing_version() {
        let row = import_record_to_audit_row(&rec_team(), &embedded_prices()).unwrap();
        assert_eq!(row.pricing_version.as_deref(), Some("devin-acu-v1-2026-06"),);
    }

    #[test]
    fn import_record_to_audit_row_model_slug_includes_plan() {
        let row = import_record_to_audit_row(&rec_team(), &embedded_prices()).unwrap();
        assert_eq!(row.model, "devin/acu/team");

        let row = import_record_to_audit_row(&rec_enterprise(), &embedded_prices()).unwrap();
        assert_eq!(row.model, "devin/acu/enterprise");
    }

    #[test]
    fn import_record_to_audit_row_occurred_at_is_window_end() {
        let row = import_record_to_audit_row(&rec_team(), &embedded_prices()).unwrap();
        assert_eq!(row.occurred_at, t(2026, 6, 1, 1));
    }

    #[test]
    fn deterministic_event_id_is_stable_across_runs() {
        let id_a =
            deterministic_event_id("TEAM_FIXTURE_001", "SESSION_FIXTURE_001", t(2026, 6, 1, 1));
        let id_b =
            deterministic_event_id("TEAM_FIXTURE_001", "SESSION_FIXTURE_001", t(2026, 6, 1, 1));
        assert_eq!(id_a, id_b);
    }

    #[test]
    fn deterministic_event_id_differs_on_window_change() {
        let id_a =
            deterministic_event_id("TEAM_FIXTURE_001", "SESSION_FIXTURE_001", t(2026, 6, 1, 1));
        let id_b =
            deterministic_event_id("TEAM_FIXTURE_001", "SESSION_FIXTURE_001", t(2026, 6, 1, 2));
        assert_ne!(id_a, id_b);
    }

    #[test]
    fn rejects_empty_required_fields() {
        let mut rec = rec_team();
        rec.tenant_id = String::new();
        let err = import_record_to_audit_row(&rec, &embedded_prices()).unwrap_err();
        assert!(matches!(err, ImportRecordError::MissingField("tenant_id")));

        let mut rec = rec_team();
        rec.devin_team_id = String::new();
        let err = import_record_to_audit_row(&rec, &embedded_prices()).unwrap_err();
        assert!(matches!(
            err,
            ImportRecordError::MissingField("devin_team_id")
        ));
    }

    #[test]
    fn rejects_unknown_plan() {
        let mut rec = rec_team();
        rec.plan = "solo".into();
        let err = import_record_to_audit_row(&rec, &embedded_prices()).unwrap_err();
        assert!(matches!(err, ImportRecordError::Lookup(_)));
    }

    #[test]
    fn rejects_negative_acu() {
        let mut rec = rec_team();
        rec.acu_consumed = -1.0;
        let err = import_record_to_audit_row(&rec, &embedded_prices()).unwrap_err();
        assert!(matches!(err, ImportRecordError::Conversion(_)));
    }

    #[test]
    fn rejects_live_mode_with_provenance() {
        let mut rec = rec_team();
        rec.ingestion_mode = IngestionMode::Live;
        // Keep the fixture hash — that's the misconfig we're testing.
        let err = import_record_to_audit_row(&rec, &embedded_prices()).unwrap_err();
        assert!(matches!(err, ImportRecordError::ProvenanceMismatch { .. },));
    }

    #[test]
    fn rejects_fixture_mode_without_provenance() {
        let mut rec = rec_team();
        rec.fixture_provenance_sha256 = None;
        let err = import_record_to_audit_row(&rec, &embedded_prices()).unwrap_err();
        assert!(matches!(err, ImportRecordError::ProvenanceMismatch { .. },));
    }

    #[test]
    fn determinism_same_input_same_output() {
        let prices = embedded_prices();
        let rec = rec_team();
        let a = import_record_to_audit_row(&rec, &prices).unwrap();
        let b = import_record_to_audit_row(&rec, &prices).unwrap();
        assert_eq!(a, b, "import_record_to_audit_row must be pure");
    }

    #[test]
    fn import_record_subject_does_not_leak_token() {
        // T7: subject is built from synthetic IDs only.
        let row = import_record_to_audit_row(&rec_team(), &embedded_prices()).unwrap();
        // The Bearer token sentinel string we use in fixtures is
        // `FAKE_DEVIN_TOKEN_*`. Confirm no row field contains that
        // pattern (defensive — `Bearer` is the canonical leak word).
        let json = serde_json::json!({
            "model": row.model,
            "tenant_id": row.tenant_id,
            "budget_id": row.budget_id,
        });
        let s = json.to_string();
        assert!(!s.contains("Bearer"));
        assert!(!s.contains("FAKE_DEVIN_TOKEN"));
        assert!(!s.to_lowercase().contains("authorization"));
    }
}
