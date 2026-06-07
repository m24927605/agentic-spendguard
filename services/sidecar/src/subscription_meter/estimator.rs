//! D13 COV_62 — Meter-only estimate.
//!
//! Computes a best-effort retail-USD estimate for a subscription
//! request and (optionally) increments the per-tenant
//! `consumed_atomic` counter. NEVER opens a ledger transaction.
//!
//! `meter_only_estimate` is an advisory path: it returns the snapshot
//! the adapter should display / log, plus the cap evaluation outcome.
//! See `hard_cap::evaluate_cap` for the threshold logic.
//!
//! Spec: docs/specs/coverage/D13_subscription_meter/design.md §4.2

use chrono::{DateTime, Datelike, TimeZone, Utc};

use super::classifier::SubscriptionKind;

/// Output of the meter-only estimate.
///
/// All amount fields are in **micro-USD** (atomic, integer math) to
/// stay consistent with the ledger's atomic accounting. Conversion to
/// USD is `amount / 1_000_000`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeterEstimate {
    /// Tenant whose meter was updated.
    pub tenant_id: String,
    /// Subscription kind as classified.
    pub kind: SubscriptionKind,
    /// Plan label written to subscription_meters.plan.
    pub plan: String,
    /// Period boundary (UTC calendar month).
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    /// Estimated retail $ for THIS request, micro-USD.
    pub estimated_amount_atomic: i64,
    /// Whether this call advanced the consumed_atomic counter; false
    /// in pure shadow modes.
    pub increment_applied: bool,
}

impl MeterEstimate {
    pub fn period_label(&self) -> String {
        format!(
            "{}-{:02}",
            self.period_start.year(),
            self.period_start.month()
        )
    }

    pub fn estimated_usd(&self) -> f64 {
        self.estimated_amount_atomic as f64 / 1_000_000.0
    }
}

/// Compute the meter-only estimate. Pure function — does NOT touch
/// the database. The caller (typically `decision::transaction::
/// run_through_reserve`) is responsible for persisting the
/// `consumed_atomic` increment via `SubscriptionMeterStore`.
///
/// `input_tokens` and `predicted_output_tokens` come from the
/// ClaimEstimate that the egress proxy / SDK attached to the
/// DecisionRequest.  Retail prices are denominated in **micro-USD per
/// 1M tokens** (matching ledger pricing_table units).
pub fn meter_only_estimate(
    tenant_id: &str,
    kind: SubscriptionKind,
    input_tokens: i64,
    predicted_output_tokens: i64,
    retail_input_price_micro_per_million: i64,
    retail_output_price_micro_per_million: i64,
    now: DateTime<Utc>,
) -> MeterEstimate {
    let (period_start, period_end) = utc_calendar_month_bounds(now);

    // Saturating math: clamp negative inputs to 0 (defensive — the
    // tokenizer should never return negatives, but the meter must
    // never go below zero).
    let in_tok = input_tokens.max(0);
    let out_tok = predicted_output_tokens.max(0);
    let in_price = retail_input_price_micro_per_million.max(0);
    let out_price = retail_output_price_micro_per_million.max(0);

    // Integer micro-USD math: (tokens * price_per_million) / 1_000_000.
    let in_amt = in_tok.saturating_mul(in_price) / 1_000_000;
    let out_amt = out_tok.saturating_mul(out_price) / 1_000_000;
    let amount_atomic = in_amt.saturating_add(out_amt);

    MeterEstimate {
        tenant_id: tenant_id.to_string(),
        kind,
        plan: kind.as_str().to_string(),
        period_start,
        period_end,
        estimated_amount_atomic: amount_atomic,
        // Increment is applied unless the caller explicitly overrides
        // (e.g. shadow / dry-run mode). Default = true.
        increment_applied: true,
    }
}

/// UTC calendar month bounds for the given moment.
///
/// Returned tuple is `(period_start, period_end_exclusive)` where
/// period_start is the first instant of the current month and
/// period_end is the first instant of the next month.
pub fn utc_calendar_month_bounds(now: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    let year = now.year();
    let month = now.month();
    let period_start = Utc
        .with_ymd_and_hms(year, month, 1, 0, 0, 0)
        .single()
        .expect("calendar month start always valid");
    let (ny, nm) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let period_end = Utc
        .with_ymd_and_hms(ny, nm, 1, 0, 0, 0)
        .single()
        .expect("calendar month end always valid");
    (period_start, period_end)
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn t(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    #[test]
    fn calendar_month_bounds_january_2026() {
        let (start, end) = utc_calendar_month_bounds(t(2026, 1, 15, 13));
        assert_eq!(start, t(2026, 1, 1, 0));
        assert_eq!(end, t(2026, 2, 1, 0));
    }

    #[test]
    fn calendar_month_bounds_december_rolls_year() {
        let (start, end) = utc_calendar_month_bounds(t(2026, 12, 31, 23));
        assert_eq!(start, t(2026, 12, 1, 0));
        assert_eq!(end, t(2027, 1, 1, 0));
    }

    #[test]
    fn estimate_atomic_amount_is_integer_micro_usd() {
        // Claude 3.5 Sonnet retail: $3 input, $15 output per 1M.
        // 100 input + 200 output tokens.
        // = 100 * 3_000_000 / 1_000_000 + 200 * 15_000_000 / 1_000_000
        // = 300 + 3000 = 3300 micro-USD = $0.0033
        let est = meter_only_estimate(
            "tenant-1",
            SubscriptionKind::ClaudeCodePro,
            100,
            200,
            3_000_000,
            15_000_000,
            t(2026, 6, 7, 12),
        );
        assert_eq!(est.estimated_amount_atomic, 3_300);
        assert!((est.estimated_usd() - 0.0033).abs() < 1e-9);
    }

    #[test]
    fn estimate_handles_zero_tokens() {
        let est = meter_only_estimate(
            "t",
            SubscriptionKind::CodexChatGpt,
            0,
            0,
            1_000_000,
            1_000_000,
            t(2026, 6, 7, 12),
        );
        assert_eq!(est.estimated_amount_atomic, 0);
    }

    #[test]
    fn estimate_clamps_negative_inputs_to_zero() {
        let est = meter_only_estimate(
            "t",
            SubscriptionKind::ClaudeCodePro,
            -10,
            -10,
            1_000_000,
            1_000_000,
            t(2026, 6, 7, 12),
        );
        assert_eq!(est.estimated_amount_atomic, 0);
    }

    #[test]
    fn period_label_formats_year_month() {
        let est = meter_only_estimate(
            "t",
            SubscriptionKind::ClaudeCodePro,
            0,
            0,
            0,
            0,
            t(2026, 6, 7, 12),
        );
        assert_eq!(est.period_label(), "2026-06");
    }

    #[test]
    fn plan_label_matches_kind() {
        let est = meter_only_estimate(
            "t",
            SubscriptionKind::CodexChatGpt,
            0,
            0,
            0,
            0,
            t(2026, 6, 7, 12),
        );
        assert_eq!(est.plan, "codex_chatgpt");
    }
}
