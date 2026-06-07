//! D16 COV_85 — `ImportRecord` shape + pure
//! `import_record_to_audit_row`.
//!
//! Mirrors the D13 importer contract
//! (`services/ledger/src/subscription_importer/stub.rs`) but with
//! Genspark-specific fields (credits consumed, plan tier, ingestion
//! mode, fixture provenance). The D13 `AuditRowDraft` shape is *not*
//! reused verbatim because the Genspark row carries the credit rate
//! provenance — the D13 stub focuses on `(input_tokens, output_tokens,
//! amount_atomic)` (token-billed providers). D16 ships its own
//! `AuditRowDraft` co-located with the importer; canonical_ingest
//! INSERT path treats both identically (same `import_source` column,
//! same `reservation_source` column).
//!
//! ## Purity (review-standards T6)
//!
//! `import_record_to_audit_row` is **pure**: no I/O, no global state,
//! no clock read. Same `(ImportRecord, CreditPriceTable)` ↦ same
//! `AuditRowDraft`. Trivially fuzz / property testable.
//!
//! ## Unknown-plan fallback (review-standards T7)
//!
//! When the plan slug is absent from the price table, we DO NOT propagate
//! the lookup error — instead, the row lands with `amount_micro_usd = 0`
//! AND `reason_code = Some("genspark_plan_unknown")`. Both fields must be
//! set; setting only one would either silently mis-price (visible non-zero
//! USD with no rate) or leave the dashboard with no signal.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::credit_price_table::{
    credits_to_micro_usd, ConversionError, CreditPriceTable, PriceLookupError,
};

/// Sentinel string for `audit_outbox.reservation_source`.
///
/// Locked in D16 design §6 decision #1: every Genspark importer row is
/// `import_genspark`. The spec's design.md §6 #1 calls for a NEW
/// distinct value, but the actual canonical_ingest CHECK constraint is
/// `('byok', 'subscription_meter')` — D14 uses `subscription_meter`
/// for its sibling rows and the SpendGuard wire continues that pattern.
/// We therefore stamp `subscription_meter` here (matches D14 and the
/// schema constraint that landed in production), keeping
/// `import_source = 'genspark_team_api'` as the per-vendor router.
/// Dashboard distinguishes Genspark vs Devin via `import_source`, not
/// `reservation_source`.
pub const RESERVATION_SOURCE_SUBSCRIPTION_METER: &str = "subscription_meter";

/// Sentinel string for `audit_outbox.import_source`.
///
/// Locked in D16 design §6 decision #5. Matches the value that mig
/// 0061 adds to the CHECK constraint.
pub const IMPORT_SOURCE_GENSPARK_TEAM_API: &str = "genspark_team_api";

/// `reason_code` stamped on rows where the plan slug is absent from
/// the embedded price table. Lands with `amount_micro_usd = 0` so the
/// dashboard sees the credit consumption but treats the dollar field
/// as unpriced — never silently mis-priced.
pub const REASON_CODE_GENSPARK_PLAN_UNKNOWN: &str = "genspark_plan_unknown";

/// UUIDv5 namespace for deterministic Genspark event IDs.
///
/// Chosen at random and frozen here. Bumping this UUID would
/// invalidate all previously-emitted Genspark event IDs — DO NOT EDIT.
pub const GENSPARK_EVENT_NAMESPACE: Uuid = Uuid::from_bytes([
    0x9e, 0x71, 0x5b, 0xa4, 0x1a, 0x73, 0x4b, 0x16, 0x91, 0xfe, 0xd0, 0x16, 0x9e, 0x71, 0x5b, 0xa0,
]);

/// Whether the ImportRecord was produced from a committed fixture
/// (default merge gate) or from the live Genspark admin API (gated on
/// `live` Cargo feature + `GENSPARK_API_TOKEN`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestionMode {
    /// Fixture replay — default merge gate. Carries
    /// `fixture_provenance_sha256` so audit rows are auditable.
    Fixture,
    /// Live admin API pull. `fixture_provenance_sha256` is `None`.
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

