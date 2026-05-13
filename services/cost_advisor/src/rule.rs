//! `CostRule` trait — spec §5.4.
//!
//! Trait surface is frozen at P0; v0.1 ships only SQL-backed impls (via
//! [`crate::SqlCostRule`]), but the trait is shaped so that v0.2+
//! native-Rust rules implement it without a breaking change. Per spec:
//! "No trait breaking change planned through v1.0."

use std::time::Duration;

use crate::proto::cost_advisor::v1::FindingEvidence;

/// Rule classification — matches the proto `FindingCategory` enum so
/// `category()` results can be projected directly into the
/// `FindingEvidence.category` wire field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    DetectedWaste,
    OptimizationHypothesis,
}

/// Evaluation context handed to each rule per evaluation cycle. Carries
/// the tenant + database handle + time bucket so the rule SQL can
/// parameterize without each rule re-deriving these.
///
/// P0 leaves the actual fields TBD because the runtime (which builds
/// this) lands in P1. Defined as a placeholder struct so the trait
/// signature is stable.
#[allow(dead_code)]
pub struct EvaluationContext {
    pub tenant_id: uuid::Uuid,
    pub now: chrono::DateTime<chrono::Utc>,
    pub pool: sqlx::PgPool,
}

/// Rule trait. Implementors:
///   - SQL-backed rules via [`crate::SqlCostRule`] (v0.1).
///   - Native-Rust rules (v0.2+; cross-run state machines etc.).
#[async_trait::async_trait]
pub trait CostRule: Send + Sync {
    fn rule_id(&self) -> &'static str;

    /// Strictly positive. Bumping this MUST also bump the trailing
    /// `_vN` in [`Self::rule_id`] (spec §11.5 A6).
    fn rule_version(&self) -> u32;

    fn category(&self) -> Category;

    /// Fields the rule reads from `canonical_events` / ledger joins.
    /// Validated at startup against the schema audit (spec §11.5 A2);
    /// rule fails to register if a declared field isn't present.
    ///
    /// Audit-report §5: today this is enforced by static check against
    /// a curated allowlist; full live audit hookup lands when the
    /// runtime ships in P1.
    fn declared_input_fields(&self) -> &'static [&'static str];

    /// Whether this rule needs `cost_baselines` populated (Tier 2).
    fn requires_baselines(&self) -> bool {
        false
    }

    /// Suppression: if a finding from this rule fired within
    /// `cooldown` of a new candidate, the new one is suppressed (deduped
    /// at fingerprint level). Default 0 = no cooldown.
    fn cooldown(&self) -> Duration {
        Duration::ZERO
    }

    /// Per-tenant rate limit for THIS rule per day (caps noisy rules).
    fn per_tenant_daily_cap(&self) -> Option<u32> {
        None
    }

    /// Stable identity for dedup. Default mirrors
    /// [`crate::fingerprint::compute`] which is what the FindingEvidence
    /// wire `fingerprint` field carries.
    fn dedupe_key(&self, finding: &FindingEvidence) -> String {
        finding.fingerprint.clone()
    }

    /// The actual evaluation. Returns 0..N findings per call.
    async fn evaluate(&self, ctx: &EvaluationContext) -> anyhow::Result<Vec<FindingEvidence>>;
}
