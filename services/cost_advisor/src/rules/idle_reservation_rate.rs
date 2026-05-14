//! `idle_reservation_rate_v1` — fireable rule descriptor.
//!
//! SQL body lives in
//! `services/cost_advisor/rules/detected_waste/idle_reservation_rate_v1.sql`.
//! CA-P0.6 unblocked it by shipping `reservations_with_ttl_status_v1`.
//! CA-P1 wired it into the runtime. CA-P3.1 made it budget-scoped
//! (per-budget grouping + identity-pinned RFC-6902 patch emission).

use crate::rule::Category;
use crate::sql_rule::SqlCostRule;

/// Fields this rule reads. Validated against the live schema at startup
/// (spec §11.5 A2). Codex r5 P1-1 found that the audit-report's claim
/// "this rule is fireable today" was wrong:
///   * `ledger.reservations.current_state` exists (NOT `latest_state`).
///   * Allowed values: `reserved | committed | released | overrun_debt`.
///     There is **no** `ttl_expired` state — TTL expiry is encoded as
///     a `release` event with `reason='TTL_EXPIRED'` on the audit chain.
///   * There is **no** `ttl_seconds` column; the table has
///     `ttl_expires_at` and `created_at`.
///   * There is no config home for `min_ttl_for_finding`.
///
/// P0.6 shipped the derived view; this rule reads from it. CA-P3.1
/// added per-budget grouping (codex CA-P3 r2 P1: positional patches
/// without budget identity were unsafe) so the rule now projects
/// `budget_id` and the decoder emits an identity-pinned 2-op patch.
pub const DECLARED_INPUT_FIELDS: &[&str] = &[
    "reservations_with_ttl_status_v1.budget_id",
    "reservations_with_ttl_status_v1.derived_state",
    "reservations_with_ttl_status_v1.ttl_seconds",
    "ledger.reservations.current_state",
    "ledger.reservations.ttl_expires_at",
    "ledger.reservations.created_at",
    "canonical_events.decision_id",
];

/// Real rule SQL (P1). Loaded at compile time via `include_str!` so
/// the rule body ships alongside the binary and there's no runtime
/// filesystem dependency. The .sql file is canonical; this constant
/// is just the in-binary copy.
const RULE_SQL: &str = include_str!(
    "../../rules/detected_waste/idle_reservation_rate_v1.sql"
);

/// Static descriptor for the rule. With P1 the SQL is non-placeholder
/// so [`SqlCostRule::is_ready`] returns `true` and the runtime
/// registers + evaluates this rule.
pub fn descriptor() -> SqlCostRule {
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
        let r = descriptor();
        assert!(
            r.rule_id().ends_with("_v1"),
            "rule_id MUST end with _vN per spec §4.0 / §11.5 A6"
        );
        assert_eq!(r.rule_version(), 1);
    }

    #[test]
    fn category_is_detected_waste() {
        let r = descriptor();
        assert_eq!(r.category(), Category::DetectedWaste);
    }

    #[test]
    fn declared_input_fields_non_empty() {
        let r = descriptor();
        assert!(!r.declared_input_fields().is_empty());
    }

    #[test]
    fn p1_rule_is_ready() {
        // P1: real SQL ships via include_str!; is_ready() flips to
        // true and the runtime registers + evaluates this rule.
        let r = descriptor();
        assert!(
            r.is_ready(),
            "P1 rule MUST report is_ready()=true; check that \
             RULE_SQL does NOT contain PLACEHOLDER_SQL_MARKER"
        );
    }

    #[test]
    fn rule_sql_references_view() {
        // Lock the contract: the SQL must read from
        // reservations_with_ttl_status_v1 (CA-P0.6 view). If a future
        // edit drops this reference, the rule would silently start
        // hitting the raw reservations table again and the
        // derived_state column wouldn't exist.
        let r = descriptor();
        assert!(
            r.sql().contains("reservations_with_ttl_status_v1"),
            "rule SQL must read from the CA-P0.6 view"
        );
    }

    #[test]
    fn rule_sql_groups_by_budget() {
        // CA-P3.1: rule must GROUP BY budget_id so each row maps to
        // exactly one budget. The decoder reads `budget_id` from the
        // row and pins identity in the proposed patch's test op.
        let r = descriptor();
        let sql = r.sql();
        assert!(
            sql.contains("GROUP BY v.tenant_id, v.budget_id"),
            "rule SQL must GROUP BY budget for per-budget findings"
        );
        assert!(
            sql.contains("v.budget_id::TEXT"),
            "rule SQL must project budget_id"
        );
    }
}
