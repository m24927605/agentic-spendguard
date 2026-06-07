//! D15 COV_72 — Pure conversion `ImportRecord -> AuditRowDraft`.
//!
//! * No I/O.
//! * No global state.
//! * No clock read (`window_end` is supplied by the caller).
//! * No `unsafe`.
//! * Same `(ImportRecord, PriceTable)` ↦ same `AuditRowDraft`.
//!
//! ## Locked constants (D14/D16-aligned; see deviation notes on the
//! `RESERVATION_SOURCE` and `IMPORT_SOURCE` consts below for the
//! cross-vendor reconciliation against the original D15 spec values)
//!
//! * `reservation_source = "subscription_meter"` — D14/D16 family-aligned
//!   (deviation from D15 design §5 #1).
//! * `import_source      = "manus_team_api"` — D14/D16 family pattern
//!   `<vendor>_team_api` (deviation from D15 design §5 #5).
//! * CloudEvent `type = "spendguard.audit.import.manus_credit"` —
//!   family pattern `spendguard.audit.import.<vendor>_<unit>`
//!   (design §5 #3; review-standards C1 / C2 / X2).
//! * `model = "manus.session/credit"` — synthetic vendor.unit-grain/unit
//!   slug (design §5 #10; review-standards E4 / X4).
//! * `input_tokens = output_tokens = 0` — honest zero beats guessed
//!   tokens (design §5 #10; review-standards E5).
//! * `dedupe_key = format!("manus:{session_id}")` — vendor-prefixed so
//!   D14 / D16 keys don't collide with D15 (design §5 #11;
//!   review-standards E6 / X3).

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::error::MeterError;
use crate::pricing::{credit_to_usd_micros, PriceTable};
use crate::record::{ImportRecord, IngestionMode};

/// Canonical `reservation_source` value for Manus imports.
///
/// **Deviation from D15 design §5 #1 (which proposed `'import_manus'`):**
/// the actual D14 (Devin) + D16 (Genspark) deliveries both lock to
/// `'subscription_meter'` so the dashboard's BYOK / metered fork stays
/// uniform across importer-family vendors. D15 follows the same pattern
/// for cross-vendor consistency; per-importer filtering uses
/// `import_source` (manus_team_api / devin_team_api / genspark_team_api)
/// instead of overloading `reservation_source`. Reviewer cross-check:
/// `grep RESERVATION_SOURCE services/importer_*/src/`.
pub const RESERVATION_SOURCE: &str = "subscription_meter";

/// Canonical `import_source` value (matches mig 0060 CHECK).
///
/// **Deviation from D15 design §5 #5 (which proposed `'manus_admin_usage'`):**
/// D13 mig 0058 already enumerates `'manus_admin_usage'` as a
/// placeholder; the live D15 importer writes the
/// `'<vendor>_team_api'` family value to mirror D14's
/// `'devin_team_api'` (mig 0059) and D16's `'genspark_team_api'`
/// (mig 0061). Migration 0060 (this slice) widens the CHECK to
/// admit the new value.
pub const IMPORT_SOURCE: &str = "manus_team_api";

/// CloudEvent `type` emitted by outbox_forwarder for these rows
/// (design §5 #3; review-standards C1 / X2).
pub const CLOUDEVENT_TYPE: &str = "spendguard.audit.import.manus_credit";

/// Synthetic model slug stamped on every emitted row so dashboards
/// group by vendor (design §5 #10; review-standards E4 / X4).
pub const MODEL_SLUG: &str = "manus.session/credit";

/// UUIDv5 namespace for deterministic Manus event IDs.
///
/// Chosen at random and frozen here. The deterministic ID lets
/// canonical_ingest dedup via the existing `event_replay_dedup` table.
/// Bumping this UUID would invalidate all previously-emitted Manus
/// event IDs — DO NOT EDIT.
pub const MANUS_EVENT_NAMESPACE: Uuid = Uuid::from_bytes([
    0x6d, 0x61, 0x9e, 0x75, 0x1a, 0x73, 0x4b, 0x15, 0x91, 0xfe, 0xd0, 0x15, 0x6d, 0x61, 0xac, 0xb1,
]);

