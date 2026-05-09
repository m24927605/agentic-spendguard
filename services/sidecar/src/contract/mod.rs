//! Phase 3 wedge — Contract DSL hot-path evaluator.
//!
//! Replaces the Stage 2 stub in `decision/transaction.rs` with a real
//! evaluator that reads parsed Contract rules and applies them to
//! incoming `BudgetClaim[]` from `DecisionRequest.inputs.projected_claims`.

pub mod evaluate;
pub mod parse;
pub mod types;

pub use evaluate::evaluate;
pub use parse::parse_from_tgz;
pub use types::{Action, Budget, Condition, Contract, EvalOutcome, Rule, SharedContract};
