//! Phase 3 wedge — Contract DSL types.
//!
//! POC subset of `docs/contract-dsl-spec-v1alpha1.md` §6 (Decision
//! Transaction State Machine) + §7 (Reservation Authorization 兩相).
//! Out of scope: full CEL predicate engine, refund/dispute (§5.1a is
//! provider-lifecycle, not contract-evaluation), multi-tier approval
//! flow (POC treats REQUIRE_APPROVAL as terminal).
//!
//! ## SLICE_02 — Contract DSL v1alpha2 additive
//!
//! Strictly additive bump per `docs/contract-dsl-spec-v1alpha2.md`. Adds:
//!   * `PredictionPolicy` enum (4 variants; default STRICT_CEILING)
//!   * `RunProjectionAction` enum (3 variants; default BLOCK_NEXT_CALL)
//!   * `RunCode` enum (3 pass-through codes; SLICE_09 wires emission)
//!   * `Contract.prediction_policy` field (top-level)
//!   * `Rule.run_projection_action` per-rule override
//!
//! No v1alpha1 invariants changed; v1alpha1 contracts get default fill
//! (`STRICT_CEILING + BLOCK_NEXT_CALL`) at parse time so the evaluator
//! sees a fully-populated Contract regardless of source apiVersion.

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
    /// SLICE_02 v1alpha2: contract-wide prediction policy. Defaults to
    /// `STRICT_CEILING` for v1alpha1 contracts (per spec §6.4). Drives
    /// which Strategy (A/B/C) is reserved vs which is recorded in
    /// `prediction_strategy_used` (calibration evidence path).
    pub prediction_policy: PredictionPolicy,
    /// SLICE_02 v1alpha2: apiVersion the bundle was loaded from.
    /// Retained verbatim for audit/observability — calibration-report
    /// can group by source apiVersion to surface mixed-fleet behavior.
    pub api_version: String,
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
    /// SLICE_02 v1alpha2: per-rule run-projection action override.
    /// Defaults to `BLOCK_NEXT_CALL` (per spec §5 default).
    /// Only consulted when the rule's reasonCode matches a `RUN_*`
    /// pass-through code (per `handle_run_code` in evaluate.rs).
    /// For v1alpha1 rules with non-RUN_* reasonCodes this field is
    /// inert (evaluator never looks at it on the per-call lattice path).
    pub run_projection_action: RunProjectionAction,
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

// =====================================================================
// SLICE_02 — Contract DSL v1alpha2 additive enums
// =====================================================================

/// Prediction policy (per `docs/contract-dsl-spec-v1alpha2.md` §4).
/// Drives reservation strategy + which `prediction_strategy_used` value
/// the evaluator records on every decision row.
///
/// `STRICT_CEILING` is the default per spec §4.1: regulated workloads
/// (healthcare / finance / government) cannot have "typical case"
/// estimates leak into enforcement. v1alpha1 contracts inherit this
/// value automatically (per §6.4) so backward compat is byte-identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PredictionPolicy {
    /// Default. Reservation = Strategy A (ceiling); regulated workloads.
    StrictCeiling,
    /// Reservation = Strategy B (empirical mean); cost-optimised
    /// non-regulated workloads where B has ≥30-sample bucket.
    EmpiricalRunCeiling,
    /// Reservation = Strategy A or B depending on `remaining_budget
    /// < 2 × A`; smoothly degrades to STRICT_CEILING near exhaustion.
    AdaptiveCeiling,
    /// Reservation = Strategy A; no enforcement; calibration-only mode.
    /// Allowed pair: ALERT_ONLY only (per §5.3).
    ShadowOnly,
}

impl PredictionPolicy {
    /// Canonical string form for audit / wire emission.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::StrictCeiling => "STRICT_CEILING",
            Self::EmpiricalRunCeiling => "EMPIRICAL_RUN_CEILING",
            Self::AdaptiveCeiling => "ADAPTIVE_CEILING",
            Self::ShadowOnly => "SHADOW_ONLY",
        }
    }

    /// Parse a YAML / wire token. Case-sensitive (spec §4 uses uppercase).
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "STRICT_CEILING" => Some(Self::StrictCeiling),
            "EMPIRICAL_RUN_CEILING" => Some(Self::EmpiricalRunCeiling),
            "ADAPTIVE_CEILING" => Some(Self::AdaptiveCeiling),
            "SHADOW_ONLY" => Some(Self::ShadowOnly),
            _ => None,
        }
    }
}

