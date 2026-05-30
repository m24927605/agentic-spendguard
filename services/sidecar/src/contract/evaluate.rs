//! Hot-path contract evaluator (Stage 2 of decision transaction).
//!
//! Per `docs/contract-dsl-spec-v1alpha1.md` §6 stages.evaluate
//! (`timeout_ms: 5`): walk rules, match claims, lattice-merge to a
//! single Decision via Contract §10 effect lattice + same-type merge.
//!
//! POC simplifications:
//! - Lattice merge uses fixed restrictiveness ranking instead of a full
//!   §10 implementation: STOP > REQUIRE_APPROVAL > DEGRADE > SKIP > CONTINUE.
//! - Claim matching is per-claim, then the most-restrictive Decision
//!   across all matched (rule × claim) pairs becomes the outcome.
//! - No CEL — only declarative when/then on amount thresholds and budget_id.
//!
//! Open-by-default semantics: claims that match no rule contribute
//! Decision::Continue. Explicit deny rules opt-in.
//!
//! ## SLICE_02 — RUN_* code pass-through (v1alpha2 additive)
//!
//! Per `docs/contract-dsl-spec-v1alpha2.md` §7.1: when a contract rule's
//! `reasonCode` is one of the 3 new RUN_* codes
//! (`RUN_BUDGET_PROJECTION_EXCEEDED`, `RUN_DRIFT_DETECTED`,
//! `RUN_STEPS_EXCEEDED`), the evaluator routes through `handle_run_code()`
//! instead of the per-call lattice — projecting the rule's
//! `run_projection_action` onto a v1alpha1 lattice decision (per §3.4).
//!
//! SLICE_02 does NOT yet emit RUN_* codes (no run_cost_projector). The
//! routing is wired so that when SLICE_09 starts emitting them, the
//! sidecar already knows how to translate. Until then, the routing
//! never fires in practice — but the unit test below asserts it does
//! the right thing when fed synthetic RUN_* rules.

use num_bigint::BigInt;

use crate::contract::types::{
    Contract, EvalOutcome, PredictionPolicy, Rule, RunCode, RunProjectionAction,
};
use crate::proto::common::v1::BudgetClaim;
use crate::proto::sidecar_adapter::v1::decision_response::Decision;

/// Evaluate a contract against a set of incoming budget claims.
///
/// Returns the lattice-merged outcome. Caller (sidecar
/// `decision/transaction.rs` Stage 2) uses `outcome.decision` to
/// branch into Reserve (CONTINUE) or RecordDeniedDecision
/// (everything else).
pub fn evaluate(contract: &Contract, claims: &[BudgetClaim]) -> EvalOutcome {
    let mut current = EvalOutcome::continue_default();

    for claim in claims {
        let claim_budget_id = match uuid::Uuid::parse_str(&claim.budget_id) {
            Ok(u) => u,
            Err(_) => continue, // malformed claim — let downstream surface it
        };
        let claim_amount = match claim.amount_atomic.parse::<BigInt>() {
            Ok(n) => n,
            Err(_) => continue,
        };

        for rule in &contract.rules {
            if rule.when.budget_id != claim_budget_id {
                continue;
            }
            if !rule_matches_claim(rule, &claim_amount) {
                continue;
            }
            current = merge(current, rule_outcome(contract, rule));
        }
    }

    current
}

fn rule_matches_claim(rule: &Rule, claim_amount: &BigInt) -> bool {
    if let Some(gt) = &rule.when.claim_amount_atomic_gt {
        if let Ok(threshold) = gt.parse::<BigInt>() {
            if !(claim_amount > &threshold) {
                return false;
            }
        }
    }
    if let Some(gte) = &rule.when.claim_amount_atomic_gte {
        if let Ok(threshold) = gte.parse::<BigInt>() {
            if !(claim_amount >= &threshold) {
                return false;
            }
        }
    }
    // At least one of gt/gte must have been specified for a meaningful
    // match. A rule with neither is a no-op (open-by-default).
    rule.when.claim_amount_atomic_gt.is_some() || rule.when.claim_amount_atomic_gte.is_some()
}

