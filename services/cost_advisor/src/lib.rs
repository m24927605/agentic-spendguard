//! SpendGuard Cost Advisor — v0.1 crate.
//!
//! See `docs/specs/cost-advisor-spec.md` (the bible),
//! `docs/specs/cost-advisor-p0-audit-report.md` (the audit that drove
//! the v0.1 scope cut + P0.5/P0.6 workstreams), and
//! `services/cost_advisor/docs/control-plane-integration.md`.
//!
//! Public surface:
//!   * [`rule::CostRule`] trait + [`sql_rule::SqlCostRule`] adapter
//!     wrap a `.sql` file into a `CostRule` impl.
//!   * [`runtime::evaluate_tenant_day`] runs the rule registry for
//!     one (tenant, date) bucket and UPSERTs into cost_findings.
//!   * [`patch_validator`] + [`proposal_writer`] (CA-P3 + P3.1)
//!     handle the cost_advisor → approval_requests write path.

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
