//! SLICE_02 — CEL helpers for v1alpha2 contract authors.
//!
//! Per `docs/contract-dsl-spec-v1alpha2.md` §6.3, v1alpha2 contracts
//! gain two new CEL accessor namespaces:
//!
//!   * `run_projection.*` — per-run projection signals from the
//!     run_cost_projector (lit up in SLICE_09; here in SLICE_02 the
//!     helpers compile + evaluate against synthetic / mock data
//!     because the projector is not wired)
//!   * `prediction.*` — per-decision predictor outputs (which tier,
//!     which strategy chose, confidence)
//!
//! ## Design: opaque-context lookup, not registered functions
//!
//! `cel-interpreter 0.9` lets us mutate a `Context` with bindings
//! before each `program.execute()` call. Rather than registering CEL
//! functions (which would require trait implementations and
//! cross-version stability constraints), SLICE_02 ships a typed
//! Rust-side context struct + helper module that pulls out the
//! invariants we care about. SLICE_09 wires the projector / predictor
//! to populate this struct on every evaluation; SLICE_02 ships:
//!
//!   * the typed context structs
//!   * helper accessors with unit-test coverage
//!   * an `into_cel_context` adapter that turns the structs into a
//!     `cel_interpreter::Context` so contract authors can write
//!     `run_projection.at_decision_micros > budget.remaining` in
//!     CEL once SLICE_09 wires the source data
//!
//! ## SLICE_02 vs SLICE_09 split
//!
//! SLICE_02 (this module): typed structs + into_cel_context + unit tests.
//! SLICE_09 (future):     run_cost_projector population at decision time.
//!
//! The unit tests below validate the helpers against synthetic data
//! (per §8.1 "CEL helpers evaluate correctly against synthetic
//! predictor metadata"); SLICE_09 will add integration tests against
//! the real projector output.

use std::collections::HashMap;

/// Run-level projection signals exposed to contract CEL expressions
/// under the `run_projection.*` namespace.
///
/// Sentinels (matching audit-chain-prediction-extension-v1alpha1.md §3.3):
///   * `predicted_remaining_steps = -1` → projector unreachable.
///     Contract authors should defensively treat this as "no signal"
///     (eg `run_projection.predicted_remaining_steps >= 0`).
///   * `steps_completed_so_far` is a non-negative counter; SQL NULL
///     maps to 0 here (per round-4 fix M10 sentinel mapping).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunProjection {
    /// Atomic-units projection of cumulative run cost at decision time.
    /// NUMERIC(38,0) on SQL; serialized as i64 here (per audit-chain
    /// §3.2 sentinel mapping; assumes < 2^63 atomic units which is
    /// satisfied by any sane Money budget).
    pub at_decision_micros: i64,
    /// Predicted number of additional `llm.call.pre` boundaries
    /// remaining in this run. -1 = projector unreachable (sentinel).
    pub predicted_remaining_steps: i32,
    /// Step count completed so far in this run.
    pub steps_completed_so_far: i64,
}

/// Per-decision predictor metadata exposed under the `prediction.*`
/// namespace.
///
/// `tier` is one of "T1" / "T2" / "T3" (tokenizer tier per spec §6.3);
/// `strategy_chosen` is one of "A" / "B" / "C" (predictor strategy per
/// spec §4 + §6.3); `confidence` is 0.0-1.0 from the predictor on B/C
/// or 0.0 sentinel on A (audit-chain spec §3.3 NULL-mapping).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PredictionContext {
    pub tier: String,
    pub strategy_chosen: String,
    pub confidence: f32,
}

