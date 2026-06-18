//! Signal layering + RUN_* code precedence.
//!
//! Spec ref `run-cost-projector-spec-v1alpha1.md` §6.
//!
//! ## Layering recipe (spec §6)
//!
//! 1. Signal 1: cold-start fallback OR historical P95.
//! 2. Signal 3 override: when hint > 0, override Signal 1's predicted_steps.
//! 3. Signal 2: always-on per-step drift detection.
//! 4. Compute projection = cumulative + this_call + (remaining_steps × baseline).
//! 5. Code precedence: BUDGET > STEPS > DRIFT.
//!
//! ## Code precedence justification (spec §6.1)
//!
//! - BUDGET > STEPS: budget exhaustion has direct financial impact —
//!   surfacing the budget signal first prevents the operator from
//!   misreading a steps-exceeded as "still within budget, just chatty".
//! - STEPS > DRIFT: steps-exceeded already implies drift in cost
//!   trajectory; emitting both is redundant. SLICE_09 emits exactly one
//!   code per Project response, but `considered_codes` preserves the full
//!   evidence array for SIEM / dashboard filters.
//!
//! ## Strategy diagnostic (`StrategyUsed`)
//!
//! Captures which signals materialized for this projection so audit
//! consumers (calibration-report, dashboard breakdown) can attribute
//! cause without re-running the layering logic from raw inputs.

use crate::proto::run_cost_projector::v1::StrategyUsed;

/// All three RUN_* codes that the projector may emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunCode {
    BudgetProjectionExceeded,
    StepsExceeded,
    DriftDetected,
}

impl RunCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BudgetProjectionExceeded => "RUN_BUDGET_PROJECTION_EXCEEDED",
            Self::StepsExceeded => "RUN_STEPS_EXCEEDED",
            Self::DriftDetected => "RUN_DRIFT_DETECTED",
        }
    }
}

/// Aggregated result of the layering pipeline.
#[derive(Debug, Clone)]
pub struct LayeringResult {
    /// Final projection_at_decision_atomic (cumulative + this_call + predicted_remaining).
    pub projection_atomic: i64,
    /// Predicted remaining steps after this call (post Signal 1 + Signal 3 override).
    pub predicted_remaining_steps: i32,
    /// Steps already completed in this run (echoed for audit).
    pub steps_completed_so_far: i64,
    /// Predicted remaining cost — used by Signal 2 drift comparison on
    /// the NEXT Project call (caller writes to RunState before saving).
    pub predicted_remaining_cost_atomic: i64,
    /// Which signals contributed (diagnostic only).
    pub strategy_used: StrategyUsed,
    /// The code that wins precedence (Some) or None if no code triggered.
    pub emitted_code: Option<RunCode>,
    /// All codes that fired (could be subset of {BUDGET, STEPS, DRIFT})
    /// including the winner. For SIEM forensics + audit row reason_codes.
    pub considered_codes: Vec<RunCode>,
}

/// Inputs to the layering compute. Caller assembles from request +
/// state cache + signal outputs.
#[derive(Debug, Clone)]
pub struct LayeringInputs {
    pub cumulative_cost_atomic: i64,
    pub this_call_reservation_atomic: i64,
    pub steps_completed: i64,
    pub budget_remaining_atomic: i64,

    /// Signal 1's output (P95-based or cold-start). i32 mirror per
    /// spec §3.1 + audit-extension §2.2.
    pub signal1_predicted_remaining_steps: i32,
    /// Whether Signal 1 fell back to cold-start (drives `strategy_used`).
    pub signal1_is_cold_start: bool,

    /// Signal 3 hint (0 = inactive).
    pub planned_steps_hint: i32,

    /// Drift detection verdict from Signal 2.
    pub drift_confirmed: bool,

    /// Per-step baseline cost. Spec §4.1 formula uses
    /// `strategy_b_per_call`; SLICE_09 conservatively uses
    /// `this_call_reservation_atomic` as the baseline (the most-recent
    /// per-call cost).
    pub per_step_baseline_atomic: i64,
}

