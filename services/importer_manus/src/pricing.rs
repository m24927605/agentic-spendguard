//! D15 COV_71 — Credit -> micro-USD conversion. Pure, no IO, **integer
//! math only**.
//!
//! The price table asset (`assets/price_table.toml`) is the source of
//! truth for tier -> micro-USD-per-credit. The asset is `include_str!`-d
//! at compile time so the binary ships with a self-contained rate table
//! — no I/O, no runtime config load (review-standards P5).
//!
//! ## Conversion rules (design §5 + review-standards §2)
//!
//! * Conversion is **pure** and **deterministic**: same
//!   `(credits_consumed, credit_cost_micro_usd)` ↦ same `i64` micro-USD.
//! * No `f64` in the hot path (P4). TOML deserialization parses
//!   `i64` directly.
//! * Negative `credits_consumed` returns `Err(NegativeAmount)`.
//! * Unknown tier returns `Err(UnknownTier)` — caller emits WARN +
//!   skips the row, NEVER fabricates an amount (T6).
//! * `saturating_mul` guards i64 overflow (T13).

use serde::Deserialize;
use std::collections::HashMap;

use crate::error::MeterError;
use crate::record::{ImportRecord, Tier};

/// Embedded Manus credit price table. Loaded once via
/// [`PriceTable::load_embedded`].
///
/// The `pricing_version` field is the contract handle: rate changes
/// MUST bump this string. Historical audit rows carry the version used
/// at conversion time so dashboards never see a rate back-revision
/// rewrite history (review-standards P5 / design §5 #12).
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PriceTable {
    /// Opaque version string. Bumped on every rate change.
    pub pricing_version: String,
    /// Effective-from timestamp (documentation-only).
    pub effective_from: chrono::DateTime<chrono::Utc>,
    /// ISO 4217 currency. Always `"USD"` today.
    pub currency: String,
    /// One entry per tier slug.
    pub tiers: HashMap<String, TierPricing>,
}

/// One tier-rate entry.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub struct TierPricing {
    /// Integer micro-USD per credit. NO f64 in the hot path (P4).
    pub credit_cost_micro_usd: i64,
}

impl PriceTable {
    /// Parse the embedded `assets/price_table.toml` asset. Panics on a
    /// malformed asset — by construction the asset is committed and a
    /// malformed asset means the build is broken.
    pub fn load_embedded() -> Self {
        const TOML: &str = include_str!("../assets/price_table.toml");
        let table: Self =
            toml::from_str(TOML).expect("embedded price_table.toml must be valid TOML");
        debug_assert!(
            !table.pricing_version.is_empty(),
            "pricing_version must be non-empty",
        );
        debug_assert!(
            table.tiers.len() >= 3,
            "price table must enumerate all three Manus tiers",
        );
        table
    }

    /// Parse an arbitrary TOML string into a price table. Used by tests
    /// that exercise specific rate shapes without mutating the embedded
    /// asset.
    pub fn from_toml_str(raw: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(raw)
    }

    /// Look up the micro-USD-per-credit rate for a tier.
    pub fn credit_cost(&self, tier: Tier) -> Result<i64, MeterError> {
        self.tiers
            .get(tier.as_str())
            .map(|t| t.credit_cost_micro_usd)
            .ok_or(MeterError::UnknownTier(tier))
    }
}

