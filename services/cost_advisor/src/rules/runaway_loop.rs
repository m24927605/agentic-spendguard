//! `runaway_loop_v1` — Cost Advisor P1.5 third fireable rule.
//!
//! See `services/cost_advisor/rules/detected_waste/runaway_loop_v1.sql`
//! for the rule body. Runtime decoder lives in
//! `services/cost_advisor/src/runtime.rs` (decode_runaway_loop).

use crate::rule::Category;
use crate::sql_rule::SqlCostRule;

pub const DECLARED_INPUT_FIELDS: &[&str] = &[
    "canonical_events.tenant_id",
    "canonical_events.run_id",
    "canonical_events.event_type",
    "canonical_events.failure_class",
    "canonical_events.payload_json.data_b64.prompt_hash",
    "canonical_events.decision_id",
    "canonical_events.event_time",
];

const RULE_SQL: &str = include_str!(
    "../../rules/detected_waste/runaway_loop_v1.sql"
);

pub fn descriptor() -> SqlCostRule {
    SqlCostRule::new(
        "runaway_loop_v1",
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
    fn rule_id_pattern_ok() {
        assert!(descriptor().rule_id().ends_with("_v1"));
    }

    #[test]
    fn rule_is_ready() {
        assert!(descriptor().is_ready());
    }

    #[test]
    fn sql_uses_safe_decode() {
        assert!(descriptor().sql().contains("cost_advisor_safe_decode_payload"));
    }

    #[test]
    fn sql_filters_non_billed_failure_classes() {
        // Orthogonal to failed_retry_burn_v1: runaway_loop_v1 only
        // fires when calls are NOT in a billed-failure class.
        let sql = descriptor().sql();
        assert!(
            sql.contains("failure_class IS NULL"),
            "runaway_loop must allow failure_class IS NULL"
        );
        assert!(
            sql.contains("'unknown'"),
            "runaway_loop must allow failure_class = 'unknown'"
        );
    }

    #[test]
    fn sql_uses_60_second_window() {
        assert!(descriptor().sql().contains("60 seconds"));
    }
}
