//! `failed_retry_burn_v1` — Cost Advisor P1.5 second fireable rule.
//!
//! See `services/cost_advisor/rules/detected_waste/failed_retry_burn_v1.sql`
//! for the rule body. Runtime decoder lives in
//! `services/cost_advisor/src/runtime.rs` (decode_failed_retry_burn).

use crate::rule::{Category, TargetDb};
use crate::sql_rule::SqlCostRule;

pub const DECLARED_INPUT_FIELDS: &[&str] = &[
    "canonical_events.tenant_id",
    "canonical_events.run_id",
    "canonical_events.event_type",
    "canonical_events.failure_class",
    "canonical_events.payload_json.data_b64.prompt_hash",
    "canonical_events.payload_json.data_b64.estimated_amount_atomic",
    "canonical_events.decision_id",
    "canonical_events.event_time",
];

const RULE_SQL: &str = include_str!(
    "../../rules/detected_waste/failed_retry_burn_v1.sql"
);

pub fn descriptor() -> SqlCostRule {
    SqlCostRule::new_with_db(
        "failed_retry_burn_v1",
        1,
        Category::DetectedWaste,
        TargetDb::Canonical,
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
        assert_eq!(descriptor().rule_version(), 1);
    }

    #[test]
    fn rule_is_ready() {
        assert!(descriptor().is_ready());
    }

    #[test]
    fn sql_uses_safe_decode() {
        // Lock the contract: rule body MUST use the
        // cost_advisor_safe_decode_payload helper (migration 0012)
        // instead of naive convert_from(decode(...)) which would
        // raise on malformed payloads.
        assert!(descriptor().sql().contains("cost_advisor_safe_decode_payload"));
    }

    #[test]
    fn sql_groups_by_run_and_prompt() {
        let sql = descriptor().sql();
        assert!(sql.contains("GROUP BY run_id"));
        assert!(sql.contains("prompt_hash"));
    }

    #[test]
    fn sql_filters_billed_failure_classes() {
        // The 4 ✅ billed-waste classes per spec §5.1.2.
        let sql = descriptor().sql();
        for cls in [
            "provider_5xx",
            "provider_4xx_billed",
            "malformed_json_response",
            "timeout_billed",
        ] {
            assert!(
                sql.contains(cls),
                "rule SQL must reference billed failure_class {}",
                cls
            );
        }
    }
}