fn rule_outcome(contract: &Contract, rule: &Rule) -> EvalOutcome {
    // SLICE_02 §7.1: RUN_* reasonCodes route through handle_run_code
    // instead of the per-call lattice. The rule author can write
    //
    //   effect:
    //     decision: stop           # v1alpha1 lattice placeholder
    //     reasonCode: RUN_BUDGET_PROJECTION_EXCEEDED
    //   run_projection_action: REQUIRE_APPROVAL
    //
    // and the evaluator IGNORES the `decision` field in favor of the
    // projection from `run_projection_action` per spec §3.4 — the rule
    // author can't "fight" the policy decision by writing
    // `decision: continue` next to a RUN_* code. The matched_rule_ids
    // is still emitted so calibration-report can attribute the rule.
    //
    // For non-RUN_* codes (everything in v1alpha1 + custom reasonCodes
    // in v1alpha2 contracts), `RunCode::from_str` returns None and we
    // fall through to the regular lattice path. Open-by-default for
    // unknown codes — the per-call lattice already handles the
    // explicit-deny opt-in semantics.
    if let Some(run_code) = RunCode::from_str(&rule.then.reason_code) {
        return handle_run_code(contract, rule, run_code, rule.run_projection_action);
    }

    // Codex R1 P1: namespace by budget_id so two rules sharing an `id`
    // under different budgets disambiguate cleanly in the audit payload.
    EvalOutcome {
        decision: rule.then.decision,
        reason_codes: vec![rule.then.reason_code.clone()],
        matched_rule_ids: vec![format!(
            "{}:{}:{}",
            contract.id, rule.when.budget_id, rule.id
        )],
    }
}

/// SLICE_02 §7.1 — RUN_* code pass-through.
///
/// Routes a matched RUN_* rule through its `run_projection_action`
/// projection onto v1alpha1 lattice (per spec §3.4):
///
/// | RunProjectionAction | v1alpha1 lattice | Notes                          |
/// |---------------------|------------------|--------------------------------|
/// | BlockNextCall       | Decision::Stop   | Default; regulated workloads.  |
/// | RequireApproval     | Decision::RequireApproval | Approval flow via §11.|
/// | AlertOnly           | Decision::Continue | Audit row still emitted.     |
///
/// The reason_codes vector ALWAYS contains the RUN_* code string so
/// downstream consumers (SIEM, dashboard, calibration-report) can
/// filter on it independent of the v1alpha1 lattice mapping. This is
/// the §3.4 invariant: "v1alpha1 lattice + audit row decision field
/// stays v1alpha1; new RUN_* codes appear in reason_codes only."
///
/// Bundle load-time `is_allowed_pair` validation guarantees the
/// (contract.prediction_policy, rule.run_projection_action) pair is
/// in the allowed set per §5.3; therefore this function does not
/// re-check it. If it ever runs against a stale Contract that
/// bypassed validation (shouldn't happen in production), the
/// projection still produces a well-formed EvalOutcome — the spec's
/// §5.3 protection is at load time, not eval time.
///
/// SLICE_02 status: this function is wired through `rule_outcome` so
/// any v1alpha2 contract with a RUN_* reason code routes through it
/// at evaluator time. NO source emits RUN_* codes until SLICE_09
/// integrates run_cost_projector. The function therefore never fires
/// on real traffic in SLICE_02 — but the unit tests below assert it
/// behaves correctly when synthetic RUN_* rules are evaluated.
pub fn handle_run_code(
    contract: &Contract,
    rule: &Rule,
    code: RunCode,
    action: RunProjectionAction,
) -> EvalOutcome {
    let decision = match action {
        RunProjectionAction::BlockNextCall => Decision::Stop,
        RunProjectionAction::RequireApproval => Decision::RequireApproval,
        RunProjectionAction::AlertOnly => Decision::Continue,
    };
    EvalOutcome {
        decision,
        reason_codes: vec![code.as_str().to_string()],
        matched_rule_ids: vec![format!(
            "{}:{}:{}",
            contract.id, rule.when.budget_id, rule.id
        )],
    }
}

