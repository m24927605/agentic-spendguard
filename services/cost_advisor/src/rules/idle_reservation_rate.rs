//! `idle_reservation_rate_v1` — placeholder rule definition.
//!
//! This is the ONE rule the P0 audit report
//! (`docs/specs/cost-advisor-p0-audit-report.md` §4) found fireable
//! under the current schema. The real SQL body lives in
//! `services/cost_advisor/rules/detected_waste/idle_reservation_rate_v1.sql`
//! and lands in P1.
//!
//! P0 declares the rule as a [`crate::SqlCostRule`] constant so that:
//!   * Its `declared_input_fields()` set is reviewed alongside the
//!     migrations (Step 3) and won't silently drift.
//!   * `cargo check` exercises the trait + proto types end-to-end.

use crate::rule::Category;
use crate::sql_rule::SqlCostRule;

/// Fields this rule reads. Validated against the live schema at startup
/// (spec §11.5 A2). All of these are confirmed present today by the P0
/// audit:
///   * `ledger.reservations.latest_state` — populated by ledger commit /
///     release / ttl-sweep paths.
///   * `ledger.reservations.ttl_seconds` — set at reserve time.
///   * `canonical_events.decision_id` — populated on every audit row.
///
/// Note the audit-report scope cut: `failed_retry_burn_v1`,
/// `runaway_loop_v1`, and `tool_call_repeated_v1` are NOT registered in
/// P0 because their declared fields (prompt_hash, agent_id, tool_name,
/// tool_args_hash) are 0% populated today.
pub const DECLARED_INPUT_FIELDS: &[&str] = &[
    "ledger.reservations.latest_state",
    "ledger.reservations.ttl_seconds",
    "canonical_events.decision_id",
];

/// Placeholder SQL. The real SQL goes in
/// `rules/detected_waste/idle_reservation_rate_v1.sql` in P1 and is
/// loaded via `include_str!`. P0 keeps an empty `SELECT` so the trait
/// surface compiles and `cargo check` passes without a file the P1
/// runtime hasn't written yet.
const RULE_SQL: &str = "-- placeholder; real SQL lands in P1\nSELECT 1 WHERE FALSE;\n";

pub fn rule() -> SqlCostRule {
    SqlCostRule::new(
        "idle_reservation_rate_v1",
        1,
        Category::DetectedWaste,
        DECLARED_INPUT_FIELDS,
        RULE_SQL,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CostRule;

    #[test]
    fn rule_id_matches_versioning_pattern() {
        let r = rule();
        assert!(
            r.rule_id().ends_with("_v1"),
            "rule_id MUST end with _vN per spec §4.0 / §11.5 A6"
        );
        assert_eq!(r.rule_version(), 1);
    }

    #[test]
    fn category_is_detected_waste() {
        let r = rule();
        assert_eq!(r.category(), Category::DetectedWaste);
    }

    #[test]
    fn declared_input_fields_non_empty() {
        let r = rule();
        assert!(!r.declared_input_fields().is_empty());
    }
}
