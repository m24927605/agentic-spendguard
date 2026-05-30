//! Signal 2 — per-step dynamic re-projection + drift detection.
//!
//! Spec ref `run-cost-projector-spec-v1alpha1.md` §4.
//!
//! ## Mechanism (per spec §4.1)
//!
//! Every Project call recomputes:
//!
//! ```text
//! cumulative_cost     = sum(per_step_costs)            // from state cache
//! this_call           = req.this_call_reservation_atomic
//! predicted_remaining = predicted_remaining_steps × per_step_cost_baseline
//! projection          = cumulative_cost + this_call + predicted_remaining
//! ```
//!
//! Signal 1 sets `predicted_remaining_steps`; Signal 2 contributes by
//! refreshing `per_step_cost_baseline` AND by tracking drift.
//!
//! ## Per-step baseline
//!
//! Spec §3.1 formula `strategy_b_per_call(tenant, model, agent, class)` is
//! the per-call B estimator from output_predictor. For SLICE_09 we don't
//! re-query output_predictor inside the projector — that would balloon the
//! 5ms p99 budget. Instead, the sidecar (Phase E) passes the per-call
//! `this_call_reservation_atomic` and Signal 2 uses THAT as the baseline
//! for predicted-remaining. This is a conservative approximation: if the
//! most-recent call was atypically expensive, predicted-remaining inflates
//! — but per-step variance is what drift detection is meant to catch,
//! and predicted-remaining should reflect the current trajectory.
//!
//! ## Drift detection (per spec §4.2)
//!
//! ```text
//! ratio_now  = predicted_remaining_cost_now / predicted_remaining_cost_prior_step
//! IF |ratio_now - 1.0| > drift_ratio_threshold
//!    AND happened drift_consecutive_threshold times in a row:
//!     emit RUN_DRIFT_DETECTED
//! ```
//!
//! The `drift_ratio_threshold` defaults to 0.5 (50% jump per step). The
//! `drift_consecutive_threshold` defaults to 3 steps (suppresses noise).
//! Both are tunable per spec §4.2.

/// Result of Signal 2's drift evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftVerdict {
    /// No prior step to compare against (first call in run) OR ratio
    /// within tolerance → no drift signal this step.
    NoDrift,
    /// Ratio out of tolerance THIS step, but consecutive counter has not
    /// crossed the threshold yet. Caller bumps the counter on RunState.
    Suspect,
    /// Ratio out of tolerance for `drift_consecutive_threshold` consecutive
    /// steps → emit RUN_DRIFT_DETECTED.
    Confirmed,
}

/// Evaluate drift for THIS step.
///
/// Returns the verdict AND the new consecutive-count value (which the
/// caller writes back to RunState before saving).
pub fn evaluate_drift(
    predicted_remaining_cost_now: i64,
    last_predicted_remaining_cost: Option<i64>,
    drift_consecutive_count: u32,
    drift_ratio_threshold: f64,
    drift_consecutive_threshold: u32,
) -> (DriftVerdict, u32) {
    let Some(prior) = last_predicted_remaining_cost else {
        // First call — no baseline to compare against.
        return (DriftVerdict::NoDrift, 0);
    };
    if prior <= 0 {
        // Zero prior is degenerate; reset.
        return (DriftVerdict::NoDrift, 0);
    }
    let ratio = predicted_remaining_cost_now as f64 / prior as f64;
    let delta = (ratio - 1.0).abs();
    if delta > drift_ratio_threshold {
        let new_count = drift_consecutive_count.saturating_add(1);
        if new_count >= drift_consecutive_threshold {
            (DriftVerdict::Confirmed, new_count)
        } else {
            (DriftVerdict::Suspect, new_count)
        }
    } else {
        // Within tolerance — reset counter.
        (DriftVerdict::NoDrift, 0)
    }
}