/// One row of Genspark admin API usage. The shape mirrors the
/// `/v1/admin/usage` response (design §4) plus the SpendGuard-side
/// bookkeeping fields.
///
/// All amounts are computed downstream; the import record carries the
/// raw vendor signal (`credits_consumed`, `plan`).
#[derive(Debug, Clone, PartialEq)]
pub struct ImportRecord {
    /// SpendGuard tenant ID (caller-supplied — the importer is a
    /// single-tenant job at a time).
    pub tenant_id: String,
    /// SpendGuard budget ID. Optional in some flows but D16 always
    /// requires one — Genspark rows land on a budget the operator
    /// pre-provisioned.
    pub budget_id: String,
    /// Genspark workspace identifier. Opaque. Same `(workspace_id,
    /// task_id, window_end)` triple ↦ idempotent event.id.
    pub workspace_id: String,
    /// Genspark task identifier — distinguishes per-task credit draws
    /// within the same workspace + window. Opaque.
    pub task_id: String,
    /// Raw credit value from the Genspark admin API.
    pub credits_consumed: f64,
    /// Genspark plan slug (`"plus"` / `"pro"` / `"premium"`). Looked
    /// up against the price table at conversion time. Unknown slugs
    /// fall back to `amount_micro_usd = 0` +
    /// `reason_code = genspark_plan_unknown`.
    pub plan: String,
    /// Optional task category for dashboard breakdown
    /// (`"research"` / `"code_generation"` / etc.). Opaque.
    pub task_category: Option<String>,
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
/// (mig 0056/0058/0061) so reviewers can grep across stack.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditRowDraft {
    /// Deterministic UUIDv5 from `(workspace_id, task_id, window_end)`.
    /// Same record produces the same `event_id` so canonical_ingest's
    /// `event_replay_dedup` skips re-emission.
    pub event_id: Uuid,
    /// Tenant scope.
    pub tenant_id: String,
    /// Budget scope.
    pub budget_id: String,
    /// Always `"subscription_meter"`.
    pub reservation_source: &'static str,
    /// Always `"genspark_team_api"`.
    pub import_source: &'static str,
    /// Pseudo-model slug stamped on the row so dashboards group by
    /// vendor: `"genspark/credit/<plan>"`.
    pub model: String,
    /// Raw credit value carried through to the audit row for
    /// observability / re-derivation.
    pub credits_consumed: f64,
    /// `i64` micro-USD. `0` when the plan slug is unknown (paired with
    /// `reason_code = genspark_plan_unknown`).
    pub amount_micro_usd: i64,
    /// Always `Some` for D16 rows — the price table's
    /// `pricing_version` at conversion time.
    pub pricing_version: Option<String>,
    /// `Some("genspark_plan_unknown")` when the plan slug was absent
    /// from the price table. None otherwise.
    pub reason_code: Option<&'static str>,
    /// Window end — anchors dashboard timeline.
    pub occurred_at: DateTime<Utc>,
    /// Fixture / live.
    pub ingestion_mode: IngestionMode,
    /// Fixture SHA-256 provenance (None in live mode).
    pub fixture_provenance_sha256: Option<String>,
    /// Pass-through task category for downstream dashboards.
    pub task_category: Option<String>,
}

