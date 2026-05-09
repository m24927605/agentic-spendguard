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

use num_bigint::BigInt;

use crate::contract::types::{Contract, EvalOutcome, Rule};
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::types::{Action, Budget, Condition, Rule};
    use uuid::Uuid;

    fn budget_id() -> Uuid {
        Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap()
    }

    fn make_contract(rules: Vec<Rule>) -> Contract {
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
        let c = make_contract(vec![Rule {
            id: "hard-cap".into(),
            when: Condition {
                budget_id: budget_id(),
                claim_amount_atomic_gt: Some("1000000000".into()),
                claim_amount_atomic_gte: None,
            },
            then: Action {
                decision: Decision::Stop,
                reason_code: "BUDGET_EXHAUSTED".into(),
                approver_role: None,
            },
        }]);
        let out = evaluate(&c, &[claim("2000000000")]);
        assert!(matches!(out.decision, Decision::Stop));
        assert_eq!(out.reason_codes, vec!["BUDGET_EXHAUSTED".to_string()]);
    }

    #[test]
    fn most_restrictive_wins() {
        let c = make_contract(vec![
            Rule {
                id: "approval".into(),
                when: Condition {
                    budget_id: budget_id(),
                    claim_amount_atomic_gt: None,
                    claim_amount_atomic_gte: Some("100".into()),
                },
                then: Action {
                    decision: Decision::RequireApproval,
                    reason_code: "AMOUNT_OVER_THRESHOLD".into(),
                    approver_role: Some("admin".into()),
                },
            },
            Rule {
                id: "hard-cap".into(),
                when: Condition {
                    budget_id: budget_id(),
                    claim_amount_atomic_gt: Some("500".into()),
                    claim_amount_atomic_gte: None,
                },
                then: Action {
                    decision: Decision::Stop,
                    reason_code: "BUDGET_EXHAUSTED".into(),
                    approver_role: None,
                },
            },
        ]);
        let out = evaluate(&c, &[claim("1000")]);
        // Both rules match; STOP wins lattice merge.
        assert!(matches!(out.decision, Decision::Stop));
        assert_eq!(out.matched_rule_ids.len(), 2);
    }

    #[test]
    fn unmatched_budget_no_op() {
        let c = make_contract(vec![Rule {
            id: "for-other-budget".into(),
            when: Condition {
                budget_id: Uuid::parse_str("33333333-3333-4333-8333-333333333333").unwrap(),
                claim_amount_atomic_gt: Some("1".into()),
                claim_amount_atomic_gte: None,
            },
            then: Action {
                decision: Decision::Stop,
                reason_code: "X".into(),
                approver_role: None,
            },
        }]);
        let out = evaluate(&c, &[claim("999999")]);
        assert!(matches!(out.decision, Decision::Continue));
    }
}
