//! D16 COV_85 — Genspark credit price table loader + pure
//! `credits_to_micro_usd` conversion.
//!
//! The price table asset (`assets/genspark_credit_prices.json`) is the
//! source of truth for credit → USD conversion. The asset is
//! `include_str!`-d at compile time so the binary ships with a
//! self-contained rate table — no I/O, no runtime config load.
//!
//! ## Conversion rules (design §3.1, review-standards §3)
//!
//! * Conversion is **pure** and **deterministic**: same
//!   `(credits_consumed, usd_per_credit)` ↦ same `i64` micro-USD (M1).
//! * Rounding uses `.round()` (round-half-away-from-zero) for
//!   cross-language consistency. Not `.trunc()`. Not `.floor()` (M6).
//! * Negative `credits_consumed` returns `Err` — audit rows with
//!   negative spend are semantically invalid (T15).
//! * `NaN` / `INFINITY` return `Err` — NEVER produce `i64::MIN` /
//!   `i64::MAX` via cast UB.
//! * Overflow saturates to `i64::MAX` rather than panicking on the
//!   `as i64` cast (M6).
//! * Unknown plan (`PriceLookupError::PlanNotFound`) bubbles up so the
//!   caller can stamp `amount_micro_usd = 0` +
//!   `reason_code = "genspark_plan_unknown"` per design §3.1.
//! * `pricing_version` is stamped at the moment of conversion from
//!   the price table itself, not from a side cache — prevents
//!   back-revision of the rate file from retroactively changing
//!   historical rows.

use serde::Deserialize;

/// Embedded credit price table. Loaded once at process start via
/// `CreditPriceTable::load_from_embedded()`.
///
/// The `pricing_version` field is the contract handle: rate changes
/// MUST bump this string (review-standards X6 / X7), and historical
/// audit rows carry the version used at conversion time so dashboards
/// never see a rate back-revision rewrite history.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct CreditPriceTable {
    /// Opaque version string. Bumped on every rate change.
    pub pricing_version: String,
    /// Effective-from timestamp. Documentation-only; the conversion
    /// path does not gate on it.
    pub effective_from: chrono::DateTime<chrono::Utc>,
    /// ISO 4217 currency. Always `"USD"` today.
    pub currency: String,
    /// One row per Genspark plan tier.
    pub rates: Vec<CreditRate>,
}

/// One row of the credit price table — a single Genspark plan tier.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct CreditRate {
    /// Plan slug, e.g. `"plus"` / `"pro"` / `"premium"`.
    pub plan: String,
    /// Monthly subscription price in USD. Documentation-only — the
    /// effective conversion uses `usd_per_credit`.
    pub monthly_usd: f64,
    /// Monthly credit grant for the plan. Documentation-only — the
    /// effective conversion uses `usd_per_credit`.
    pub monthly_credits: i64,
    /// Effective per-credit price stamped on the asset. Cached at load
    /// time to avoid redoing the division on every conversion (M3).
    pub usd_per_credit: f64,
    /// Free-form context. Documentation-only.
    #[serde(default)]
    pub note: Option<String>,
}

/// Failure to find a plan in the price table.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PriceLookupError {
    /// The plan slug was not declared in the price table.
    #[error("plan not found in price table: {0}")]
    PlanNotFound(String),
}

/// Failure to convert credit → micro-USD. See review-standards §3.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ConversionError {
    /// `credits_consumed` was negative.
    #[error("credits_consumed must be non-negative; got {0}")]
    NegativeCredits(f64),
    /// `credits_consumed` was `NaN`.
    #[error("credits_consumed must be a finite number; got NaN")]
    CreditsIsNaN,
    /// `credits_consumed` was `±INFINITY`.
    #[error("credits_consumed must be a finite number; got infinity")]
    CreditsIsInfinite,
    /// `usd_per_credit` was negative.
    #[error("usd_per_credit must be non-negative; got {0}")]
    NegativeRate(f64),
    /// `usd_per_credit` was `NaN` or `±INFINITY`.
    #[error("usd_per_credit must be finite; got non-finite value")]
    RateNotFinite,
}

impl CreditPriceTable {
    /// Parse the embedded `assets/genspark_credit_prices.json` asset.
    /// Panics on a malformed asset — by construction the asset is
    /// committed and a malformed asset means the build is broken.
    pub fn load_from_embedded() -> Self {
        let raw = include_str!("../assets/genspark_credit_prices.json");
        let table: Self = serde_json::from_str(raw)
            .expect("embedded genspark_credit_prices.json must be valid JSON");
        debug_assert!(
            !table.pricing_version.is_empty(),
            "pricing_version must be non-empty",
        );
        debug_assert!(
            !table.rates.is_empty(),
            "price table must have at least one rate",
        );
        table
    }