/// Build a CEL `Context` populated with `run_projection.*` and
/// `prediction.*` bindings.
///
/// Returns a `Context` ready for `program.execute(&ctx)`.
///
/// # SLICE_09-consumable API
///
/// SLICE_02 round-1 m1: this function is the supported entry point for
/// SLICE_09 (run_cost_projector) callers. The expected wiring sequence
/// at SLICE_09 time:
///
/// 1. The projector populates a `RunProjection` from
///    `(at_decision_micros, predicted_remaining_steps,
///    steps_completed_so_far)` per
///    `docs/run-cost-projector-spec-v1alpha1.md` §3.
/// 2. The predictor populates a `PredictionContext` from
///    `(tier, strategy_chosen, confidence)` per
///    `docs/predictor-architecture-spec-v1alpha1.md` §5.
/// 3. Call `into_cel_context(&run, &prediction)` to build the binding
///    `Context`.
/// 4. Pass the `Context` into `cel_interpreter::Program::execute` for
///    the rule's `condition: <cel-expr>` per spec §6.3 grammar.
///
/// Note: SLICE_02 ships this struct + helper with unit-test coverage
/// against synthetic data; the hot-path evaluator does NOT call it
/// (the wedge evaluator uses declarative `when.claim_amount_atomic_gt`
/// per `evaluate.rs`). Re-exported at `crate::contract::into_cel_context`
/// (per `mod.rs`) so SLICE_09 callers can `use crate::contract::*`
/// without reaching into the private module path.
///
/// SLICE_02 status: the CEL evaluator is wired but not yet called from
/// the hot path (the wedge evaluator uses declarative when/then on
/// claim amounts, per evaluate.rs). The contract author wiring in
/// SLICE_09 will replace the when/then with full CEL programs that
/// reference these accessors. Tests below validate that the bindings
/// can be looked up.
pub fn into_cel_context(
    run: &RunProjection,
    prediction: &PredictionContext,
) -> cel_interpreter::Context<'static> {
    use cel_interpreter::{Context, Value};

    // We build a fresh root Context so the helper is self-contained.
    // Callers that already hold a Context should merge instead — but
    // SLICE_02 callers always build from scratch.
    let mut ctx = Context::default();

    // run_projection.* bindings. CEL has no native int64 — i64 is
    // serialized as Value::Int (i64) so contract authors writing
    // `run_projection.at_decision_micros > 1_000_000` get correct
    // numeric semantics.
    let mut run_map: HashMap<cel_interpreter::objects::Key, Value> = HashMap::new();
    run_map.insert(
        "at_decision_micros".into(),
        Value::Int(run.at_decision_micros),
    );
    run_map.insert(
        "predicted_remaining_steps".into(),
        Value::Int(run.predicted_remaining_steps as i64),
    );
    run_map.insert(
        "steps_completed_so_far".into(),
        Value::Int(run.steps_completed_so_far),
    );
    ctx.add_variable("run_projection", Value::Map(run_map.into()))
        .expect("bind run_projection");

    // prediction.* bindings. Strings → Value::String; f32 → Value::Float.
    let mut pred_map: HashMap<cel_interpreter::objects::Key, Value> = HashMap::new();
    pred_map.insert(
        "tier".into(),
        Value::String(std::sync::Arc::new(prediction.tier.clone())),
    );
    pred_map.insert(
        "strategy_chosen".into(),
        Value::String(std::sync::Arc::new(prediction.strategy_chosen.clone())),
    );
    pred_map.insert(
        "confidence".into(),
        Value::Float(prediction.confidence as f64),
    );
    ctx.add_variable("prediction", Value::Map(pred_map.into()))
        .expect("bind prediction");

    ctx
}

#[cfg(test)]
mod tests {
    use super::*;
    use cel_interpreter::Program;

    /// Helper to compile + execute a CEL expression against
    /// synthetic context. Returns the CEL Value.
    fn eval_cel(expr: &str, run: &RunProjection, prediction: &PredictionContext) -> cel_interpreter::Value {
        let program = Program::compile(expr).expect("compile");
        let mut ctx = into_cel_context(run, prediction);
        program.execute(&mut ctx).expect("execute")
    }

    fn synth_run() -> RunProjection {
        RunProjection {
            at_decision_micros: 50_000_000, // $50.00
            predicted_remaining_steps: 5,
            steps_completed_so_far: 3,
        }
    }

    fn synth_prediction() -> PredictionContext {
        PredictionContext {
            tier: "T2".into(),
            strategy_chosen: "B".into(),
            confidence: 0.85,
        }
    }