/// Run the §6 layering pipeline end-to-end. Pure function — caller owns
/// state mutation (record_step, write drift counter, etc.).
pub fn compute_layering(inputs: &LayeringInputs) -> LayeringResult {
    // Step 2: Signal 3 override.
    let signal3_active = inputs.planned_steps_hint > 0;
    let predicted_remaining_steps = if signal3_active {
        crate::signal_3::apply_signal3_override(
            inputs.signal1_predicted_remaining_steps,
            inputs.planned_steps_hint,
            inputs.steps_completed,
        )
    } else {
        inputs.signal1_predicted_remaining_steps
    };

    // Step 3 + 4: project.
    let predicted_remaining_cost_atomic =
        (predicted_remaining_steps as i64).saturating_mul(inputs.per_step_baseline_atomic.max(0));
    let projection_atomic = crate::signal_2::compute_projection(
        inputs.cumulative_cost_atomic,
        inputs.this_call_reservation_atomic,
        predicted_remaining_cost_atomic,
    );
    let future_commitment_atomic = inputs
        .this_call_reservation_atomic
        .saturating_add(predicted_remaining_cost_atomic);

    // Step 5: code detection.
    //
    // BUDGET gate invariant (spec §4.1/§6 reconciled; contract-dsl-spec
    // -v1alpha2.md §3.1): the gate compares `this_call_reservation +
    // predicted_remaining_cost` (== `future_commitment_atomic`) against
    // `budget_remaining_atomic`, which is the LIVE ledger AVAILABLE balance
    // (sidecar sources it from ledger.QueryBudget `available_atomic`, already
    // NET of prior reservations / cumulative spend). cumulative_cost_atomic is
    // therefore DELIBERATELY excluded from this comparison — re-adding it would
    // double-count prior spend already netted out of `budget_remaining_atomic`
    // and produce false RUN_BUDGET_PROJECTION_EXCEEDED stops. cumulative is
    // still folded into the REPORTED `projection_atomic` diagnostic above.
    // Pinned by `live_available_budget_does_not_double_count_prior_spend` and
    // `budget_gate_excludes_cumulative_invariant`.
    //
    // RUN_BUDGET_PROJECTION_EXCEEDED is an ADVISORY projection code; the ledger
    // reserve path remains the hard money-stop oracle (single_writer_per_budget
    // atomic claim), so this advisory gate never substitutes for that fence.
    let mut considered = Vec::with_capacity(3);
    if future_commitment_atomic > inputs.budget_remaining_atomic {
        considered.push(RunCode::BudgetProjectionExceeded);
    }
    if signal3_active
        && crate::signal_3::steps_exceeded(inputs.planned_steps_hint, inputs.steps_completed)
    {
        considered.push(RunCode::StepsExceeded);
    }
    if inputs.drift_confirmed {
        considered.push(RunCode::DriftDetected);
    }

    // Code precedence per spec §6.1.
    let emitted_code = if considered.contains(&RunCode::BudgetProjectionExceeded) {
        Some(RunCode::BudgetProjectionExceeded)
    } else if considered.contains(&RunCode::StepsExceeded) {
        Some(RunCode::StepsExceeded)
    } else if considered.contains(&RunCode::DriftDetected) {
        Some(RunCode::DriftDetected)
    } else {
        None
    };

    // Diagnostic strategy_used.
    let strategy_used = match (
        signal3_active,
        inputs.signal1_is_cold_start,
        inputs.drift_confirmed,
    ) {
        (true, true, _) => StrategyUsed::S3,
        (true, false, true) => StrategyUsed::S1s2s3,
        (true, false, false) => StrategyUsed::S1s2s3, // S3 + S1 historical; S2 always-on so S1S2S3
        (false, true, _) => StrategyUsed::ColdStart,
        (false, false, true) => StrategyUsed::S1s2,
        (false, false, false) => StrategyUsed::S1s2, // S1 historical + S2 always-on, drift quiet
    };

    LayeringResult {
        projection_atomic,
        predicted_remaining_steps,
        steps_completed_so_far: inputs.steps_completed,
        predicted_remaining_cost_atomic,
        strategy_used,
        emitted_code,
        considered_codes: considered,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn baseline_inputs() -> LayeringInputs {
        LayeringInputs {
            cumulative_cost_atomic: 0,
            this_call_reservation_atomic: 100,
            steps_completed: 0,
            budget_remaining_atomic: 1_000_000,
            signal1_predicted_remaining_steps: 10,
            signal1_is_cold_start: false,
            planned_steps_hint: 0,
            drift_confirmed: false,
            per_step_baseline_atomic: 100,
        }
    }

    #[test]
    fn happy_path_no_codes_emitted() {
        // projection = 0 + 100 + (10 × 100) = 1100; budget = 1_000_000 → no code.
        let r = compute_layering(&baseline_inputs());
        assert_eq!(r.projection_atomic, 1100);
        assert_eq!(r.predicted_remaining_steps, 10);
        assert_eq!(r.emitted_code, None);
        assert!(r.considered_codes.is_empty());
    }

    #[test]
    fn budget_projection_exceeded_fires() {
        let mut i = baseline_inputs();
        i.budget_remaining_atomic = 500; // future 100 + 1000 > remaining 500.
        let r = compute_layering(&i);
        assert_eq!(r.emitted_code, Some(RunCode::BudgetProjectionExceeded));
        assert_eq!(r.considered_codes, vec![RunCode::BudgetProjectionExceeded]);
    }

    #[test]
    fn live_available_budget_does_not_double_count_prior_spend() {
        let mut i = baseline_inputs();
        i.cumulative_cost_atomic = 100;
        i.this_call_reservation_atomic = 100;
        i.signal1_predicted_remaining_steps = 9;
        i.per_step_baseline_atomic = 100;
        i.budget_remaining_atomic = 1000;
        let r = compute_layering(&i);
        assert_eq!(r.projection_atomic, 1100);
        assert_eq!(r.predicted_remaining_cost_atomic, 900);
        assert_eq!(
            r.emitted_code, None,
            "available budget is live remaining balance, so compare only this call plus future predicted cost"
        );

        i.budget_remaining_atomic = 999;
        let r = compute_layering(&i);
        assert_eq!(r.emitted_code, Some(RunCode::BudgetProjectionExceeded));
    }

    #[test]
    fn budget_gate_excludes_cumulative_invariant() {
        // INVARIANT (contract-drift reconciliation): `budget_remaining_atomic`
        // is the LIVE ledger available balance, already net of prior spend.
        // The BUDGET gate therefore compares ONLY
        // `this_call_reservation + predicted_remaining_cost` against it and
        // MUST be invariant to cumulative_cost_atomic. Pinned so a future change
        // that "makes the code match the older spec formula" (adding cumulative
        // back into the gate) is caught as a regression — that direction would
        // double-count and over-trigger this money-stop control.
        let mut a = baseline_inputs();
        a.cumulative_cost_atomic = 0;
        a.this_call_reservation_atomic = 100;
        a.signal1_predicted_remaining_steps = 4;
        a.per_step_baseline_atomic = 100; // future predicted = 400
        a.budget_remaining_atomic = 500; // 100 + 400 == 500, NOT > → no code

        let mut b = a.clone();
        // Same live-available budget, but a large prior cumulative spend. If the
        // gate (incorrectly) re-added cumulative, this would now exceed.
        b.cumulative_cost_atomic = 1_000_000;

        let ra = compute_layering(&a);
        let rb = compute_layering(&b);
        assert_eq!(
            ra.emitted_code, None,
            "future commitment 500 is not > live available 500"
        );
        assert_eq!(
            rb.emitted_code, ra.emitted_code,
            "BUDGET gate must be invariant to cumulative_cost_atomic (live available is already net of prior spend)"
        );
        // The reported diagnostic projection, by contrast, DOES include cumulative.
        assert_eq!(ra.projection_atomic, 500); // 0 + 100 + 400
        assert_eq!(rb.projection_atomic, 1_000_500); // 1_000_000 + 100 + 400
    }

    #[test]
    fn steps_exceeded_fires_when_hint_active() {
        let mut i = baseline_inputs();
        i.planned_steps_hint = 5;
        i.steps_completed = 6; // > hint of 5 → STEPS_EXCEEDED.
        i.budget_remaining_atomic = 1_000_000; // budget OK.
        let r = compute_layering(&i);
        assert_eq!(r.emitted_code, Some(RunCode::StepsExceeded));
        assert!(r.considered_codes.contains(&RunCode::StepsExceeded));
    }

    #[test]
    fn drift_fires_when_confirmed() {
        let mut i = baseline_inputs();
        i.drift_confirmed = true;
        i.budget_remaining_atomic = 1_000_000;
        let r = compute_layering(&i);
        assert_eq!(r.emitted_code, Some(RunCode::DriftDetected));
    }

    #[test]
    fn budget_wins_over_steps_and_drift() {
        // Construct a state where ALL THREE codes fire so we can assert
        // §6.1 precedence (BUDGET > STEPS > DRIFT).
        //   - STEPS_EXCEEDED: hint=5, steps_completed=6 → 6 > 5
        //   - DRIFT: drift_confirmed=true
        //   - BUDGET: this_call + future predicted cost > live remaining budget
        //
        // For Signal 3-overridden remaining_steps to be non-zero (so BUDGET
        // future predicted cost has meaningful magnitude), we'd need
        // steps_completed <= hint. Spec §5.2 max(0, hint - completed) caps
        // remaining_steps at 0 when steps_completed > hint, so BUDGET here
        // depends purely on this_call.
        let mut i = baseline_inputs();
        i.planned_steps_hint = 5;
        i.steps_completed = 6; // STEPS_EXCEEDED fires (6 > 5).
        i.cumulative_cost_atomic = 800; // Already burned 800 of budget.
        i.this_call_reservation_atomic = 200; // This call adds 200.
        i.drift_confirmed = true; // DRIFT fires.
        i.budget_remaining_atomic = 100; // this_call 200 > remaining 100 → BUDGET fires.
        let r = compute_layering(&i);
        // Precedence per §6.1: BUDGET > STEPS > DRIFT.
        assert_eq!(r.emitted_code, Some(RunCode::BudgetProjectionExceeded));
        // All three considered codes recorded for SIEM forensics.
        assert!(r
            .considered_codes
            .contains(&RunCode::BudgetProjectionExceeded));
        assert!(r.considered_codes.contains(&RunCode::StepsExceeded));
        assert!(r.considered_codes.contains(&RunCode::DriftDetected));
    }

    #[test]
    fn steps_wins_over_drift() {
        let mut i = baseline_inputs();
        i.planned_steps_hint = 5;
        i.steps_completed = 6;
        i.drift_confirmed = true;
        i.budget_remaining_atomic = 1_000_000; // BUDGET inactive.
        let r = compute_layering(&i);
        // Precedence: STEPS > DRIFT.
        assert_eq!(r.emitted_code, Some(RunCode::StepsExceeded));
    }

    #[test]
    fn signal_3_overrides_signal_1_remaining_steps() {
        let mut i = baseline_inputs();
        i.signal1_predicted_remaining_steps = 100; // Signal 1 says lots of steps.
        i.planned_steps_hint = 8; // Signal 3 says only 8 total.
        i.steps_completed = 3;
        // Override → remaining = 8 - 3 = 5.
        let r = compute_layering(&i);
        assert_eq!(r.predicted_remaining_steps, 5);
        // projection = 0 + 100 + (5 × 100) = 600.
        assert_eq!(r.projection_atomic, 600);
    }

    #[test]
    fn cold_start_strategy_used_label() {
        let mut i = baseline_inputs();
        i.signal1_is_cold_start = true;
        let r = compute_layering(&i);
        assert_eq!(r.strategy_used, StrategyUsed::ColdStart);
    }

    #[test]
    fn s1s2s3_strategy_used_label_when_hint_active() {
        let mut i = baseline_inputs();
        i.planned_steps_hint = 5;
        let r = compute_layering(&i);
        assert_eq!(r.strategy_used, StrategyUsed::S1s2s3);
    }

    #[test]
    fn s3_strategy_used_label_when_hint_active_and_cold_start() {
        let mut i = baseline_inputs();
        i.planned_steps_hint = 5;
        i.signal1_is_cold_start = true;
        let r = compute_layering(&i);
        // hint active + cold-start S1 → S3 (S1 effectively absent so we
        // label by Signal 3 alone).
        assert_eq!(r.strategy_used, StrategyUsed::S3);
    }

    #[test]
    fn run_code_as_str_round_trip() {
        // Strings must match contract-dsl-spec-v1alpha2.md §3.x.
        assert_eq!(
            RunCode::BudgetProjectionExceeded.as_str(),
            "RUN_BUDGET_PROJECTION_EXCEEDED"
        );
        assert_eq!(RunCode::StepsExceeded.as_str(), "RUN_STEPS_EXCEEDED");
        assert_eq!(RunCode::DriftDetected.as_str(), "RUN_DRIFT_DETECTED");
    }

    #[test]
    fn negative_baseline_clamped_to_zero() {
        // Defense-in-depth: per_step_baseline_atomic should never be
        // negative but the wire allows it.
        let mut i = baseline_inputs();
        i.per_step_baseline_atomic = -100;
        let r = compute_layering(&i);
        // predicted_remaining_cost = remaining_steps × max(0, -100) = 0.
        assert_eq!(r.predicted_remaining_cost_atomic, 0);
        // projection = 0 + 100 + 0 = 100.
        assert_eq!(r.projection_atomic, 100);
    }
}
