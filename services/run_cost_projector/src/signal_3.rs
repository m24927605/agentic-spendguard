//! Signal 3 — explicit hint via SDK decorator.
//!
//! Spec ref `run-cost-projector-spec-v1alpha1.md` §5.
//!
//! ## SDK surface (SLICE_12)
//!
//! ```python
//! from spendguard import with_run_plan
//!
//! @with_run_plan(planned_calls=8, planned_tools=2)
//! async def my_agent_function(...):
//!     ...
//! ```
//!
//! SDK decorator stuffs `planned_steps_hint = N + M` into the
//! `request_decision` metadata; sidecar reads it from
//! `DecisionRequest.planned_steps_hint` (additive proto field; SLICE_02
//! added the pass-through stub; SLICE_09 Phase E activates the read).
//! Projector receives via `ProjectRequest.planned_steps_hint`.
//!
//! ## Override semantics (spec §5.2)
//!
//! ```text
//! IF req.planned_steps_hint > 0:
//!     predicted_remaining_steps = max(0, hint - steps_completed_so_far)
//! ELSE:
//!     predicted_remaining_steps = signal1_value
//! ```
//!
//! ## RUN_STEPS_EXCEEDED trigger (spec §5.3)
//!
//! ```text
//! IF hint > 0 AND steps_completed_so_far > hint:
//!     emit RUN_STEPS_EXCEEDED
//! ```
//!
//! Vanilla agents (no decorator) → hint = 0 → never triggers
//! RUN_STEPS_EXCEEDED. This keeps universal-coverage Signal 1 + 2 active
//! while opt-in Signal 3 fires only for SDK-cooperative workloads.

/// Apply Signal 3 override to Signal 1's output. Returns the final
/// `predicted_remaining_steps` after the override decision.
///
/// `hint = 0` means "Signal 3 inactive" (proto3 default) — Signal 1's
/// value is preserved unchanged.
pub fn apply_signal3_override(
    signal1_predicted_steps: i32,
    planned_steps_hint: i32,
    steps_completed: i64,
) -> i32 {
    if planned_steps_hint <= 0 {
        return signal1_predicted_steps;
    }
    // Signal 3 active — compute hint-based remaining.
    let remaining = (planned_steps_hint as i64 - steps_completed)
        .max(0)
        .min(i32::MAX as i64) as i32;
    remaining
}

/// Detect RUN_STEPS_EXCEEDED per spec §5.3.
///
/// Returns true iff the SDK declared a plan AND the run has already
/// executed more steps than the plan. Hint = 0 → never triggers.
pub fn steps_exceeded(planned_steps_hint: i32, steps_completed: i64) -> bool {
    if planned_steps_hint <= 0 {
        return false;
    }
    steps_completed > planned_steps_hint as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_hint_preserves_signal_1() {
        // hint=0 → Signal 1's 5 passes through.
        assert_eq!(apply_signal3_override(5, 0, 2), 5);
        // hint negative → treated as inactive too.
        assert_eq!(apply_signal3_override(5, -1, 2), 5);
    }

    #[test]
    fn hint_overrides_signal_1() {
        // hint=8, completed=3 → 5 remaining (overrides Signal 1's 99).
        assert_eq!(apply_signal3_override(99, 8, 3), 5);
    }

    #[test]
    fn hint_floors_at_zero_when_completed_exceeds() {
        // hint=5, completed=10 → 0 remaining (saturates at 0 per spec §5.2 max(0, …)).
        assert_eq!(apply_signal3_override(99, 5, 10), 0);
    }

    #[test]
    fn steps_exceeded_only_triggers_with_active_hint() {
        // No hint → never triggers, no matter how high steps.
        assert!(!steps_exceeded(0, 1000));
        assert!(!steps_exceeded(-1, 1000));
        // Hint=10, completed=10 → equal, NOT exceeded (strict >).
        assert!(!steps_exceeded(10, 10));
        // Hint=10, completed=11 → exceeded.
        assert!(steps_exceeded(10, 11));
        // Hint=10, completed=0 → not exceeded.
        assert!(!steps_exceeded(10, 0));
    }
}