    #[test]
    fn run_projection_at_decision_micros_accessible() {
        let v = eval_cel(
            "run_projection.at_decision_micros",
            &synth_run(),
            &synth_prediction(),
        );
        match v {
            cel_interpreter::Value::Int(n) => assert_eq!(n, 50_000_000),
            other => panic!("expected Int, got {:?}", other),
        }
    }

    #[test]
    fn run_projection_predicted_remaining_steps_accessible() {
        let v = eval_cel(
            "run_projection.predicted_remaining_steps",
            &synth_run(),
            &synth_prediction(),
        );
        match v {
            cel_interpreter::Value::Int(n) => assert_eq!(n, 5),
            other => panic!("expected Int, got {:?}", other),
        }
    }

    #[test]
    fn run_projection_steps_completed_so_far_accessible() {
        let v = eval_cel(
            "run_projection.steps_completed_so_far",
            &synth_run(),
            &synth_prediction(),
        );
        match v {
            cel_interpreter::Value::Int(n) => assert_eq!(n, 3),
            other => panic!("expected Int, got {:?}", other),
        }
    }

    #[test]
    fn prediction_tier_accessible() {
        let v = eval_cel(
            "prediction.tier",
            &synth_run(),
            &synth_prediction(),
        );
        match v {
            cel_interpreter::Value::String(s) => assert_eq!(&*s, "T2"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn prediction_strategy_chosen_accessible() {
        let v = eval_cel(
            "prediction.strategy_chosen",
            &synth_run(),
            &synth_prediction(),
        );
        match v {
            cel_interpreter::Value::String(s) => assert_eq!(&*s, "B"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn prediction_confidence_accessible() {
        let v = eval_cel(
            "prediction.confidence",
            &synth_run(),
            &synth_prediction(),
        );
        match v {
            cel_interpreter::Value::Float(f) => assert!((f - 0.85).abs() < 1e-6),
            other => panic!("expected Float, got {:?}", other),
        }
    }

    #[test]
    fn projector_unreachable_sentinel_evaluates() {
        // Spec §3.3: predicted_remaining_steps = -1 sentinel for
        // projector unreachable. Contract authors defensively check:
        //   run_projection.predicted_remaining_steps >= 0
        let run = RunProjection {
            at_decision_micros: 0,
            predicted_remaining_steps: -1,
            steps_completed_so_far: 0,
        };
        let v = eval_cel(
            "run_projection.predicted_remaining_steps >= 0",
            &run,
            &synth_prediction(),
        );
        match v {
            cel_interpreter::Value::Bool(b) => assert!(!b, "sentinel -1 should fail >= 0 check"),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn realistic_projection_rule_evaluates() {
        // Synthetic example mirroring spec §6.3 sample rule:
        //   run_projection.at_decision_micros >
        //   budget("daily_usd").remaining.amountMicros
        //
        // We can't bind `budget(...)` in SLICE_02 (no budget context),
        // but we can validate a self-contained projection-vs-threshold
        // comparison so SLICE_09 has a working evaluator template.
        let run = RunProjection {
            at_decision_micros: 100_000_000, // $100
            predicted_remaining_steps: 5,
            steps_completed_so_far: 3,
        };
        // Threshold $80.00 → projection $100 > threshold ⇒ true.
        let v = eval_cel(
            "run_projection.at_decision_micros > 80000000",
            &run,
            &synth_prediction(),
        );
        match v {
            cel_interpreter::Value::Bool(b) => assert!(b),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn strategy_chosen_string_equality_works() {
        // Realistic SLICE_09 contract author would write:
        //   prediction.strategy_chosen == "A" && prediction.confidence > 0.9
        let v = eval_cel(
            r#"prediction.strategy_chosen == "B""#,
            &synth_run(),
            &synth_prediction(),
        );
        match v {
            cel_interpreter::Value::Bool(b) => assert!(b),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn defaults_compile_and_execute() {
        // Empty defaults shouldn't panic CEL binding.
        let v = eval_cel(
            "run_projection.at_decision_micros",
            &RunProjection::default(),
            &PredictionContext::default(),
        );
        match v {
            cel_interpreter::Value::Int(n) => assert_eq!(n, 0),
            other => panic!("expected Int, got {:?}", other),
        }
    }
}
