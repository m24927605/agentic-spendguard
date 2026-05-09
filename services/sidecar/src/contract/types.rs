//! Phase 3 wedge — Contract DSL types.
//!
//! POC subset of `docs/contract-dsl-spec-v1alpha1.md` §6 (Decision
//! Transaction State Machine) + §7 (Reservation Authorization 兩相).
//! Out of scope: full CEL predicate engine, refund/dispute (§5.1a is
//! provider-lifecycle, not contract-evaluation), multi-tier approval
//! flow (POC treats REQUIRE_APPROVAL as terminal).

use std::sync::Arc;

use uuid::Uuid;

use crate::proto::sidecar_adapter::v1::decision_response::Decision;

/// Parsed contract bundle ready for hot-path evaluation.
#[derive(Debug, Clone)]
pub struct Contract {
    pub id: Uuid,
    pub name: String,
    pub budgets: Vec<Budget>,
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone)]
pub struct Budget {
    pub id: Uuid,
    pub limit_amount_atomic: String, // NUMERIC(38,0) decimal string
    pub currency: String,
    pub reservation_ttl_seconds: i64,
    pub require_hard_cap: bool,
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub id: String,
    pub when: Condition,
    pub then: Action,
}

#[derive(Debug, Clone)]
pub struct Condition {
    pub budget_id: Uuid,
    /// Match when `claim.amount_atomic > value`.
    pub claim_amount_atomic_gt: Option<String>,
    /// Match when `claim.amount_atomic >= value`.
    pub claim_amount_atomic_gte: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Action {
    pub decision: Decision,
    pub reason_code: String,
    pub approver_role: Option<String>,
}

/// Evaluator output. `decision` is the lattice-merged final decision
/// across all matched rules (most-restrictive wins per Contract §10).
#[derive(Debug, Clone)]
pub struct EvalOutcome {
    pub decision: Decision,
    pub reason_codes: Vec<String>,
    pub matched_rule_ids: Vec<String>,
}

impl EvalOutcome {
    /// Default CONTINUE outcome when no rules match (open-by-default
    /// for unmatched claims; explicit DENY rules opt-in).
    pub fn continue_default() -> Self {
        Self {
            decision: Decision::Continue,
            reason_codes: Vec::new(),
            matched_rule_ids: Vec::new(),
        }
    }
}

pub type SharedContract = Arc<Contract>;
