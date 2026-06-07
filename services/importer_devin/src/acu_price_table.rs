//! D14 COV_68 — ACU price table loader + pure `acu_to_micro_usd`
//! conversion.
//!
//! The price table asset (`assets/devin_acu_prices.json`) is the source
//! of truth for ACU → USD conversion. The asset is `include_str!`-d
//! at compile time so the binary ships with a self-contained rate
//! table — no I/O, no runtime config load.
//!
//! ## Conversion rules (design §4.2, review-standards §3)
//!
//! * Conversion is **pure** and **deterministic**: same
//!   `(acu_consumed, usd_per_acu)` ↦ same `i64` micro-USD (C1).
//! * Rounding uses `.round()` (round-half-away-from-zero) for
//!   cross-language consistency. Not `.trunc()`. Not `.floor()` (C2).
//! * Negative `acu_consumed` returns `Err` — audit rows with negative
//!   spend are semantically invalid (C3).
//! * `NaN` / `INFINITY` return `Err` — NEVER produce `i64::MIN` /
//!   `i64::MAX` via cast UB (C4).
//! * Overflow saturates to `i64::MAX` rather than panicking on the
//!   `as i64` cast (C5).
//! * Enterprise plan with `usd_per_acu = None` returns
//!   `Ok(None)` from `acu_to_micro_usd` — the caller stamps
//!   `reason_code = "devin_enterprise_negotiated_rate"` (C6).
//! * `pricing_version` is stamped at the moment of conversion from
//!   the price table itself, not from a side cache — prevents
//!   back-revision of the rate file from retroactively changing
//!   historical rows (C7).

use serde::Deserialize;

/// Embedded ACU price table. Loaded once at process start via
/// `AcuPriceTable::load_from_embedded()`.
///
/// The `pricing_version` field is the contract handle: rate changes
/// MUST bump this string (review-standards T10), and historical audit
/// rows carry the version used at conversion time so dashboards never
/// see a rate back-revision rewrite history.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct AcuPriceTable {
    /// Opaque version string. Bumped on every rate change.
    pub pricing_version: String,
    /// Effective-from timestamp. Documentation-only; the conversion
    /// path does not gate on it.
    pub effective_from: chrono::DateTime<chrono::Utc>,
    /// ISO 4217 currency. Always `"USD"` today.
    pub currency: String,
    /// One row per Devin plan tier.
    pub rates: Vec<AcuRate>,
}

/// One row of the ACU price table — a single Devin plan tier.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct AcuRate {
    /// Plan slug, e.g. `"team"` / `"enterprise"`.
    pub plan: String,
    /// `Some(rate)` for public plans; `None` for enterprise / NDA
    /// negotiated rates. The importer emits
    /// `amount_micro_usd = NULL` + `reason_code =
    /// "devin_enterprise_negotiated_rate"` for the latter.
    pub usd_per_acu: Option<f64>,
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

/// Failure to convert ACU → micro-USD. See review-standards §3 (C1-C5).
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ConversionError {
    /// `acu_consumed` was negative.
    #[error("acu_consumed must be non-negative; got {0}")]
    NegativeAcu(f64),
    /// `acu_consumed` was `NaN`.
    #[error("acu_consumed must be a finite number; got NaN")]
    AcuIsNaN,
    /// `acu_consumed` was `±INFINITY`.
    #[error("acu_consumed must be a finite number; got infinity")]
    AcuIsInfinite,
    /// `usd_per_acu` was negative.
    #[error("usd_per_acu must be non-negative; got {0}")]
    NegativeRate(f64),
    /// `usd_per_acu` was `NaN` or `±INFINITY`.
    #[error("usd_per_acu must be finite; got non-finite value")]
    RateNotFinite,
}