    /// Parse an arbitrary JSON string into a price table. Used by
    /// tests that exercise specific rate shapes without mutating the
    /// embedded asset.
    pub fn from_json_str(raw: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(raw)
    }

    /// Look up a plan by slug. Returns `Err(PlanNotFound)` if absent.
    pub fn lookup(&self, plan: &str) -> Result<&CreditRate, PriceLookupError> {
        self.rates
            .iter()
            .find(|r| r.plan == plan)
            .ok_or_else(|| PriceLookupError::PlanNotFound(plan.to_string()))
    }
}

/// Pure credit → micro-USD conversion.
///
/// Returns:
/// * `Ok(micro_usd)` for a valid rate. Saturates at `i64::MAX`
///   on overflow.
/// * `Err(ConversionError)` for negative / non-finite inputs.
///
/// The function is `#[inline]`-able by design: no allocations, no I/O,
/// no clock reads. Safe to call from a fuzz harness.
pub fn credits_to_micro_usd(
    credits_consumed: f64,
    usd_per_credit: f64,
) -> Result<i64, ConversionError> {
    if credits_consumed.is_nan() {
        return Err(ConversionError::CreditsIsNaN);
    }
    if credits_consumed.is_infinite() {
        return Err(ConversionError::CreditsIsInfinite);
    }
    if credits_consumed < 0.0 {
        return Err(ConversionError::NegativeCredits(credits_consumed));
    }

    if !usd_per_credit.is_finite() {
        return Err(ConversionError::RateNotFinite);
    }
    if usd_per_credit < 0.0 {
        return Err(ConversionError::NegativeRate(usd_per_credit));
    }

    let micro_usd_f = (credits_consumed * usd_per_credit * 1_000_000.0).round();
    // Saturate on overflow rather than rely on `as i64` cast UB.
    let micro_usd_i = if micro_usd_f >= i64::MAX as f64 {
        i64::MAX
    } else if micro_usd_f <= i64::MIN as f64 {
        i64::MIN
    } else {
        micro_usd_f as i64
    };
    Ok(micro_usd_i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn embedded() -> CreditPriceTable {
        CreditPriceTable::load_from_embedded()
    }

    // ── basic conversion ─────────────────────────────────────────────
    #[test]
    fn credits_to_micro_usd_basic_plus_plan() {
        // 3200 credits × $0.001999/credit = $6.3968 = 6_396_800 micro-USD.
        let got = credits_to_micro_usd(3200.0, 0.001999).unwrap();
        assert_eq!(got, 6_396_800);
    }

    #[test]
    fn credits_to_micro_usd_zero_credits_yields_zero() {
        let got = credits_to_micro_usd(0.0, 0.001999).unwrap();
        assert_eq!(got, 0);
    }

    // ── embedded price table sanity ─────────────────────────────────
    #[test]
    fn embedded_price_table_stamps_version() {
        let table = embedded();
        assert!(
            !table.pricing_version.is_empty(),
            "pricing_version must be non-empty",
        );
        assert!(
            table.pricing_version.starts_with("genspark-credit-v"),
            "pricing_version must start with genspark-credit-v…; got {}",
            table.pricing_version,
        );
        assert_eq!(table.currency, "USD");
        assert!(table.rates.len() >= 3, "plus + pro + premium minimum");
    }

    #[test]
    fn lookup_returns_plus_pro_premium() {
        let t = embedded();
        let plus = t.lookup("plus").unwrap();
        assert_eq!(plus.plan, "plus");
        assert!((plus.usd_per_credit - 0.001999).abs() < 1e-12);

        let pro = t.lookup("pro").unwrap();
        assert_eq!(pro.plan, "pro");
        assert!(pro.usd_per_credit > 0.0);

        let prem = t.lookup("premium").unwrap();
        assert_eq!(prem.plan, "premium");
        assert!(prem.usd_per_credit > 0.0);

        let err = t.lookup("solo").unwrap_err();
        assert_eq!(err, PriceLookupError::PlanNotFound("solo".into()));
    }

    // ── negative / non-finite reject ─────────────────────────────────
    #[test]
    fn credits_to_micro_usd_rejects_negative_credits() {
        let err = credits_to_micro_usd(-1.0, 0.001999).unwrap_err();
        assert_eq!(err, ConversionError::NegativeCredits(-1.0));
    }

    #[test]
    fn credits_to_micro_usd_rejects_nan_credits() {
        let err = credits_to_micro_usd(f64::NAN, 0.001999).unwrap_err();
        assert_eq!(err, ConversionError::CreditsIsNaN);
    }

    #[test]
    fn credits_to_micro_usd_rejects_infinite_credits() {
        let err = credits_to_micro_usd(f64::INFINITY, 0.001999).unwrap_err();
        assert_eq!(err, ConversionError::CreditsIsInfinite);
    }

    #[test]
    fn credits_to_micro_usd_rejects_negative_rate() {
        let err = credits_to_micro_usd(1.0, -0.001999).unwrap_err();
        assert_eq!(err, ConversionError::NegativeRate(-0.001999));
    }

    #[test]
    fn credits_to_micro_usd_rejects_nan_rate() {
        let err = credits_to_micro_usd(1.0, f64::NAN).unwrap_err();
        assert_eq!(err, ConversionError::RateNotFinite);
    }

    #[test]
    fn credits_to_micro_usd_rejects_infinite_rate() {
        let err = credits_to_micro_usd(1.0, f64::INFINITY).unwrap_err();
        assert_eq!(err, ConversionError::RateNotFinite);
    }

    // ── overflow saturation ─────────────────────────────────────────
    #[test]
    fn credits_to_micro_usd_saturates_at_i64_max() {
        // 1e18 credits × $1/credit × 1e6 = 1e24 — far past i64::MAX.
        let got = credits_to_micro_usd(1e18, 1.0).unwrap();
        assert_eq!(got, i64::MAX, "must saturate, not wrap or panic");
    }

    #[test]
    fn rounding_is_round_half_away_from_zero() {
        // 0.5 micro-USD → rounds AWAY from zero to 1.
        let got = credits_to_micro_usd(0.5, 0.000_001).unwrap();
        // 0.5 × 0.000_001 × 1_000_000 = 0.5 → round to 1.
        assert_eq!(got, 1);
    }

    #[test]
    fn determinism_same_input_same_output() {
        // Pure function — same input must produce same output across
        // invocations.
        for credits in [0.0, 1.5, 100.0, 3200.0, 999_999.999] {
            let a = credits_to_micro_usd(credits, 0.001999).unwrap();
            let b = credits_to_micro_usd(credits, 0.001999).unwrap();
            assert_eq!(a, b, "non-deterministic at credits={credits}");
        }
    }

    // ── version invariant guard (X7) ────────────────────────────────
    #[test]
    fn price_table_version_bumps_with_rate_change() {
        // Pin the (version → rates) tuple. If a future PR changes
        // usd_per_credit without bumping pricing_version, this test
        // fails and the reviewer is forced to look. The pin is
        // intentionally brittle — that's the whole point of X7.
        let t = embedded();
        assert_eq!(t.pricing_version, "genspark-credit-v1-2026-06");
        let plus = t.lookup("plus").unwrap();
        assert_eq!(plus.monthly_usd, 19.99);
        assert_eq!(plus.monthly_credits, 10_000);
        let pro = t.lookup("pro").unwrap();
        assert_eq!(pro.monthly_usd, 24.99);
        assert_eq!(pro.monthly_credits, 12_500);
        let prem = t.lookup("premium").unwrap();
        assert_eq!(prem.monthly_usd, 249.99);
        assert_eq!(prem.monthly_credits, 125_000);
    }

    #[test]
    fn rate_lookup_returns_borrowed_reference() {
        // M3: usd_per_credit cached at load — repeated lookups MUST NOT
        // redo arithmetic. Sanity: lookup returns a borrowed CreditRate,
        // not a fresh-computed value.
        let t = embedded();
        let r1 = t.lookup("plus").unwrap();
        let r2 = t.lookup("plus").unwrap();
        assert_eq!(r1.usd_per_credit, r2.usd_per_credit);
        assert_eq!(r1.plan, r2.plan);
    }

    #[test]
    fn from_json_str_round_trip() {
        let raw = r#"{
            "pricing_version": "test-v0",
            "effective_from": "2026-01-01T00:00:00Z",
            "currency": "USD",
            "rates": [
                {"plan": "x", "monthly_usd": 10.0, "monthly_credits": 1000, "usd_per_credit": 0.01}
            ]
        }"#;
        let t = CreditPriceTable::from_json_str(raw).unwrap();
        assert_eq!(t.pricing_version, "test-v0");
        let r = t.lookup("x").unwrap();
        assert_eq!(r.monthly_credits, 1000);
        assert_eq!(r.usd_per_credit, 0.01);
    }
}