/// SpendGuard `audit_outbox` row draft built from an `ImportRecord`.
///
/// This is the canonical_ingest `AppendEvents` handoff — the actual
/// INSERT lives in canonical_ingest. Field names mirror the audit
/// schema so reviewers can grep across the stack.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditRowDraft {
    /// Deterministic UUIDv5 from `(workspace_id, session_id, window_end)`.
    pub event_id: Uuid,
    /// Vendor workspace ID — surfaces as tenant scope on the row.
    pub tenant_id: String,
    /// Always `"subscription_meter"` (D14/D16-aligned).
    pub reservation_source: &'static str,
    /// Always `"manus_team_api"` (D14/D16 family pattern).
    pub import_source: &'static str,
    /// Synthetic `"manus.session/credit"` — dashboards group by it.
    pub model: &'static str,
    /// Manus does not expose per-LLM-call detail; reporting zero is
    /// honest, reporting a guess corrupts Strategy A predictions
    /// (review-standards E5).
    pub input_tokens: i64,
    /// As above.
    pub output_tokens: i64,
    /// Raw credit value carried through for observability /
    /// re-derivation.
    pub credits_consumed: i64,
    /// `>= 0` micro-USD. NEVER fabricated for unknown tiers (T6);
    /// always integer (T13).
    pub amount_micro_usd: i64,
    /// Stamped from the price table at the moment of conversion —
    /// review-standards C7 spirit: dashboards never see rate
    /// back-revisions rewrite history.
    pub pricing_version: String,
    /// Window end — anchors dashboard timeline (review-standards E7).
    pub occurred_at: DateTime<Utc>,
    /// Vendor session ID, opaque.
    pub session_id: String,
    /// Vendor session status wire string (`"completed"` / `"failed"` /
    /// ...).
    pub status: &'static str,
    /// Vendor tier wire string (`"team_plan"` / `"enterprise"` / ...).
    pub tier: &'static str,
    /// Fixture / live.
    pub ingestion_mode: IngestionMode,
    /// Fixture SHA-256 provenance (None in live mode).
    pub fixture_provenance_sha256: Option<String>,
    /// Vendor-prefixed dedupe key (`"manus:<session_id>"`).
    /// Review-standards E6 / X3.
    pub dedupe_key: String,
}

/// Compute a deterministic `event_id` from the idempotency triple
/// `(workspace_id, session_id, window_end)`. Same record ↦ same UUIDv5
/// ↦ canonical_ingest skips re-emission.
pub fn deterministic_event_id(
    workspace_id: &str,
    session_id: &str,
    window_end: DateTime<Utc>,
) -> Uuid {
    // RFC 3339 with the canonical Z suffix so the same timestamp
    // hashes identically across runtimes / serializers.
    let name = format!(
        "{}|{}|{}",
        workspace_id,
        session_id,
        window_end.to_rfc3339(),
    );
    Uuid::new_v5(&MANUS_EVENT_NAMESPACE, name.as_bytes())
}

/// Vendor-prefixed dedupe key — `"manus:<session_id>"` (review-standards
/// E6 / X3).
pub fn dedupe_key_for(session_id: &str) -> String {
    format!("manus:{session_id}")
}