impl Default for PredictionPolicy {
    fn default() -> Self {
        Self::StrictCeiling
    }
}

/// Per-rule action when a RUN_* code matches (per spec §5).
///
/// Default is `BLOCK_NEXT_CALL` (per spec §5 default) so v1alpha1
/// contracts that don't know about RUN_* codes get the strictest
/// behavior when SLICE_09 starts emitting them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunProjectionAction {
    /// Default. RUN_* triggers → `Decision::Stop` (v1alpha1 lattice).
    BlockNextCall,
    /// RUN_* triggers → `Decision::RequireApproval`.
    RequireApproval,
    /// RUN_* triggers → `Decision::Continue` + audit event only.
    /// Disallowed under `STRICT_CEILING` (per spec §5.3 allowed-pairs).
    AlertOnly,
}

impl RunProjectionAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BlockNextCall => "BLOCK_NEXT_CALL",
            Self::RequireApproval => "REQUIRE_APPROVAL",
            Self::AlertOnly => "ALERT_ONLY",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "BLOCK_NEXT_CALL" => Some(Self::BlockNextCall),
            "REQUIRE_APPROVAL" => Some(Self::RequireApproval),
            "ALERT_ONLY" => Some(Self::AlertOnly),
            _ => None,
        }
    }
}

impl Default for RunProjectionAction {
    fn default() -> Self {
        Self::BlockNextCall
    }
}

/// The three RUN_* decision codes introduced in v1alpha2 (per spec §3).
///
/// SLICE_02 routes these through `handle_run_code` (per spec §7.1).
/// Emission is wired in SLICE_09 (run_cost_projector).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunCode {
    /// Per spec §3.1. Per-run projected cumulative > budget remaining.
    BudgetProjectionExceeded,
    /// Per spec §3.2. Run-instance drift (per-step cost rising > 2σ).
    DriftDetected,
    /// Per spec §3.3. Step count exceeded `with_run_plan` hint.
    StepsExceeded,
}

impl RunCode {
    /// Canonical string form — must match the wire-format
    /// `DecisionResponse.run_code_triggered` field and the
    /// audit row `reason_codes` array entry per spec §3.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BudgetProjectionExceeded => "RUN_BUDGET_PROJECTION_EXCEEDED",
            Self::DriftDetected => "RUN_DRIFT_DETECTED",
            Self::StepsExceeded => "RUN_STEPS_EXCEEDED",
        }
    }

    /// Recognize a string reasonCode emitted by upstream RUN_* sources.
    /// Returns None for non-RUN_* codes (per-call codes pass through
    /// the regular lattice).
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "RUN_BUDGET_PROJECTION_EXCEEDED" => Some(Self::BudgetProjectionExceeded),
            "RUN_DRIFT_DETECTED" => Some(Self::DriftDetected),
            "RUN_STEPS_EXCEEDED" => Some(Self::StepsExceeded),
            _ => None,
        }
    }
}