/// Pure `credits * micro_usd_per_credit` conversion.
///
/// * `saturating_mul` guards against i64 overflow on hostile fixture
///   input (review-standards T13).
/// * Returns `Err(NegativeAmount)` if the saturating multiply produced
///   a negative result — only possible when `credits_consumed < 0`
///   slipped past fixture validation (defensive belt-and-suspenders).
///
/// The function is `#[inline]`-able by design: no allocations, no I/O,
/// no clock reads. Safe to call from a fuzz harness.
pub fn credit_to_usd_micros(rec: &ImportRecord, table: &PriceTable) -> Result<i64, MeterError> {
    let per_credit = table.credit_cost(rec.tier)?;
    let total = rec.credits_consumed.saturating_mul(per_credit);
    if total < 0 {
        return Err(MeterError::NegativeAmount);
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::IngestionMode;
    use chrono::{TimeZone, Utc};
    use pretty_assertions::assert_eq;

    fn embedded() -> PriceTable {
        PriceTable::load_embedded()
    }

    fn rec(tier: Tier, credits: i64) -> ImportRecord {
        ImportRecord {
            session_id: "ses_FAKE_unit_001".into(),
            workspace_id: "ws_FAKE_unit_001".into(),
            tier,
            credits_consumed: credits,
            status: crate::record::SessionStatus::Completed,
            window_start: Utc.with_ymd_and_hms(2026, 6, 5, 0, 0, 0).unwrap(),
            window_end: Utc.with_ymd_and_hms(2026, 6, 5, 1, 0, 0).unwrap(),
            ingestion_mode: IngestionMode::Fixture,
            fixture_provenance_sha256: Some("0".repeat(64)),
        }
    }

    // ── Test 1: embedded table loads + has all three tiers ────────────
    #[test]
    fn price_table_load_embedded_succeeds() {
        // Pinned constants per review-standards P1-P3 + design §5.
        let t = embedded();
        assert_eq!(t.pricing_version, "manus-credit-v1-2026-06");
        assert_eq!(t.currency, "USD");
        assert!(t.tiers.len() >= 3);
        assert!(t.tiers.contains_key("team_plan"));
        assert!(t.tiers.contains_key("enterprise"));
        assert!(t.tiers.contains_key("enterprise_byok"));
    }

    // ── Test 2: P1 team_plan exactly 20_526 ───────────────────────────
    #[test]
    fn price_table_team_plan_rate_is_locked_at_20_526() {
        let t = embedded();
        let rate = t.credit_cost(Tier::TeamPlan).unwrap();
        // P1: (39.00 / 1900.0) * 1_000_000.0 ≈ 20526.3; truncate -> 20_526.
        assert_eq!(rate, 20_526);
    }

    // ── Test 3: P2 enterprise default zero ────────────────────────────
    #[test]
    fn price_table_enterprise_default_is_zero() {
        let t = embedded();
        // P2: 0 by default; operator-override at deploy time.
        assert_eq!(t.credit_cost(Tier::Enterprise).unwrap(), 0);
    }

    // ── Test 4: P3 BYOK load-bearing zero ─────────────────────────────
    #[test]
    fn price_table_enterprise_byok_is_load_bearing_zero() {
        let t = embedded();
        // P3: 0 BYOK customers pay LLM provider direct; never raise.
        assert_eq!(t.credit_cost(Tier::EnterpriseByok).unwrap(), 0);
    }

    // ── Test 5: headline conversion math team_plan ────────────────────
    #[test]
    fn credit_to_usd_micros_team_plan_basic() {
        // 47 credits × 20_526 micro-USD = 964_722.
        let r = rec(Tier::TeamPlan, 47);
        let amt = credit_to_usd_micros(&r, &embedded()).unwrap();
        assert_eq!(amt, 47 * 20_526);
        assert_eq!(amt, 964_722);
    }

    // ── Test 6: zero credits yields zero micros ───────────────────────
    #[test]
    fn credit_to_usd_micros_zero_credits_yields_zero() {
        let r = rec(Tier::TeamPlan, 0);
        let amt = credit_to_usd_micros(&r, &embedded()).unwrap();
        assert_eq!(amt, 0);
    }

    // ── Test 7: enterprise tier yields zero (default) ─────────────────
    #[test]
    fn credit_to_usd_micros_enterprise_default_yields_zero() {
        let r = rec(Tier::Enterprise, 350);
        let amt = credit_to_usd_micros(&r, &embedded()).unwrap();
        assert_eq!(amt, 0);
    }

    // ── Test 8: BYOK tier yields zero (load-bearing) ──────────────────
    #[test]
    fn credit_to_usd_micros_enterprise_byok_yields_zero() {
        // P3 load-bearing: NEVER bill BYOK customers via the importer.
        let r = rec(Tier::EnterpriseByok, 1024);
        let amt = credit_to_usd_micros(&r, &embedded()).unwrap();
        assert_eq!(amt, 0);
    }

    // ── Test 9: saturating multiply caps at i64::MAX (T13) ────────────
    #[test]
    fn credit_to_usd_micros_saturates_at_i64_max() {
        // 9e17 credits × 20_526 micro-USD/credit ≈ 1.85e22 — well past
        // i64::MAX (~9.2e18). Saturating multiply MUST cap, never wrap
        // or panic.
        let r = rec(Tier::TeamPlan, i64::MAX / 2);
        let amt = credit_to_usd_micros(&r, &embedded()).unwrap();
        assert_eq!(amt, i64::MAX);
    }

    // ── Test 10: from_toml_str round-trip ─────────────────────────────
    #[test]
    fn price_table_from_toml_str_round_trip() {
        let raw = r#"
pricing_version = "test-v1"
effective_from  = "2026-06-01T00:00:00Z"
currency        = "USD"

[tiers.team_plan]
credit_cost_micro_usd = 1234

[tiers.enterprise]
credit_cost_micro_usd = 0

[tiers.enterprise_byok]
credit_cost_micro_usd = 0
"#;
        let t = PriceTable::from_toml_str(raw).unwrap();
        assert_eq!(t.pricing_version, "test-v1");
        assert_eq!(t.credit_cost(Tier::TeamPlan).unwrap(), 1234);
    }

    // ── Test 11: missing tier in custom table returns UnknownTier ─────
    #[test]
    fn price_table_missing_tier_returns_unknown_tier_error() {
        let raw = r#"
pricing_version = "test-v1"
effective_from  = "2026-06-01T00:00:00Z"
currency        = "USD"

[tiers.enterprise_byok]
credit_cost_micro_usd = 0
"#;
        let t = PriceTable::from_toml_str(raw).unwrap();
        let err = t.credit_cost(Tier::TeamPlan).unwrap_err();
        assert_eq!(err, MeterError::UnknownTier(Tier::TeamPlan));
    }

    // ── Test 12: determinism — same input, same output ────────────────
    #[test]
    fn credit_to_usd_micros_is_deterministic() {
        let r = rec(Tier::TeamPlan, 47);
        let t = embedded();
        let a = credit_to_usd_micros(&r, &t).unwrap();
        let b = credit_to_usd_micros(&r, &t).unwrap();
        assert_eq!(a, b, "credit_to_usd_micros must be pure / deterministic");
    }

    // ── Test 13: large legitimate amount survives ─────────────────────
    #[test]
    fn credit_to_usd_micros_large_legitimate_amount() {
        // 950 credits is the fixture's "large" team_plan row.
        // 950 × 20_526 = 19_499_700 micro-USD.
        let r = rec(Tier::TeamPlan, 950);
        let amt = credit_to_usd_micros(&r, &embedded()).unwrap();
        assert_eq!(amt, 19_499_700);
    }

    // ── Test 14: negative credits MeterError surfaces ─────────────────
    #[test]
    fn credit_to_usd_micros_negative_credits_returns_error() {
        // Defensive: even if fixture validation missed it, MeterError
        // surfaces (NegativeAmount).
        let r = rec(Tier::TeamPlan, -1);
        let err = credit_to_usd_micros(&r, &embedded()).unwrap_err();
        assert_eq!(err, MeterError::NegativeAmount);
    }

    // ── Test 15: pricing_version stamping invariant ───────────────────
    #[test]
    fn embedded_price_table_stamps_version_with_known_prefix() {
        let t = embedded();
        // Cross-language stability: version conforms to the
        // `manus-credit-v…` prefix family.
        assert!(
            t.pricing_version.starts_with("manus-credit-v"),
            "pricing_version must start with manus-credit-v…; got {}",
            t.pricing_version,
        );
    }
}