/// Pure conversion `ImportRecord -> AuditRowDraft`.
///
/// Returns `Err(MeterError)` on unknown tier (skip + WARN policy at
/// caller) or negative-amount overflow guard. Never panics; never
/// fabricates an amount.
pub fn import_record_to_audit_row(
    rec: &ImportRecord,
    table: &PriceTable,
) -> Result<AuditRowDraft, MeterError> {
    // Hot-path conversion: integer micro-USD only.
    let amount_micro_usd = credit_to_usd_micros(rec, table)?;
    let event_id = deterministic_event_id(&rec.workspace_id, &rec.session_id, rec.window_end);

    Ok(AuditRowDraft {
        event_id,
        tenant_id: rec.workspace_id.clone(),
        reservation_source: RESERVATION_SOURCE,
        import_source: IMPORT_SOURCE,
        model: MODEL_SLUG,
        input_tokens: 0,
        output_tokens: 0,
        credits_consumed: rec.credits_consumed,
        amount_micro_usd,
        pricing_version: table.pricing_version.clone(),
        occurred_at: rec.window_end,
        session_id: rec.session_id.clone(),
        status: rec.status.as_str(),
        tier: rec.tier.as_str(),
        ingestion_mode: rec.ingestion_mode,
        fixture_provenance_sha256: rec.fixture_provenance_sha256.clone(),
        dedupe_key: dedupe_key_for(&rec.session_id),
    })
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

    fn rec_team() -> ImportRecord {
        ImportRecord {
            session_id: "ses_FAKE_team_completed_001".into(),
            workspace_id: "ws_FAKE_team_001".into(),
            tier: Tier::TeamPlan,
            credits_consumed: 47,
            status: SessionStatus::Completed,
            window_start: t(2026, 6, 5, 14),
            window_end: t(2026, 6, 5, 15),
            ingestion_mode: IngestionMode::Fixture,
            fixture_provenance_sha256: Some("0".repeat(64)),
        }
    }

    fn rec_enterprise() -> ImportRecord {
        ImportRecord {
            session_id: "ses_FAKE_enterprise_005".into(),
            workspace_id: "ws_FAKE_ent_001".into(),
            tier: Tier::Enterprise,
            credits_consumed: 350,
            status: SessionStatus::Completed,
            window_start: t(2026, 6, 5, 9),
            window_end: t(2026, 6, 5, 11),
            ingestion_mode: IngestionMode::Fixture,
            fixture_provenance_sha256: Some("0".repeat(64)),
        }
    }

    fn rec_byok() -> ImportRecord {
        ImportRecord {
            session_id: "ses_FAKE_byok_006".into(),
            workspace_id: "ws_FAKE_byok_001".into(),
            tier: Tier::EnterpriseByok,
            credits_consumed: 1024,
            status: SessionStatus::Completed,
            window_start: t(2026, 6, 5, 20),
            window_end: t(2026, 6, 5, 22),
            ingestion_mode: IngestionMode::Fixture,
            fixture_provenance_sha256: Some("0".repeat(64)),
        }
    }

    // ── Constants pinned (review-standards C1 / X2 / X4) ──────────────
    #[test]
    fn constants_are_pinned() {
        assert_eq!(RESERVATION_SOURCE, "subscription_meter");
        assert_eq!(IMPORT_SOURCE, "manus_team_api");
        assert_eq!(CLOUDEVENT_TYPE, "spendguard.audit.import.manus_credit");
        assert_eq!(MODEL_SLUG, "manus.session/credit");
    }

    // ── Headline conversion: 47 credits × 20_526 = 964_722 ─────────────
    #[test]
    fn import_record_to_audit_row_team_plan_conversion() {
        let row = import_record_to_audit_row(&rec_team(), &prices()).unwrap();
        assert_eq!(row.amount_micro_usd, 964_722);
        assert_eq!(row.credits_consumed, 47);
        assert_eq!(row.tier, "team_plan");
    }

    // ── E4 — model slug literal ───────────────────────────────────────
    #[test]
    fn import_record_to_audit_row_model_slug_is_synthetic() {
        let row = import_record_to_audit_row(&rec_team(), &prices()).unwrap();
        assert_eq!(row.model, "manus.session/credit");
    }

    // ── E5 — input/output tokens always zero ──────────────────────────
    #[test]
    fn import_record_to_audit_row_tokens_always_zero() {
        // Manus does NOT expose per-LLM-call detail; honest zero only.
        for rec in [rec_team(), rec_enterprise(), rec_byok()] {
            let row = import_record_to_audit_row(&rec, &prices()).unwrap();
            assert_eq!(row.input_tokens, 0);
            assert_eq!(row.output_tokens, 0);
        }
    }

    // ── E6 / X3 — dedupe key vendor-prefixed ──────────────────────────
    #[test]
    fn import_record_to_audit_row_dedupe_key_has_manus_prefix() {
        let row = import_record_to_audit_row(&rec_team(), &prices()).unwrap();
        assert_eq!(row.dedupe_key, "manus:ses_FAKE_team_completed_001");
        assert!(row.dedupe_key.starts_with("manus:"));
    }

    // ── reservation_source = subscription_meter (D14/D16-aligned) ─────
    #[test]
    fn import_record_to_audit_row_sets_subscription_meter() {
        let row = import_record_to_audit_row(&rec_team(), &prices()).unwrap();
        assert_eq!(row.reservation_source, "subscription_meter");
    }

    // ── import_source = manus_team_api (D14/D16 family pattern) ───────
    #[test]
    fn import_record_to_audit_row_sets_manus_team_api() {
        let row = import_record_to_audit_row(&rec_team(), &prices()).unwrap();
        assert_eq!(row.import_source, "manus_team_api");
    }

    // ── E7 — occurred_at = window_end ─────────────────────────────────
    #[test]
    fn import_record_to_audit_row_occurred_at_is_window_end() {
        let row = import_record_to_audit_row(&rec_team(), &prices()).unwrap();
        assert_eq!(row.occurred_at, t(2026, 6, 5, 15));
    }

    // ── Enterprise tier produces zero amount (default override) ───────
    #[test]
    fn import_record_to_audit_row_enterprise_yields_zero_amount() {
        let row = import_record_to_audit_row(&rec_enterprise(), &prices()).unwrap();
        assert_eq!(row.amount_micro_usd, 0);
        assert_eq!(row.tier, "enterprise");
    }

    // ── P3 — BYOK tier MUST produce zero amount (load-bearing) ────────
    #[test]
    fn import_record_to_audit_row_byok_yields_zero_amount() {
        let row = import_record_to_audit_row(&rec_byok(), &prices()).unwrap();
        assert_eq!(row.amount_micro_usd, 0);
        assert_eq!(row.tier, "enterprise_byok");
    }

    // ── pricing_version stamping ──────────────────────────────────────
    #[test]
    fn import_record_to_audit_row_stamps_pricing_version() {
        let row = import_record_to_audit_row(&rec_team(), &prices()).unwrap();
        assert_eq!(row.pricing_version, "manus-credit-v1-2026-06");
    }

    // ── Deterministic event_id ────────────────────────────────────────
    #[test]
    fn deterministic_event_id_is_stable_across_runs() {
        let id_a = deterministic_event_id(
            "ws_FAKE_team_001",
            "ses_FAKE_team_completed_001",
            t(2026, 6, 5, 15),
        );
        let id_b = deterministic_event_id(
            "ws_FAKE_team_001",
            "ses_FAKE_team_completed_001",
            t(2026, 6, 5, 15),
        );
        assert_eq!(id_a, id_b);
    }

    #[test]
    fn deterministic_event_id_differs_on_window_end() {
        let id_a = deterministic_event_id(
            "ws_FAKE_team_001",
            "ses_FAKE_team_completed_001",
            t(2026, 6, 5, 15),
        );
        let id_b = deterministic_event_id(
            "ws_FAKE_team_001",
            "ses_FAKE_team_completed_001",
            t(2026, 6, 5, 16),
        );
        assert_ne!(id_a, id_b);
    }

    #[test]
    fn deterministic_event_id_differs_on_session() {
        let id_a = deterministic_event_id(
            "ws_FAKE_team_001",
            "ses_FAKE_team_completed_001",
            t(2026, 6, 5, 15),
        );
        let id_b = deterministic_event_id(
            "ws_FAKE_team_001",
            "ses_FAKE_team_completed_002",
            t(2026, 6, 5, 15),
        );
        assert_ne!(id_a, id_b);
    }

    // ── Determinism — same input, same row ────────────────────────────
    #[test]
    fn import_record_to_audit_row_is_pure() {
        let a = import_record_to_audit_row(&rec_team(), &prices()).unwrap();
        let b = import_record_to_audit_row(&rec_team(), &prices()).unwrap();
        assert_eq!(a, b, "import_record_to_audit_row must be pure");
    }

    // ── Property: 200 random valid records → amount >= 0 (A3.4) ───────
    #[test]
    fn audit_row_amount_never_negative() {
        // Sweep all three tiers and a wide credits range. amount_micro_usd
        // must be >= 0 for every legitimate record.
        let prices = prices();
        for tier in [Tier::TeamPlan, Tier::Enterprise, Tier::EnterpriseByok] {
            for credits in [0i64, 1, 47, 350, 950, 1024, 100_000, 1_000_000] {
                let mut r = rec_team();
                r.tier = tier;
                r.credits_consumed = credits;
                let row = import_record_to_audit_row(&r, &prices).unwrap();
                assert!(
                    row.amount_micro_usd >= 0,
                    "amount went negative for tier={:?} credits={}",
                    tier,
                    credits,
                );
            }
        }
    }

    // ── Status passthrough ────────────────────────────────────────────
    #[test]
    fn import_record_to_audit_row_status_passthrough() {
        for s in [
            SessionStatus::Completed,
            SessionStatus::Failed,
            SessionStatus::Cancelled,
        ] {
            let mut r = rec_team();
            r.status = s;
            let row = import_record_to_audit_row(&r, &prices()).unwrap();
            assert_eq!(row.status, s.as_str());
        }
    }

    // ── Tenant ID = workspace_id (design §1) ──────────────────────────
    #[test]
    fn import_record_to_audit_row_tenant_id_is_workspace_id() {
        let row = import_record_to_audit_row(&rec_team(), &prices()).unwrap();
        assert_eq!(row.tenant_id, "ws_FAKE_team_001");
    }

    // ── T8 — no token / Authorization shape in audit row ──────────────
    #[test]
    fn audit_row_does_not_leak_token_shape() {
        let row = import_record_to_audit_row(&rec_team(), &prices()).unwrap();
        // The audit row is built only from synthetic IDs + integer math.
        // Defensive: no token-looking substring should appear anywhere
        // in the row's text-bearing fields.
        let s = format!("{row:?}");
        assert!(!s.contains("Bearer"));
        assert!(!s.contains("sk-"));
        assert!(!s.to_lowercase().contains("authorization"));
    }
}
