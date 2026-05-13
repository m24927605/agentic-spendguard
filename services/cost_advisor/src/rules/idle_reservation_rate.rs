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
/// So this rule is **blocked**, just like the other three. The P1
/// path: build a ledger view (call it P0.6) that surfaces
/// `(reservation_id, derived_state, ttl_seconds)` by joining
/// `reservations` with `audit_outbox` release events. The rule SQL
/// then reads the view, not the raw projection.
///
/// declared_input_fields below tracks fields the rule WILL read once
/// the view exists. Today the rule registers as non-ready and the P1
/// runtime never schedules it.
pub const DECLARED_INPUT_FIELDS: &[&str] = &[
    // From the P0.6 derived view (NOT yet built):
    "reservations_with_ttl_status_v1.derived_state",
    "reservations_with_ttl_status_v1.ttl_seconds",
    // Direct columns:
    "ledger.reservations.current_state",
    "ledger.reservations.ttl_expires_at",
    "ledger.reservations.created_at",
    "canonical_events.decision_id",
];

/// Placeholder SQL. The real SQL goes in
/// `rules/detected_waste/idle_reservation_rate_v1.sql` in P1 and is
/// loaded via `include_str!`. The marker comment must match
/// [`crate::sql_rule::PLACEHOLDER_SQL_MARKER`] so the runtime
/// recognizes this rule as not-ready and refuses to register it.
const RULE_SQL: &str = "-- placeholder; real SQL lands in P1\nSELECT 1 WHERE FALSE;\n";

/// Static descriptor for the rule. **Not registered in P0** — the P1
/// runtime calls [`SqlCostRule::is_ready`] before registering and the
/// placeholder fails that gate. The descriptor exists so the rule's
/// metadata (id, version, declared fields, category) is testable and
/// available for documentation tooling.
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
    fn placeholder_rule_is_not_ready() {
        // Codex r5 P1-7: P0 placeholder MUST NOT pass is_ready.
        // The P1 runtime gates registration on this — if it ever
        // flips to true here without real SQL, the runtime would
        // register a rule whose evaluate() returns Err.
        let r = descriptor();
        assert!(
            !r.is_ready(),
            "P0 placeholder rule must report is_ready()=false; \
             ensure RULE_SQL contains PLACEHOLDER_SQL_MARKER"
        );
    }
}
