//! SpendGuard Cost Advisor — P0 skeleton.
//!
//! See `docs/specs/cost-advisor-spec.md` (the bible) and
//! `docs/specs/cost-advisor-p0-audit-report.md` (the audit that revised
//! v0.1 scope to a single rule + a P0.5 enrichment workstream).
//!
//! Public surface today is the [`rule::CostRule`] trait + the
//! [`sql_rule::SqlCostRule`] adapter that wraps a `.sql` file into a
//! `CostRule` impl. The runtime that orchestrates rule evaluation
//! lands in P1; this crate exists in P0 so the trait surface can be
//! frozen alongside the proto contract.

pub mod proto {
    pub mod cost_advisor {
        pub mod v1 {
            tonic::include_proto!("spendguard.cost_advisor.v1");
        }
    }
}

pub mod fingerprint;
pub mod patch_validator;
pub mod proposal_writer;
pub mod rule;
pub mod runtime;
pub mod sql_rule;
pub mod rules;

pub use rule::{Category, CostRule, EvaluationContext};
pub use sql_rule::SqlCostRule;