/// SLICE_09 Phase E — projector-driven RUN_* projection.
///
/// Called by `decision/transaction.rs` after `evaluate()` returns and after
/// run_cost_projector.Project supplies an `emitted_code` (non-empty). The
/// contract's `prediction_policy` determines the default
/// `RunProjectionAction` per spec §5.3 allowed-pairs table:
///
/// | PredictionPolicy        | Default RunProjectionAction      |
/// |-------------------------|----------------------------------|
/// | STRICT_CEILING          | BlockNextCall (only allowed)     |
/// | EMPIRICAL_RUN_CEILING   | BlockNextCall (conservative)     |
/// | ADAPTIVE_CEILING        | BlockNextCall (conservative)     |
/// | SHADOW_ONLY             | AlertOnly (only allowed)         |
///
/// If the matched contract rules carry an explicit RUN_* rule (SLICE_02
/// pass-through), that path takes precedence via `evaluate()`'s
/// `handle_run_code` route — this function fires only when the projector
/// emits a code without a corresponding contract rule (the zero-config
/// universal-coverage case per spec §3.3).
///
/// Returns an EvalOutcome with the projected lattice decision + the RUN_*
/// reason code. Caller merges with the prior `evaluate()` outcome via the
/// same most-restrictive lattice (re-exposed below as `merge_outcomes`).
pub fn apply_projector_code(contract: &Contract, code_str: &str) -> Option<EvalOutcome> {
    let code = RunCode::from_str(code_str)?;
    // Default action by policy (per spec §5.3).
    let action = match contract.prediction_policy {
        PredictionPolicy::ShadowOnly => RunProjectionAction::AlertOnly,
        PredictionPolicy::StrictCeiling
        | PredictionPolicy::EmpiricalRunCeiling
        | PredictionPolicy::AdaptiveCeiling => RunProjectionAction::BlockNextCall,
    };
    let decision = match action {
        RunProjectionAction::BlockNextCall => {
            // Spec contract-dsl-v1alpha2 §3.4: STOP_RUN_PROJECTION is the
            // dashboard categorisation; for lattice purposes it has the
            // same precedence as STOP. We emit StopRunProjection so
            // build_response can surface the distinct enum to consumers
            // that filter on it (SIEM dashboards).
            Decision::StopRunProjection
        }
        RunProjectionAction::RequireApproval => Decision::RequireApproval,
        RunProjectionAction::AlertOnly => Decision::Continue,
    };
    Some(EvalOutcome {
        decision,
        reason_codes: vec![code.as_str().to_string()],
        // Synthetic matched_rule_id — calibration-report uses this to
        // attribute the run-projection decision to the projector service
        // rather than a contract rule.
        matched_rule_ids: vec![format!("{}:projector:{}", contract.id, code.as_str())],
    })
}

/// SLICE_09 Phase E — most-restrictive merge of an evaluator outcome with
/// a projector outcome. Re-exposes the internal merge function for use by
/// decision/transaction.rs.
pub fn merge_outcomes(a: EvalOutcome, b: EvalOutcome) -> EvalOutcome {
    merge(a, b)
}

/// Most-restrictive merge. Order from least to most restrictive:
/// CONTINUE < SKIP < DEGRADE < REQUIRE_APPROVAL < STOP.
fn merge(a: EvalOutcome, b: EvalOutcome) -> EvalOutcome {
    let pick_b = restrictiveness(b.decision) > restrictiveness(a.decision);
    if pick_b {
        // b wins decision; accumulate reason_codes + matched_rule_ids.
        let mut reasons = a.reason_codes;
        reasons.extend(b.reason_codes);
        let mut rules = a.matched_rule_ids;
        rules.extend(b.matched_rule_ids);
        EvalOutcome {
            decision: b.decision,
            reason_codes: reasons,
            matched_rule_ids: rules,
        }
    } else {
        // a wins decision; still accumulate b's evidence.
        let mut reasons = a.reason_codes;
        reasons.extend(b.reason_codes);
        let mut rules = a.matched_rule_ids;
        rules.extend(b.matched_rule_ids);
        EvalOutcome {
            decision: a.decision,
            reason_codes: reasons,
            matched_rule_ids: rules,
        }
    }
}