impl AcuPriceTable {
    /// Parse the embedded `assets/devin_acu_prices.json` asset. Panics
    /// on a malformed asset — by construction the asset is committed
    /// and a malformed asset means the build is broken.
    pub fn load_from_embedded() -> Self {
        let raw = include_str!("../assets/devin_acu_prices.json");
        let table: Self =
            serde_json::from_str(raw).expect("embedded devin_acu_prices.json must be valid JSON");
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
    pub fn lookup(&self, plan: &str) -> Result<&AcuRate, PriceLookupError> {
        self.rates
            .iter()
            .find(|r| r.plan == plan)
            .ok_or_else(|| PriceLookupError::PlanNotFound(plan.to_string()))
    }
}

/// Pure ACU → micro-USD conversion.
///
/// Returns:
/// * `Ok(Some(micro_usd))` for a valid rate. Saturates at `i64::MAX`
///   on overflow (C5).
/// * `Ok(None)` when `usd_per_acu` is `None` (enterprise negotiated
///   rate) — the caller emits `reason_code =
///   "devin_enterprise_negotiated_rate"` and `amount_micro_usd = NULL`
///   on the audit row (C6).
/// * `Err(ConversionError)` for negative / non-finite inputs (C3-C4).
///
/// The function is `#[inline]`-able by design: no allocations, no I/O,
/// no clock reads. Safe to call from a fuzz harness.
pub fn acu_to_micro_usd(
    acu_consumed: f64,
    usd_per_acu: Option<f64>,
) -> Result<Option<i64>, ConversionError> {
    if acu_consumed.is_nan() {
        return Err(ConversionError::AcuIsNaN);
    }
    if acu_consumed.is_infinite() {
        return Err(ConversionError::AcuIsInfinite);
    }
    if acu_consumed < 0.0 {
        return Err(ConversionError::NegativeAcu(acu_consumed));
    }

    let Some(rate) = usd_per_acu else {
        // Enterprise negotiated rate — caller stamps reason_code.
        return Ok(None);
    };
    if !rate.is_finite() {
        return Err(ConversionError::RateNotFinite);
    }
    if rate < 0.0 {
        return Err(ConversionError::NegativeRate(rate));
    }

    let micro_usd_f = (acu_consumed * rate * 1_000_000.0).round();
    // C5: saturate on overflow rather than rely on `as i64` cast UB.
    // Pre-flight against the i64 boundary in f64 space (i64::MAX
    // exactly representable as f64 = 9223372036854775807.0).
    let micro_usd_i = if micro_usd_f >= i64::MAX as f64 {
        i64::MAX
    } else if micro_usd_f <= i64::MIN as f64 {
        i64::MIN
    } else {
        // Safe because we've bounded the f64 within i64 range above.
        micro_usd_f as i64
    };
    Ok(Some(micro_usd_i))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn embedded() -> AcuPriceTable {
        AcuPriceTable::load_from_embedded()
    }

    // ── Test 1/6: round-trip basic conversion ────────────────────────
    #[test]
    fn acu_to_micro_usd_basic_team_plan() {
        // 12.5 ACU × $2.25/ACU = $28.125 = 28_125_000 micro-USD.
        // This is the headline gate A10.3.
        let got = acu_to_micro_usd(12.5, Some(2.25)).unwrap();
        assert_eq!(got, Some(28_125_000));
    }

    // ── Test 2/6: enterprise NULL passthrough ────────────────────────
    #[test]
    fn acu_to_micro_usd_enterprise_returns_none() {
        let got = acu_to_micro_usd(100.0, None).unwrap();
        assert_eq!(got, None, "enterprise rate must yield None");
    }

    // ── Test 3/6: pricing_version stamping ───────────────────────────
    #[test]
    fn embedded_price_table_stamps_version() {
        let table = embedded();
        assert!(
            !table.pricing_version.is_empty(),
            "pricing_version must be non-empty",
        );
        // Cross-language stability: version is opaque but conforms to
        // a known prefix family `devin-acu-v*`.
        assert!(
            table.pricing_version.starts_with("devin-acu-v"),
            "pricing_version must start with devin-acu-v…; got {}",
            table.pricing_version,
        );
        assert_eq!(table.currency, "USD");
        assert!(table.rates.len() >= 2, "team + enterprise minimum");
    }

    // ── Test 4/6: lookup happy path + miss ───────────────────────────
    #[test]
    fn lookup_returns_team_and_enterprise() {
        let t = embedded();
        let team = t.lookup("team").unwrap();
        assert_eq!(team.plan, "team");
        assert_eq!(team.usd_per_acu, Some(2.25));

        let ent = t.lookup("enterprise").unwrap();
        assert_eq!(ent.plan, "enterprise");
        assert_eq!(ent.usd_per_acu, None);

        let err = t.lookup("solo").unwrap_err();
        assert_eq!(err, PriceLookupError::PlanNotFound("solo".into()));
    }

    // ── Test 5/6: negative / non-finite reject ───────────────────────
    #[test]
    fn acu_to_micro_usd_rejects_negative_acu() {
        let err = acu_to_micro_usd(-1.0, Some(2.25)).unwrap_err();
        assert_eq!(err, ConversionError::NegativeAcu(-1.0));
    }

    #[test]
    fn acu_to_micro_usd_rejects_nan() {
        let err = acu_to_micro_usd(f64::NAN, Some(2.25)).unwrap_err();
        assert_eq!(err, ConversionError::AcuIsNaN);
    }

    #[test]
    fn acu_to_micro_usd_rejects_infinity() {
        let err = acu_to_micro_usd(f64::INFINITY, Some(2.25)).unwrap_err();
        assert_eq!(err, ConversionError::AcuIsInfinite);
    }

    #[test]
    fn acu_to_micro_usd_rejects_negative_rate() {
        let err = acu_to_micro_usd(1.0, Some(-2.25)).unwrap_err();
        assert_eq!(err, ConversionError::NegativeRate(-2.25));
    }

    #[test]
    fn acu_to_micro_usd_rejects_non_finite_rate() {
        let err = acu_to_micro_usd(1.0, Some(f64::NAN)).unwrap_err();
        assert_eq!(err, ConversionError::RateNotFinite);
        let err = acu_to_micro_usd(1.0, Some(f64::INFINITY)).unwrap_err();
        assert_eq!(err, ConversionError::RateNotFinite);
    }

    // ── Test 6/6: overflow saturation (C5) ───────────────────────────
    #[test]
    fn acu_to_micro_usd_saturates_at_i64_max() {
        // 1e18 ACU × $1 × 1e6 = 1e24 — far past i64::MAX (≈ 9.2e18).
        let got = acu_to_micro_usd(1e18, Some(1.0)).unwrap();
        assert_eq!(got, Some(i64::MAX), "must saturate, not wrap or panic");
    }

    #[test]
    fn acu_to_micro_usd_zero_acu_yields_zero_micro() {
        let got = acu_to_micro_usd(0.0, Some(2.25)).unwrap();
        assert_eq!(got, Some(0));
    }

    #[test]
    fn rounding_is_round_half_away_from_zero() {
        // 0.000_000_5 USD = 0.5 micro-USD → rounds AWAY from zero to 1.
        // (C2: NOT trunc, NOT floor.)
        let got = acu_to_micro_usd(0.5, Some(0.000_001)).unwrap();
        // 0.5 × 0.000_001 × 1_000_000 = 0.5 → round to 1.
        assert_eq!(got, Some(1));
    }

    #[test]
    fn determinism_same_input_same_output() {
        // C1: pure function — same input must produce same output
        // across invocations.
        for acu in [0.0, 1.5, 12.5, 100.0, 999_999.999] {
            let a = acu_to_micro_usd(acu, Some(2.25)).unwrap();
            let b = acu_to_micro_usd(acu, Some(2.25)).unwrap();
            assert_eq!(a, b, "non-deterministic at acu={acu}");
        }
    }

    // ── version invariant guard (T10) ────────────────────────────────
    #[test]
    fn price_table_version_bumps_with_rate_change() {
        // Pin the (version → rates) tuple. If a future PR changes
        // usd_per_acu without bumping pricing_version, this test fails
        // and the reviewer is forced to look. The pin is intentionally
        // brittle — that's the whole point of T10.
        let t = embedded();
        assert_eq!(t.pricing_version, "devin-acu-v1-2026-06");
        assert_eq!(t.lookup("team").unwrap().usd_per_acu, Some(2.25));
        assert_eq!(t.lookup("enterprise").unwrap().usd_per_acu, None);
    }
}