/// Allowed-pairs check per spec §5.3.
///
/// This is enforced at bundle load time in `bundle.rs`; the evaluator
/// trusts that any `(policy, action)` pair reaching it has passed.
///
///   STRICT_CEILING        → BLOCK_NEXT_CALL only
///   EMPIRICAL_RUN_CEILING → all 3
///   ADAPTIVE_CEILING      → all 3
///   SHADOW_ONLY           → ALERT_ONLY only
pub fn is_allowed_pair(policy: PredictionPolicy, action: RunProjectionAction) -> bool {
    match (policy, action) {
        (PredictionPolicy::StrictCeiling, RunProjectionAction::BlockNextCall) => true,
        (PredictionPolicy::StrictCeiling, _) => false,

        (PredictionPolicy::EmpiricalRunCeiling, _) => true,
        (PredictionPolicy::AdaptiveCeiling, _) => true,

        (PredictionPolicy::ShadowOnly, RunProjectionAction::AlertOnly) => true,
        (PredictionPolicy::ShadowOnly, _) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prediction_policy_round_trip() {
        for s in [
            "STRICT_CEILING",
            "EMPIRICAL_RUN_CEILING",
            "ADAPTIVE_CEILING",
            "SHADOW_ONLY",
        ] {
            let p = PredictionPolicy::from_str(s).expect("known");
            assert_eq!(p.as_str(), s);
        }
        assert!(PredictionPolicy::from_str("UNKNOWN").is_none());
        // Case-sensitivity: lowercase rejected.
        assert!(PredictionPolicy::from_str("strict_ceiling").is_none());
    }

    #[test]
    fn run_projection_action_round_trip() {
        for s in ["BLOCK_NEXT_CALL", "REQUIRE_APPROVAL", "ALERT_ONLY"] {
            let a = RunProjectionAction::from_str(s).expect("known");
            assert_eq!(a.as_str(), s);
        }
        assert!(RunProjectionAction::from_str("UNKNOWN").is_none());
    }

    #[test]
    fn run_code_round_trip() {
        for s in [
            "RUN_BUDGET_PROJECTION_EXCEEDED",
            "RUN_DRIFT_DETECTED",
            "RUN_STEPS_EXCEEDED",
        ] {
            let c = RunCode::from_str(s).expect("known");
            assert_eq!(c.as_str(), s);
        }
        // Non-RUN_* codes return None — pass-through to lattice.
        assert!(RunCode::from_str("BUDGET_EXHAUSTED").is_none());
        assert!(RunCode::from_str("LARGE_CLAIM_REQUIRES_APPROVAL").is_none());
    }

    #[test]
    fn defaults_match_spec() {
        // Spec §4 default = STRICT_CEILING.
        assert_eq!(PredictionPolicy::default(), PredictionPolicy::StrictCeiling);
        // Spec §5 default = BLOCK_NEXT_CALL.
        assert_eq!(
            RunProjectionAction::default(),
            RunProjectionAction::BlockNextCall
        );
    }

    #[test]
    fn allowed_pairs_table_strict_ceiling() {
        // STRICT_CEILING allows ONLY BlockNextCall (spec §5.3).
        assert!(is_allowed_pair(
            PredictionPolicy::StrictCeiling,
            RunProjectionAction::BlockNextCall
        ));
        assert!(!is_allowed_pair(
            PredictionPolicy::StrictCeiling,
            RunProjectionAction::RequireApproval
        ));
        assert!(!is_allowed_pair(
            PredictionPolicy::StrictCeiling,
            RunProjectionAction::AlertOnly
        ));
    }

    #[test]
    fn allowed_pairs_table_empirical_and_adaptive() {
        // EMPIRICAL + ADAPTIVE allow all 3 (spec §5.3).
        for policy in [
            PredictionPolicy::EmpiricalRunCeiling,
            PredictionPolicy::AdaptiveCeiling,
        ] {
            for action in [
                RunProjectionAction::BlockNextCall,
                RunProjectionAction::RequireApproval,
                RunProjectionAction::AlertOnly,
            ] {
                assert!(
                    is_allowed_pair(policy, action),
                    "expected {:?} + {:?} to be allowed",
                    policy,
                    action
                );
            }
        }
    }

    #[test]
    fn allowed_pairs_table_shadow_only() {
        // SHADOW_ONLY allows ONLY AlertOnly (spec §5.3).
        assert!(is_allowed_pair(
            PredictionPolicy::ShadowOnly,
            RunProjectionAction::AlertOnly
        ));
        assert!(!is_allowed_pair(
            PredictionPolicy::ShadowOnly,
            RunProjectionAction::BlockNextCall
        ));
        assert!(!is_allowed_pair(
            PredictionPolicy::ShadowOnly,
            RunProjectionAction::RequireApproval
        ));
    }

    #[test]
    fn allowed_pairs_full_4x3_combinations() {
        // Property: exactly 1 + 3 + 3 + 1 = 8 allowed pairs out of 12.
        let mut allowed_count = 0;
        let mut denied_count = 0;
        for policy in [
            PredictionPolicy::StrictCeiling,
            PredictionPolicy::EmpiricalRunCeiling,
            PredictionPolicy::AdaptiveCeiling,
            PredictionPolicy::ShadowOnly,
        ] {
            for action in [
                RunProjectionAction::BlockNextCall,
                RunProjectionAction::RequireApproval,
                RunProjectionAction::AlertOnly,
            ] {
                if is_allowed_pair(policy, action) {
                    allowed_count += 1;
                } else {
                    denied_count += 1;
                }
            }
        }
        assert_eq!(allowed_count, 8, "spec §5.3 = 8 allowed pairs");
        assert_eq!(denied_count, 4, "spec §5.3 = 4 denied pairs");
    }
}