/// All ways `import_record_to_audit_row` can fail.
///
/// Note: an unknown plan does NOT fail — it falls back to
/// `amount_micro_usd = 0` + `reason_code = genspark_plan_unknown`
/// (review-standards T7, F4). Only arithmetic / field-validation
/// failures surface as `Err`.
#[derive(Debug, thiserror::Error)]
pub enum ImportRecordError {
    /// The credit value / rate failed validation.
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
/// `(workspace_id, task_id, window_end)`. Same record ↦ same UUIDv5
/// ↦ canonical_ingest skips re-emission.
pub fn deterministic_event_id(
    workspace_id: &str,
    task_id: &str,
    window_end: DateTime<Utc>,
) -> Uuid {
    // RFC 3339 with the canonical Z suffix so the same timestamp
    // hashes identically across runtimes / serializers.
    let name = format!(
        "{}|{}|{}",
        workspace_id,
        task_id,
        window_end.to_rfc3339(),
    );
    Uuid::new_v5(&GENSPARK_EVENT_NAMESPACE, name.as_bytes())
}

/// Pure conversion `ImportRecord -> AuditRowDraft`.
///
/// * No I/O.
/// * No global state.
/// * No clock read (`window_end` is supplied by the caller).
/// * No `unsafe`.
/// * Same `(ImportRecord, CreditPriceTable)` ↦ same `AuditRowDraft`.
///
/// Returns `Err(ImportRecordError)` on missing fields, a negative /
/// non-finite credit value, or a fixture/live provenance mismatch.
///
/// Does NOT return `Err` for an unknown plan slug — that case lands as
/// `amount_micro_usd = 0` + `reason_code = genspark_plan_unknown`
/// per design §3.1 + review-standards T7.
pub fn import_record_to_audit_row(
    rec: &ImportRecord,
    prices: &CreditPriceTable,
) -> Result<AuditRowDraft, ImportRecordError> {
    // ── Required-field validation ────────────────────────────────────
    if rec.tenant_id.is_empty() {
        return Err(ImportRecordError::MissingField("tenant_id"));
    }
    if rec.budget_id.is_empty() {
        return Err(ImportRecordError::MissingField("budget_id"));
    }
    if rec.workspace_id.is_empty() {
        return Err(ImportRecordError::MissingField("workspace_id"));
    }
    if rec.task_id.is_empty() {
        return Err(ImportRecordError::MissingField("task_id"));
    }
    if rec.plan.is_empty() {
        return Err(ImportRecordError::MissingField("plan"));
    }

    // ── Provenance mode invariant (review-standards X1) ─────────────
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
    // T7: unknown plan ↦ amount = 0, reason_code = genspark_plan_unknown.
    let (amount_micro_usd, reason_code): (i64, Option<&'static str>) = match prices.lookup(&rec.plan)
    {
        Ok(rate) => {
            let amt = credits_to_micro_usd(rec.credits_consumed, rate.usd_per_credit)?;
            (amt, None)
        }
        Err(PriceLookupError::PlanNotFound(_)) => (0, Some(REASON_CODE_GENSPARK_PLAN_UNKNOWN)),
    };

    let event_id = deterministic_event_id(&rec.workspace_id, &rec.task_id, rec.window_end);

    Ok(AuditRowDraft {
        event_id,
        tenant_id: rec.tenant_id.clone(),
        budget_id: rec.budget_id.clone(),
        reservation_source: RESERVATION_SOURCE_SUBSCRIPTION_METER,
        import_source: IMPORT_SOURCE_GENSPARK_TEAM_API,
        model: format!("genspark/credit/{}", rec.plan),
        credits_consumed: rec.credits_consumed,
        amount_micro_usd,
        pricing_version: Some(prices.pricing_version.clone()),
        reason_code,
        occurred_at: rec.window_end,
        ingestion_mode: rec.ingestion_mode,
        fixture_provenance_sha256: rec.fixture_provenance_sha256.clone(),
        task_category: rec.task_category.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use pretty_assertions::assert_eq;

    fn embedded_prices() -> CreditPriceTable {
        CreditPriceTable::load_from_embedded()
    }

    fn t(y: i32, mo: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, 0, 0).unwrap()
    }

    fn rec_plus() -> ImportRecord {
        ImportRecord {
            tenant_id: "tenant-alpha".into(),
            budget_id: "genspark-budget".into(),
            workspace_id: "FAKE_ws_001".into(),
            task_id: "FAKE_task_001".into(),
            credits_consumed: 3200.0,
            plan: "plus".into(),
            task_category: Some("research".into()),
            window_start: t(2026, 6, 1, 0),
            window_end: t(2026, 6, 1, 1),
            ingestion_mode: IngestionMode::Fixture,
            fixture_provenance_sha256: Some("0".repeat(64)),
        }
    }

    fn rec_premium() -> ImportRecord {
        ImportRecord {
            tenant_id: "tenant-alpha".into(),
            budget_id: "genspark-budget".into(),
            workspace_id: "FAKE_ws_002".into(),
            task_id: "FAKE_task_002".into(),
            credits_consumed: 50_000.0,
            plan: "premium".into(),
            task_category: Some("code_generation".into()),
            window_start: t(2026, 6, 1, 0),
            window_end: t(2026, 6, 1, 1),
            ingestion_mode: IngestionMode::Fixture,
            fixture_provenance_sha256: Some("0".repeat(64)),
        }
    }

    fn rec_unknown_plan() -> ImportRecord {
        ImportRecord {
            tenant_id: "tenant-alpha".into(),
            budget_id: "genspark-budget".into(),
            workspace_id: "FAKE_ws_003".into(),
            task_id: "FAKE_task_003".into(),
            credits_consumed: 1000.0,
            plan: "enterprise".into(),
            task_category: None,
            window_start: t(2026, 6, 1, 0),
            window_end: t(2026, 6, 1, 1),
            ingestion_mode: IngestionMode::Fixture,
            fixture_provenance_sha256: Some("0".repeat(64)),
        }
    }

    // ── Headline conversion gate: 3200 credits × $0.001999/credit ────
    #[test]
    fn import_record_to_audit_row_amount_conversion() {
        let row = import_record_to_audit_row(&rec_plus(), &embedded_prices()).unwrap();
        // 3200 × 0.001999 × 1e6 = 6_396_800
        assert_eq!(row.amount_micro_usd, 6_396_800);
        assert_eq!(row.reason_code, None);
    }

    #[test]
    fn import_record_to_audit_row_sets_subscription_meter() {
        let row = import_record_to_audit_row(&rec_plus(), &embedded_prices()).unwrap();
        assert_eq!(row.reservation_source, "subscription_meter");
    }

    #[test]
    fn import_record_to_audit_row_sets_import_source_genspark_team_api() {
        let row = import_record_to_audit_row(&rec_plus(), &embedded_prices()).unwrap();
        assert_eq!(row.import_source, "genspark_team_api");
    }

    // ── Unknown-plan fallback (T7) — BOTH fields set ─────────────────
    #[test]
    fn import_record_to_audit_row_unknown_plan_zeros_amount_and_sets_reason() {
        let row = import_record_to_audit_row(&rec_unknown_plan(), &embedded_prices()).unwrap();
        // T7: BOTH fields must be set, never just one.
        assert_eq!(row.amount_micro_usd, 0);
        assert_eq!(row.reason_code, Some("genspark_plan_unknown"));
    }

    #[test]
    fn import_record_to_audit_row_stamps_pricing_version() {
        let row = import_record_to_audit_row(&rec_plus(), &embedded_prices()).unwrap();
        assert_eq!(
            row.pricing_version.as_deref(),
            Some("genspark-credit-v1-2026-06"),
        );
    }

    #[test]
    fn import_record_to_audit_row_pricing_version_stamped_even_on_unknown_plan() {
        // Unknown plan still gets a pricing_version stamp so the
        // dashboard knows which rate-table run produced the row.
        let row = import_record_to_audit_row(&rec_unknown_plan(), &embedded_prices()).unwrap();
        assert_eq!(
            row.pricing_version.as_deref(),
            Some("genspark-credit-v1-2026-06"),
        );
    }

    #[test]
    fn import_record_to_audit_row_model_slug_includes_plan() {
        let row = import_record_to_audit_row(&rec_plus(), &embedded_prices()).unwrap();
        assert_eq!(row.model, "genspark/credit/plus");

        let row = import_record_to_audit_row(&rec_premium(), &embedded_prices()).unwrap();
        assert_eq!(row.model, "genspark/credit/premium");

        let row = import_record_to_audit_row(&rec_unknown_plan(), &embedded_prices()).unwrap();
        assert_eq!(row.model, "genspark/credit/enterprise");
    }

    #[test]
    fn import_record_to_audit_row_occurred_at_is_window_end() {
        let row = import_record_to_audit_row(&rec_plus(), &embedded_prices()).unwrap();
        assert_eq!(row.occurred_at, t(2026, 6, 1, 1));
    }

    #[test]
    fn import_record_to_audit_row_carries_task_category() {
        let row = import_record_to_audit_row(&rec_plus(), &embedded_prices()).unwrap();
        assert_eq!(row.task_category.as_deref(), Some("research"));

        let row = import_record_to_audit_row(&rec_unknown_plan(), &embedded_prices()).unwrap();
        assert_eq!(row.task_category, None);
    }

    #[test]
    fn deterministic_event_id_is_stable_across_runs() {
        let id_a = deterministic_event_id("FAKE_ws_001", "FAKE_task_001", t(2026, 6, 1, 1));
        let id_b = deterministic_event_id("FAKE_ws_001", "FAKE_task_001", t(2026, 6, 1, 1));
        assert_eq!(id_a, id_b);
    }

    #[test]
    fn deterministic_event_id_differs_on_window_change() {
        let id_a = deterministic_event_id("FAKE_ws_001", "FAKE_task_001", t(2026, 6, 1, 1));
        let id_b = deterministic_event_id("FAKE_ws_001", "FAKE_task_001", t(2026, 6, 1, 2));
        assert_ne!(id_a, id_b);
    }

    #[test]
    fn deterministic_event_id_differs_on_workspace_change() {
        let id_a = deterministic_event_id("FAKE_ws_001", "FAKE_task_001", t(2026, 6, 1, 1));
        let id_b = deterministic_event_id("FAKE_ws_002", "FAKE_task_001", t(2026, 6, 1, 1));
        assert_ne!(id_a, id_b);
    }

    #[test]
    fn deterministic_event_id_differs_on_task_change() {
        let id_a = deterministic_event_id("FAKE_ws_001", "FAKE_task_001", t(2026, 6, 1, 1));
        let id_b = deterministic_event_id("FAKE_ws_001", "FAKE_task_002", t(2026, 6, 1, 1));
        assert_ne!(id_a, id_b);
    }

    #[test]
    fn rejects_empty_tenant_id() {
        let mut rec = rec_plus();
        rec.tenant_id = String::new();
        let err = import_record_to_audit_row(&rec, &embedded_prices()).unwrap_err();
        assert!(matches!(err, ImportRecordError::MissingField("tenant_id")));
    }

    #[test]
    fn rejects_empty_budget_id() {
        let mut rec = rec_plus();
        rec.budget_id = String::new();
        let err = import_record_to_audit_row(&rec, &embedded_prices()).unwrap_err();
        assert!(matches!(err, ImportRecordError::MissingField("budget_id")));
    }

    #[test]
    fn rejects_empty_workspace_id() {
        let mut rec = rec_plus();
        rec.workspace_id = String::new();
        let err = import_record_to_audit_row(&rec, &embedded_prices()).unwrap_err();
        assert!(matches!(
            err,
            ImportRecordError::MissingField("workspace_id")
        ));
    }

    #[test]
    fn rejects_empty_task_id() {
        let mut rec = rec_plus();
        rec.task_id = String::new();
        let err = import_record_to_audit_row(&rec, &embedded_prices()).unwrap_err();
        assert!(matches!(err, ImportRecordError::MissingField("task_id")));
    }

    #[test]
    fn rejects_empty_plan() {
        let mut rec = rec_plus();
        rec.plan = String::new();
        let err = import_record_to_audit_row(&rec, &embedded_prices()).unwrap_err();
        assert!(matches!(err, ImportRecordError::MissingField("plan")));
    }

    #[test]
    fn rejects_negative_credits() {
        let mut rec = rec_plus();
        rec.credits_consumed = -1.0;
        let err = import_record_to_audit_row(&rec, &embedded_prices()).unwrap_err();
        assert!(matches!(err, ImportRecordError::Conversion(_)));
    }

    #[test]
    fn rejects_nan_credits() {
        let mut rec = rec_plus();
        rec.credits_consumed = f64::NAN;
        let err = import_record_to_audit_row(&rec, &embedded_prices()).unwrap_err();
        assert!(matches!(err, ImportRecordError::Conversion(_)));
    }

    #[test]
    fn rejects_infinite_credits() {
        let mut rec = rec_plus();
        rec.credits_consumed = f64::INFINITY;
        let err = import_record_to_audit_row(&rec, &embedded_prices()).unwrap_err();
        assert!(matches!(err, ImportRecordError::Conversion(_)));
    }

    #[test]
    fn rejects_live_mode_with_provenance() {
        let mut rec = rec_plus();
        rec.ingestion_mode = IngestionMode::Live;
        // Keep the fixture hash — that's the misconfig we're testing.
        let err = import_record_to_audit_row(&rec, &embedded_prices()).unwrap_err();
        assert!(matches!(err, ImportRecordError::ProvenanceMismatch { .. }));
    }

    #[test]
    fn rejects_fixture_mode_without_provenance() {
        let mut rec = rec_plus();
        rec.fixture_provenance_sha256 = None;
        let err = import_record_to_audit_row(&rec, &embedded_prices()).unwrap_err();
        assert!(matches!(err, ImportRecordError::ProvenanceMismatch { .. }));
    }

    #[test]
    fn determinism_same_input_same_output() {
        let prices = embedded_prices();
        let rec = rec_plus();
        let a = import_record_to_audit_row(&rec, &prices).unwrap();
        let b = import_record_to_audit_row(&rec, &prices).unwrap();
        assert_eq!(a, b, "import_record_to_audit_row must be pure");
    }

    #[test]
    fn determinism_holds_for_unknown_plan_too() {
        let prices = embedded_prices();
        let rec = rec_unknown_plan();
        let a = import_record_to_audit_row(&rec, &prices).unwrap();
        let b = import_record_to_audit_row(&rec, &prices).unwrap();
        assert_eq!(a, b, "unknown-plan fallback must be deterministic");
    }

    #[test]
    fn import_record_subject_does_not_leak_token() {
        // T1: subject is built from synthetic IDs only. Defensive
        // check — `Bearer` is the canonical leak word.
        let row = import_record_to_audit_row(&rec_plus(), &embedded_prices()).unwrap();
        let json = serde_json::json!({
            "model": row.model,
            "tenant_id": row.tenant_id,
            "budget_id": row.budget_id,
        });
        let s = json.to_string();
        assert!(!s.contains("Bearer"));
        assert!(!s.contains("FAKE_GENSPARK_TOKEN"));
        assert!(!s.to_lowercase().contains("authorization"));
    }

    // ── F4 invariant: unknown-plan row STILL emits a row (no skip) ───
    #[test]
    fn unknown_plan_row_is_still_emitted_not_skipped() {
        // F4: import_record_to_audit_row must NOT return Err for an
        // unknown plan — the row lands with amount_micro_usd = 0 so
        // the dashboard sees the credit consumption.
        let row = import_record_to_audit_row(&rec_unknown_plan(), &embedded_prices()).unwrap();
        assert!(row.credits_consumed > 0.0, "credit signal preserved");
        assert_eq!(row.amount_micro_usd, 0);
        assert_eq!(row.reason_code, Some("genspark_plan_unknown"));
    }

    #[test]
    fn pro_plan_conversion_produces_positive_amount() {
        let mut rec = rec_plus();
        rec.plan = "pro".into();
        rec.credits_consumed = 5_000.0;
        let row = import_record_to_audit_row(&rec, &embedded_prices()).unwrap();
        // 5_000 × 0.0019992 × 1e6 = 9_996_000
        assert!(row.amount_micro_usd > 0);
        assert_eq!(row.reason_code, None);
    }

    #[test]
    fn ingestion_mode_propagates_to_audit_row() {
        let row = import_record_to_audit_row(&rec_plus(), &embedded_prices()).unwrap();
        assert_eq!(row.ingestion_mode, IngestionMode::Fixture);
        assert!(row.fixture_provenance_sha256.is_some());
    }
}
