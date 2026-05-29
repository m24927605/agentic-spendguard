//! Phase 3 wedge — Contract DSL hot-path evaluator.
//!
//! Replaces the Stage 2 stub in `decision/transaction.rs` with a real
//! evaluator that reads parsed Contract rules and applies them to
//! incoming `BudgetClaim[]` from `DecisionRequest.inputs.projected_claims`.
//!
//! ## SLICE_02 — Contract DSL v1alpha2 additive bump
//!
//! Adds the `cel_helpers` module for `run_projection.*` + `prediction.*`
//! accessor bindings, the `PredictionPolicy` / `RunProjectionAction` /
//! `RunCode` enums in `types.rs`, and the `handle_run_code()`
//! pass-through routing in `evaluate.rs`. Strictly additive over the
//! v1alpha1 wedge — v1alpha1 contracts continue to evaluate
//! byte-identically (per spec §6.4 + §8.2).

pub mod cel_helpers;
pub mod evaluate;
pub mod parse;
pub mod types;

pub use evaluate::{evaluate, handle_run_code};
pub use parse::parse_from_tgz;
pub use types::{
    is_allowed_pair, Action, Budget, Condition, Contract, EvalOutcome, PredictionPolicy, Rule,
    RunCode, RunProjectionAction, SharedContract,
};