/// Compute the layered projection per spec §6 step 4.
///
/// `cumulative` is the sum of prior step reservations from RunState.
/// `this_call` is the current call's reservation (output_predictor).
/// `remaining_steps × per_step_baseline` is the forward-looking estimate.
///
/// Returns the projection atomic-micros. i64-saturating arithmetic prevents
/// overflow on degenerate inputs (e.g. 9223372036854775807 + 1 stays at
/// i64::MAX rather than wrapping negative, which would pass the
/// `projection > budget_remaining` test trivially).
pub fn compute_projection(cumulative: i64, this_call: i64, remaining_cost: i64) -> i64 {
    cumulative
        .saturating_add(this_call)
        .saturating_add(remaining_cost)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_call_no_drift() {
        let (verdict, new_count) = evaluate_drift(100, None, 0, 0.5, 3);
        assert_eq!(verdict, DriftVerdict::NoDrift);
        assert_eq!(new_count, 0);
    }

    #[test]
    fn within_tolerance_resets_counter() {
        // Ratio 100/100 = 1.0, delta = 0 — within tolerance even with
        // an active counter (suppression of false positives).
        let (verdict, new_count) = evaluate_drift(100, Some(100), 2, 0.5, 3);
        assert_eq!(verdict, DriftVerdict::NoDrift);
        assert_eq!(new_count, 0, "counter should reset on tolerance");
    }

    #[test]
    fn out_of_tolerance_increments_counter_to_suspect() {
        // Ratio 200/100 = 2.0, delta = 1.0 > 0.5.
        let (verdict, new_count) = evaluate_drift(200, Some(100), 0, 0.5, 3);
        assert_eq!(verdict, DriftVerdict::Suspect);
        assert_eq!(new_count, 1);
    }

    #[test]
    fn three_consecutive_confirms_drift() {
        // Step 1: 200 vs 100 → Suspect, count=1
        let (v1, c1) = evaluate_drift(200, Some(100), 0, 0.5, 3);
        assert_eq!(v1, DriftVerdict::Suspect);
        // Step 2: 200 vs 100 → Suspect, count=2
        let (v2, c2) = evaluate_drift(200, Some(100), c1, 0.5, 3);
        assert_eq!(v2, DriftVerdict::Suspect);
        // Step 3: 200 vs 100 → Confirmed, count=3
        let (v3, c3) = evaluate_drift(200, Some(100), c2, 0.5, 3);
        assert_eq!(v3, DriftVerdict::Confirmed);
        assert_eq!(c3, 3);
    }

    #[test]
    fn zero_prior_resets_counter() {
        let (verdict, new_count) = evaluate_drift(100, Some(0), 5, 0.5, 3);
        assert_eq!(verdict, DriftVerdict::NoDrift);
        assert_eq!(new_count, 0);
    }

    #[test]
    fn negative_prior_resets_counter() {
        // Defense-in-depth: prior should never be negative but i64 wire
        // permits it. Should not divide-by-zero or surface as a Suspect.
        let (verdict, new_count) = evaluate_drift(100, Some(-1), 0, 0.5, 3);
        assert_eq!(verdict, DriftVerdict::NoDrift);
        assert_eq!(new_count, 0);
    }

    #[test]
    fn ratio_downward_drift_also_triggers() {
        // Ratio 10/100 = 0.1, delta = 0.9 > 0.5. Spec §4.2 talks about
        // "上升" but the abs-delta formula catches both directions —
        // a sudden cost collapse is also drift worth surfacing.
        let (verdict, _) = evaluate_drift(10, Some(100), 0, 0.5, 3);
        assert_eq!(verdict, DriftVerdict::Suspect);
    }

    #[test]
    fn compute_projection_saturates_on_overflow() {
        let max = i64::MAX;
        // max + 1 + 1 should saturate at max, not wrap negative.
        let proj = compute_projection(max, 1, 1);
        assert_eq!(proj, i64::MAX);
    }

    #[test]
    fn compute_projection_basic_sum() {
        // Sanity: 100 + 50 + 200 = 350.
        let proj = compute_projection(100, 50, 200);
        assert_eq!(proj, 350);
    }
}