fn restrictiveness(d: Decision) -> u8 {
    match d {
        Decision::Unspecified => 0,
        Decision::Continue => 1,
        Decision::Skip => 2,
        Decision::Degrade => 3,
        Decision::RequireApproval => 4,
        Decision::Stop => 5,
        // SLICE_02 §3.4: STOP_RUN_PROJECTION shares STOP precedence
        // for lattice merge purposes. The DecisionResponse-level enum
        // value is informational categorisation; the lattice still
        // resolves to STOP behavior. Same ranking as STOP keeps the
        // most-restrictive-wins semantics consistent — a per-call
        // STOP next to a STOP_RUN_PROJECTION mid-rule should not
        // accidentally elevate one above the other; both terminate
        // the run identically.
        Decision::StopRunProjection => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::types::{Action, Budget, Condition, PredictionPolicy, Rule, RunCode};
    use uuid::Uuid;

    fn budget_id() -> Uuid {
        Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap()
    }

    fn make_contract(rules: Vec<Rule>) -> Contract {
        // SLICE_02: default policy = STRICT_CEILING + apiVersion v1alpha1.
        // Tests that need other policies override via `make_contract_with_policy`.
        Contract {
            id: Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap(),
            name: "test".into(),
            budgets: vec![Budget {
                id: budget_id(),
                limit_amount_atomic: "1000000000".into(),
                currency: "USD".into(),
                reservation_ttl_seconds: 600,
                require_hard_cap: true,
            }],
            rules,
            prediction_policy: PredictionPolicy::default(),
            api_version: "spendguard.ai/v1alpha1".into(),
        }
    }

    fn make_contract_with_policy(
        rules: Vec<Rule>,
        policy: PredictionPolicy,
    ) -> Contract {
        let mut c = make_contract(rules);
        c.prediction_policy = policy;
        c.api_version = "spendguard.ai/v1alpha2".into();
        c
    }

    /// Default-fill helper: build a v1alpha1-style rule (no
    /// run_projection_action awareness). The test author writes
    /// `then.decision` + `then.reason_code` and the rule gets
    /// `run_projection_action = BlockNextCall` (the default).
    fn rule_v1alpha1(
        id: &str,
        condition: Condition,
        decision: Decision,
        reason_code: &str,
    ) -> Rule {
        Rule {
            id: id.into(),
            when: condition,
            then: Action {
                decision,
                reason_code: reason_code.into(),
                approver_role: None,
            },
            run_projection_action: RunProjectionAction::default(),
        }
    }

    fn claim(amount: &str) -> BudgetClaim {
        BudgetClaim {
            budget_id: budget_id().to_string(),
            unit: None,
            amount_atomic: amount.into(),
            direction: 1,
            window_instance_id: String::new(),
        }
    }

    #[test]
    fn no_rules_returns_continue() {
        let c = make_contract(vec![]);
        let out = evaluate(&c, &[claim("100")]);
        assert!(matches!(out.decision, Decision::Continue));
        assert!(out.matched_rule_ids.is_empty());
    }

    #[test]
    fn hard_cap_rule_denies() {
        let c = make_contract(vec![rule_v1alpha1(
            "hard-cap",
            Condition {
                budget_id: budget_id(),
                claim_amount_atomic_gt: Some("1000000000".into()),
                claim_amount_atomic_gte: None,
            },
            Decision::Stop,
            "BUDGET_EXHAUSTED",
        )]);
        let out = evaluate(&c, &[claim("2000000000")]);
        assert!(matches!(out.decision, Decision::Stop));
        assert_eq!(out.reason_codes, vec!["BUDGET_EXHAUSTED".to_string()]);
    }

    #[test]
    fn most_restrictive_wins() {
        let c = make_contract(vec![
            rule_v1alpha1(
                "approval",
                Condition {
                    budget_id: budget_id(),
                    claim_amount_atomic_gt: None,
                    claim_amount_atomic_gte: Some("100".into()),
                },
                Decision::RequireApproval,
                "AMOUNT_OVER_THRESHOLD",
            ),
            rule_v1alpha1(
                "hard-cap",
                Condition {
                    budget_id: budget_id(),
                    claim_amount_atomic_gt: Some("500".into()),
                    claim_amount_atomic_gte: None,
                },
                Decision::Stop,
                "BUDGET_EXHAUSTED",
            ),
        ]);
        let out = evaluate(&c, &[claim("1000")]);
        // Both rules match; STOP wins lattice merge.
        assert!(matches!(out.decision, Decision::Stop));
        assert_eq!(out.matched_rule_ids.len(), 2);
    }

    #[test]
    fn unmatched_budget_no_op() {
        let c = make_contract(vec![rule_v1alpha1(
            "for-other-budget",
            Condition {
                budget_id: Uuid::parse_str("33333333-3333-4333-8333-333333333333").unwrap(),
                claim_amount_atomic_gt: Some("1".into()),
                claim_amount_atomic_gte: None,
            },
            Decision::Stop,
            "X",
        )]);
        let out = evaluate(&c, &[claim("999999")]);
        assert!(matches!(out.decision, Decision::Continue));
    }

    // =================================================================
    // SLICE_02 — RUN_* code pass-through tests (per spec §7.1 + §3.4)
    // =================================================================

    /// Build a RUN_* rule with explicit projection action. SLICE_02
    /// pass-through assumes the bundle loader validated the
    /// (policy, action) pair; tests bypass the loader to exercise
    /// the projection directly.
    fn run_rule(
        id: &str,
        run_code: RunCode,
        action: RunProjectionAction,
    ) -> Rule {
        Rule {
            id: id.into(),
            when: Condition {
                budget_id: budget_id(),
                // Match on any positive claim so the rule fires
                // unconditionally in the synthetic test.
                claim_amount_atomic_gt: Some("0".into()),
                claim_amount_atomic_gte: None,
            },
            then: Action {
                // §3.4: the lattice `decision` field is essentially
                // ignored when reasonCode is RUN_*. We set Stop here
                // to demonstrate that the projection from `action`
                // OVERRIDES this value (StopRunProjection in spirit;
                // STOP via lattice mapping per §3.4).
                decision: Decision::Stop,
                reason_code: run_code.as_str().into(),
                approver_role: None,
            },
            run_projection_action: action,
        }
    }

    #[test]
    fn run_code_block_next_call_projects_to_stop() {
        let c = make_contract_with_policy(
            vec![run_rule(
                "run-budget-projection",
                RunCode::BudgetProjectionExceeded,
                RunProjectionAction::BlockNextCall,
            )],
            PredictionPolicy::EmpiricalRunCeiling,
        );
        let out = evaluate(&c, &[claim("100")]);
        assert!(matches!(out.decision, Decision::Stop));
        assert_eq!(
            out.reason_codes,
            vec!["RUN_BUDGET_PROJECTION_EXCEEDED".to_string()]
        );
        assert_eq!(out.matched_rule_ids.len(), 1);
    }

    #[test]
    fn run_code_require_approval_projects_to_require_approval() {
        let c = make_contract_with_policy(
            vec![run_rule(
                "run-drift",
                RunCode::DriftDetected,
                RunProjectionAction::RequireApproval,
            )],
            PredictionPolicy::EmpiricalRunCeiling,
        );
        let out = evaluate(&c, &[claim("100")]);
        assert!(matches!(out.decision, Decision::RequireApproval));
        assert_eq!(out.reason_codes, vec!["RUN_DRIFT_DETECTED".to_string()]);
    }

    #[test]
    fn run_code_alert_only_projects_to_continue() {
        let c = make_contract_with_policy(
            vec![run_rule(
                "run-steps",
                RunCode::StepsExceeded,
                RunProjectionAction::AlertOnly,
            )],
            PredictionPolicy::ShadowOnly,
        );
        let out = evaluate(&c, &[claim("100")]);
        // §5.3 ALERT_ONLY → Continue (audit row emitted upstream).
        assert!(matches!(out.decision, Decision::Continue));
        assert_eq!(out.reason_codes, vec!["RUN_STEPS_EXCEEDED".to_string()]);
        // Rule still attributed (calibration-report needs this).
        assert_eq!(out.matched_rule_ids.len(), 1);
    }

    #[test]
    fn run_code_dispatch_all_three_codes() {
        // Spec §8.1: evaluator routes all 3 RUN_* codes correctly.
        for code in [
            RunCode::BudgetProjectionExceeded,
            RunCode::DriftDetected,
            RunCode::StepsExceeded,
        ] {
            let c = make_contract_with_policy(
                vec![run_rule(
                    "synthetic",
                    code,
                    RunProjectionAction::BlockNextCall,
                )],
                PredictionPolicy::EmpiricalRunCeiling,
            );
            let out = evaluate(&c, &[claim("100")]);
            assert!(matches!(out.decision, Decision::Stop));
            assert_eq!(out.reason_codes, vec![code.as_str().to_string()]);
        }
    }

    #[test]
    fn run_code_overrides_rule_then_decision() {
        // §3.4 invariant: rule author cannot fight projection by
        // writing `decision: continue` next to a RUN_* code. Build
        // a rule whose lattice `decision: Continue` would normally
        // be open-pass — the RUN_* path must project to Stop via
        // BlockNextCall.
        let c = make_contract_with_policy(
            vec![Rule {
                id: "trojan".into(),
                when: Condition {
                    budget_id: budget_id(),
                    claim_amount_atomic_gt: Some("0".into()),
                    claim_amount_atomic_gte: None,
                },
                then: Action {
                    decision: Decision::Continue, // ignored by §3.4
                    reason_code: "RUN_BUDGET_PROJECTION_EXCEEDED".into(),
                    approver_role: None,
                },
                run_projection_action: RunProjectionAction::BlockNextCall,
            }],
            PredictionPolicy::EmpiricalRunCeiling,
        );
        let out = evaluate(&c, &[claim("100")]);
        // Projection wins — decision = Stop, not Continue.
        assert!(matches!(out.decision, Decision::Stop));
    }

    #[test]
    fn run_code_coexists_with_per_call_rule_lattice_merge() {
        // §3.4: per-call lattice and run-projection rules can fire on
        // the same claim. Most-restrictive wins via the same lattice.
        // Setup: a per-call BUDGET_EXHAUSTED Stop + a RUN_DRIFT_DETECTED
        // AlertOnly (projects to Continue). Stop must win.
        let c = make_contract_with_policy(
            vec![
                rule_v1alpha1(
                    "per-call-stop",
                    Condition {
                        budget_id: budget_id(),
                        claim_amount_atomic_gt: Some("0".into()),
                        claim_amount_atomic_gte: None,
                    },
                    Decision::Stop,
                    "BUDGET_EXHAUSTED",
                ),
                run_rule(
                    "run-alert",
                    RunCode::DriftDetected,
                    RunProjectionAction::AlertOnly,
                ),
            ],
            PredictionPolicy::EmpiricalRunCeiling,
        );
        let out = evaluate(&c, &[claim("100")]);
        // Stop > Continue lattice precedence.
        assert!(matches!(out.decision, Decision::Stop));
        // Both reason_codes accumulated (per-call BUDGET_EXHAUSTED +
        // RUN_DRIFT_DETECTED). Order is rule-iteration order.
        assert_eq!(out.reason_codes.len(), 2);
        assert!(out.reason_codes.contains(&"BUDGET_EXHAUSTED".to_string()));
        assert!(out
            .reason_codes
            .contains(&"RUN_DRIFT_DETECTED".to_string()));
    }
}
