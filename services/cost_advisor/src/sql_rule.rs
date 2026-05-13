//! `SqlCostRule` — wraps a `.sql` file into a `CostRule` impl.
//!
//! Spec §5.4: "V0 ships SQL-only `CostRule` impls (a generic
//! `SqlCostRule` adapter wraps any `.sql` file)."
//!
//! Rule files live in `services/cost_advisor/rules/<category>/<rule_id>.sql`.
//! Each file MUST return columns matching the keys in [`SqlRuleRow`] so
//! this adapter can decode rows directly into `FindingEvidence`. The
//! v0.1 runtime (P1) will scan that directory, instantiate one
//! `SqlCostRule` per file, and register them with the rule registry.
//!
//! P0 ships the trait and the adapter; the registry + scanner live in
//! P1.

use std::time::Duration;

use crate::proto::cost_advisor::v1::FindingEvidence;
use crate::rule::{Category, CostRule, EvaluationContext};

/// One row decoded from a rule's `.sql` SELECT result. Each column maps
/// directly onto a `FindingEvidence` proto field. Implementation lives
/// alongside the runtime in P1 — declared here as an opaque type
/// placeholder so the trait surface compiles standalone.
#[allow(dead_code)]
pub struct SqlRuleRow {
    pub fingerprint: String,
    pub scope_json: serde_json::Value,
    pub metrics_json: serde_json::Value,
    pub decision_refs: Vec<uuid::Uuid>,
    pub waste_estimate_json: Option<serde_json::Value>,
    pub time_bucket: String,
}

/// A SQL-backed rule. The lifecycle:
///   1. Crate loader reads `rules/<category>/<rule_id>.sql` at startup.
///   2. Top-of-file comment is parsed for declared input fields and
///      validated against schema audit results (spec §11.5 A2).
///   3. At each evaluation tick, the rule's SQL is executed against the
///      `EvaluationContext.pool` with `(tenant_id, time_bucket_start,
///      time_bucket_end)` parameters.
///   4. Each returned row is decoded into a [`FindingEvidence`] proto
///      message and emitted upstream for severity classification + dedup
///      + UPSERT into `cost_findings`.
///
/// Today (P0): the struct compiles standalone; rows-to-proto decode and
/// the actual `sqlx::query` invocation land in P1.
///
/// Readiness gate (codex r5 P1-7): a rule is only "ready" if its SQL
/// is non-placeholder. The runtime checks [`is_ready`] before
/// registering — a placeholder rule never reaches the evaluation tick,
/// so a stub like `SELECT 1 WHERE FALSE` cannot trip health checks or
/// produce confusing empty findings.
pub struct SqlCostRule {
    rule_id: &'static str,
    rule_version: u32,
    category: Category,
    declared_input_fields: &'static [&'static str],
    sql: &'static str,
}

/// Marker comment that the rule loader matches to decide whether the
/// SQL body is a placeholder. P1's runtime refuses to register any
/// rule whose SQL contains this marker (see [`SqlCostRule::is_ready`]).
pub const PLACEHOLDER_SQL_MARKER: &str = "-- placeholder; real SQL lands in P1";

impl SqlCostRule {
    pub const fn new(
        rule_id: &'static str,
        rule_version: u32,
        category: Category,
        declared_input_fields: &'static [&'static str],
        sql: &'static str,
    ) -> Self {
        Self {
            rule_id,
            rule_version,
            category,
            declared_input_fields,
            sql,
        }
    }

    /// Raw SQL text. Exposed for tests + the P1 runtime that actually
    /// executes the query.
    pub fn sql(&self) -> &'static str {
        self.sql
    }

    /// `true` once the rule has real (non-placeholder) SQL wired.
    /// The P1 runtime gates registration on this — a placeholder rule
    /// never reaches the evaluation tick.
    pub fn is_ready(&self) -> bool {
        !self.sql.contains(PLACEHOLDER_SQL_MARKER)
    }
}

#[async_trait::async_trait]
impl CostRule for SqlCostRule {
    fn rule_id(&self) -> &'static str {
        self.rule_id
    }

    fn rule_version(&self) -> u32 {
        self.rule_version
    }

    fn category(&self) -> Category {
        self.category
    }

    fn declared_input_fields(&self) -> &'static [&'static str] {
        self.declared_input_fields
    }

    fn cooldown(&self) -> Duration {
        // Default: rely on fingerprint UPSERT dedup.
        Duration::ZERO
    }

    async fn evaluate(&self, _ctx: &EvaluationContext) -> anyhow::Result<Vec<FindingEvidence>> {
        // Codex r5 P1-7: if the runtime ever evaluates a placeholder
        // rule, log + return Ok(empty) rather than Err. The runtime's
        // registration path SHOULD have gated on `is_ready()` first,
        // but defense-in-depth here keeps a misconfigured runtime
        // from tripping health checks on a stub.
        if !self.is_ready() {
            tracing::warn!(
                rule_id = self.rule_id,
                "SqlCostRule::evaluate called on placeholder rule; returning no findings. \
                 P1 runtime should gate registration on SqlCostRule::is_ready()."
            );
            return Ok(Vec::new());
        }

        // P1: execute self.sql() against ctx.pool, decode rows into
        // FindingEvidence per the `SqlRuleRow` shape, return.
        Err(anyhow::anyhow!(
            "SqlCostRule::evaluate is_ready() but row decode lands in P1 (rule_id={})",
            self.rule_id
        ))
    }
}
