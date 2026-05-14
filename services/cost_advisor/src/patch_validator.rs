//! CA-P3 / owner-ack #55: RFC-6902 patch allowlist validator.
//!
//! Mirrors `cost_advisor_validate_proposed_dsl_patch` in migration
//! 0043. Rust-side validation runs BEFORE the SQL INSERT so the
//! runtime can fail fast with a structured error; the DB-side CHECK
//! constraint is the authoritative gate.
//!
//! Why two layers (codex CA-P1.6 r4 spirit — defense in depth):
//!   * Rust validator: callers (test harness, future REST API)
//!     get a structured error explaining WHICH op/path failed.
//!   * DB CHECK: guarantees no out-of-allowlist patch can land in
//!     `approval_requests` even if a buggy/malicious caller bypasses
//!     the Rust path.
//!
//! Allowed RFC-6902 ops:
//!   * `replace` — for the 5 mutation paths below.
//!   * `test` (CA-P3.1) — for identity pinning on
//!     `/spec/budgets/<i>/id` with a lowercase hyphenated UUID value.
//!
//! Allowed JSON Pointer paths (each `<i>` is an RFC 6901 array index
//! — `0` or `[1-9][0-9]*`, no leading zeros). All paths are under
//! `/spec/` matching the contract YAML schema parsed by
//! `services/sidecar/src/contract/parse.rs`:
//!   replace:
//!     1. `/spec/budgets/<i>/limit_amount_atomic`
//!     2. `/spec/budgets/<i>/reservation_ttl_seconds`
//!     3. `/spec/rules/<i>/when/claim_amount_atomic_gt`
//!     4. `/spec/rules/<i>/then/decision`
//!     5. `/spec/rules/<i>/then/approver_role`
//!   test:
//!     6. `/spec/budgets/<i>/id`
//!
//! Same-index pinning invariant (CA-P3.1 r1 P2): any replace op on
//! `/spec/budgets/<i>/*` MUST be preceded earlier in the patch by a
//! test op on `/spec/budgets/<i>/id` at the same `<i>`. Rule replaces
//! don't require pinning (only one rule path emits patches today; if
//! future rule paths add positional-mutation risk, a similar
//! /spec/rules/<i>/id test op will be added).
//!
//! Value-schema gate per leaf (codex CA-P3 r1 P2):
//!   * limit_amount_atomic, claim_amount_atomic_gt: string of 1..38 digits
//!   * reservation_ttl_seconds: integer in [1, 86400]
//!   * decision: enum {STOP, REQUIRE_APPROVAL, DEGRADE, CONTINUE, SKIP}
//!   * approver_role: non-empty string matching `[A-Za-z0-9_-]+`, ≤ 64

use serde_json::Value;
use thiserror::Error;

const MAX_PATCH_OPS: usize = 8;

const DECISIONS: &[&str] = &["STOP", "REQUIRE_APPROVAL", "DEGRADE", "CONTINUE", "SKIP"];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PatchValidationError {
    #[error("patch must be a JSON array (RFC-6902 shape)")]
    NotArray,
    #[error("patch must contain at least one op")]
    Empty,
    #[error("patch has {len} ops; max is {max}")]
    TooManyOps { len: usize, max: usize },
    #[error("op #{index}: must be a JSON object")]
    OpNotObject { index: usize },
    #[error("op #{index}: missing or non-string `op` field")]
    OpMissingOp { index: usize },
    #[error("op #{index}: `op` must be `replace` or `test`, got `{op}`")]
    OpNotReplace { index: usize, op: String },
    #[error("op #{index}: missing or non-string `path` field")]
    OpMissingPath { index: usize },
    #[error("op #{index}: path `{path}` is not in the cost_advisor allowlist")]
    OpPathNotAllowed { index: usize, path: String },
    #[error("op #{index}: replace op missing `value` field")]
    OpMissingValue { index: usize },
    #[error("op #{index}: value for `{leaf}` failed schema check: {reason}")]
    OpValueSchema {
        index: usize,
        leaf: String,
        reason: String,
    },
    #[error("op #{index}: replace on /spec/budgets/{idx}/* requires a preceding test op on /spec/budgets/{idx}/id at the same index")]
    BudgetReplaceMissingTestPin { index: usize, idx: u32 },
}

/// Validate an RFC-6902 patch against the cost_advisor allowlist.
///
/// Returns `Ok(())` if every op passes; `Err` with structured detail
/// otherwise. Pure function — no I/O.
pub fn validate(patch: &Value) -> Result<(), PatchValidationError> {
    let arr = patch.as_array().ok_or(PatchValidationError::NotArray)?;
    if arr.is_empty() {
        return Err(PatchValidationError::Empty);
    }
    if arr.len() > MAX_PATCH_OPS {
        return Err(PatchValidationError::TooManyOps {
            len: arr.len(),
            max: MAX_PATCH_OPS,
        });
    }
    let mut pinned_budget_indices: Vec<u32> = Vec::new();
    for (i, op) in arr.iter().enumerate() {
        validate_op(i, op, &mut pinned_budget_indices)?;
    }
    Ok(())
}

fn validate_op(
    index: usize,
    op: &Value,
    pinned_budget_indices: &mut Vec<u32>,
) -> Result<(), PatchValidationError> {
    let obj = op
        .as_object()
        .ok_or(PatchValidationError::OpNotObject { index })?;

    let op_kind = obj
        .get("op")
        .and_then(Value::as_str)
        .ok_or(PatchValidationError::OpMissingOp { index })?;
    if op_kind != "replace" && op_kind != "test" {
        return Err(PatchValidationError::OpNotReplace {
            index,
            op: op_kind.to_string(),
        });
    }

    let path = obj
        .get("path")
        .and_then(Value::as_str)
        .ok_or(PatchValidationError::OpMissingPath { index })?;

    let value = obj
        .get("value")
        .ok_or(PatchValidationError::OpMissingValue { index })?;

    if op_kind == "test" {
        // CA-P3.1: only `/spec/budgets/<i>/id` is allowlisted for
        // test ops; value MUST be a lowercase hyphenated UUID.
        let Some(budget_idx) = budget_id_test_path_index(path) else {
            return Err(PatchValidationError::OpPathNotAllowed {
                index,
                path: path.to_string(),
            });
        };
        validate_uuid_value(index, "id", value)?;
        pinned_budget_indices.push(budget_idx);
        return Ok(());
    }

    // op_kind == "replace"
    let Some((leaf, budget_idx_opt)) = path_leaf_if_allowed(path) else {
        return Err(PatchValidationError::OpPathNotAllowed {
            index,
            path: path.to_string(),
        });
    };

    // Same-index pinning invariant: budget replaces require a
    // preceding test op pinning the budget's id at the SAME index.
    if let Some(budget_idx) = budget_idx_opt {
        if !pinned_budget_indices.contains(&budget_idx) {
            return Err(PatchValidationError::BudgetReplaceMissingTestPin {
                index,
                idx: budget_idx,
            });
        }
    }

    validate_value(index, leaf, value)
}

/// If `path` is /spec/budgets/<i>/id with valid index, returns Some(i).
fn budget_id_test_path_index(path: &str) -> Option<u32> {
    let rest = path.strip_prefix('/')?;
    let segments: Vec<&str> = rest.split('/').collect();
    match segments.as_slice() {
        ["spec", "budgets", idx, "id"] if is_array_index(idx) => idx.parse().ok(),
        _ => None,
    }
}

fn validate_uuid_value(
    index: usize,
    leaf: &str,
    value: &Value,
) -> Result<(), PatchValidationError> {
    let mismatch = |reason: &str| PatchValidationError::OpValueSchema {
        index,
        leaf: leaf.to_string(),
        reason: reason.to_string(),
    };
    let s = value.as_str().ok_or_else(|| mismatch("must be a string"))?;
    // RFC 4122 lowercase hyphenated: 8-4-4-4-12 hex digits.
    if s.len() != 36 {
        return Err(mismatch("must be a 36-char hyphenated UUID"));
    }
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        let expect_hyphen = matches!(i, 8 | 13 | 18 | 23);
        if expect_hyphen {
            if b != b'-' {
                return Err(mismatch("hyphen mis-positioned"));
            }
        } else if !matches!(b, b'0'..=b'9' | b'a'..=b'f') {
            return Err(mismatch("must be lowercase hex"));
        }
    }
    Ok(())
}

/// If `path` matches one of the 5 allowlisted JSON Pointer patterns,
/// returns `(leaf, Some(budget_idx))` for budget paths (which require
/// pinning) or `(leaf, None)` for rule paths (which don't). All paths
/// are under `/spec/` matching the contract YAML schema.
///
/// Implemented by structural parse rather than regex so a future
/// relaxation is a code addition with a clear branch.
fn path_leaf_if_allowed(path: &str) -> Option<(&'static str, Option<u32>)> {
    let rest = path.strip_prefix('/')?;
    let segments: Vec<&str> = rest.split('/').collect();

    match segments.as_slice() {
        ["spec", "budgets", idx, "limit_amount_atomic"] if is_array_index(idx) => {
            let i = idx.parse().ok()?;
            Some(("limit_amount_atomic", Some(i)))
        }
        ["spec", "budgets", idx, "reservation_ttl_seconds"] if is_array_index(idx) => {
            let i = idx.parse().ok()?;
            Some(("reservation_ttl_seconds", Some(i)))
        }
        ["spec", "rules", idx, "when", "claim_amount_atomic_gt"] if is_array_index(idx) => {
            Some(("claim_amount_atomic_gt", None))
        }
        ["spec", "rules", idx, "then", "decision"] if is_array_index(idx) => {
            Some(("decision", None))
        }
        ["spec", "rules", idx, "then", "approver_role"] if is_array_index(idx) => {
            Some(("approver_role", None))
        }
        _ => None,
    }
}

/// RFC 6901 §4: array indices are `0` OR `[1-9][0-9]*` — no leading
/// zeros (codex CA-P3 r1 P2 tightening; previous version accepted
/// any digit string).
fn is_array_index(s: &str) -> bool {
    match s.as_bytes() {
        [] => false,
        [b'0'] => true,
        [first, rest @ ..] if (b'1'..=b'9').contains(first) => {
            rest.iter().all(|c| c.is_ascii_digit())
        }
        _ => false,
    }
}

/// Validate the `value` JSON against the leaf-specific schema.
///
/// Codex CA-P3 r1 P2: previously we only checked that `value` exists;
/// a buggy / malicious patch could set decision to `null` or 42 and
/// pass. Now each leaf has a shallow schema gate.
fn validate_value(index: usize, leaf: &str, value: &Value) -> Result<(), PatchValidationError> {
    let mismatch = |reason: &str| PatchValidationError::OpValueSchema {
        index,
        leaf: leaf.to_string(),
        reason: reason.to_string(),
    };

    match leaf {
        "limit_amount_atomic" | "claim_amount_atomic_gt" => {
            let s = value.as_str().ok_or_else(|| mismatch("must be a string"))?;
            validate_atomic_amount_str(s).map_err(|r| mismatch(&r))
        }
        "reservation_ttl_seconds" => {
            let n = value
                .as_i64()
                .ok_or_else(|| mismatch("must be a JSON integer"))?;
            if !(1..=86_400).contains(&n) {
                return Err(mismatch("must be in [1, 86400]"));
            }
            Ok(())
        }
        "decision" => {
            let s = value.as_str().ok_or_else(|| mismatch("must be a string"))?;
            if !DECISIONS.contains(&s) {
                return Err(mismatch(
                    "must be one of STOP, REQUIRE_APPROVAL, DEGRADE, CONTINUE, SKIP",
                ));
            }
            Ok(())
        }
        "approver_role" => {
            let s = value.as_str().ok_or_else(|| mismatch("must be a string"))?;
            if s.is_empty() || s.len() > 64 {
                return Err(mismatch("must be 1..=64 chars"));
            }
            if !s
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(mismatch("must match [A-Za-z0-9_-]+"));
            }
            Ok(())
        }
        // Unreachable: path_leaf_if_allowed only returns known leaves.
        _ => Err(mismatch("unknown leaf")),
    }
}

fn validate_atomic_amount_str(s: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err("must be non-empty".to_string());
    }
    if s.len() > 38 {
        return Err("must be ≤ 38 digits (NUMERIC(38,0) bound)".to_string());
    }
    if !s.chars().all(|c| c.is_ascii_digit()) {
        return Err("must be a digit string".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Standard test UUID for pinning ops.
    const UUID_A: &str = "a1b2c3d4-e5f6-7890-abcd-ef0123456789";

    /// Helper: build a valid pinned budget patch for tests.
    fn pinned_budget_replace(idx: u32, leaf: &str, value: serde_json::Value) -> serde_json::Value {
        json!([
            {"op":"test","path":format!("/spec/budgets/{}/id", idx),"value":UUID_A},
            {"op":"replace","path":format!("/spec/budgets/{}/{}", idx, leaf),"value":value}
        ])
    }

    #[test]
    fn valid_budget_limit_patch_pinned() {
        let p = pinned_budget_replace(0, "limit_amount_atomic", json!("1000000000"));
        assert!(validate(&p).is_ok());
    }

    #[test]
    fn valid_budget_ttl_patch_pinned() {
        let p = pinned_budget_replace(0, "reservation_ttl_seconds", json!(60));
        assert!(validate(&p).is_ok());
    }

    #[test]
    fn valid_rule_decision_patch() {
        // Rule replaces don't require pinning.
        let p = json!([{"op":"replace","path":"/spec/rules/2/then/decision","value":"REQUIRE_APPROVAL"}]);
        assert!(validate(&p).is_ok());
    }

    #[test]
    fn valid_rule_multi_op_patch() {
        let p = json!([
            {"op":"replace","path":"/spec/rules/0/when/claim_amount_atomic_gt","value":"500000000"},
            {"op":"replace","path":"/spec/rules/0/then/approver_role","value":"finance"}
        ]);
        assert!(validate(&p).is_ok());
    }

    #[test]
    fn rejects_add_op() {
        let p = json!([{"op":"add","path":"/spec/budgets/0/limit_amount_atomic","value":"x"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpNotReplace { .. })
        ));
    }

    #[test]
    fn rejects_remove_op() {
        let p = json!([{"op":"remove","path":"/spec/rules/0/then/decision"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpNotReplace { .. })
        ));
    }

    #[test]
    fn rejects_out_of_allowlist_path() {
        let p = json!([{"op":"replace","path":"/metadata/owner_team","value":"x"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpPathNotAllowed { .. })
        ));
    }

    #[test]
    fn rejects_no_spec_prefix_path() {
        // CA-P3.1 r1 P1: old CA-P3 shape without /spec/ prefix MUST
        // be rejected (the contract YAML schema nests under spec).
        let p = json!([{"op":"replace","path":"/budgets/0/limit_amount_atomic","value":"1"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpPathNotAllowed { .. })
        ));
    }

    #[test]
    fn rejects_old_field_name_budget_id() {
        // CA-P3.1 r1 P1: the budget identity field is `id`, not `budget_id`.
        let p = json!([
            {"op":"test","path":"/spec/budgets/0/budget_id","value":UUID_A}
        ]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpPathNotAllowed { .. })
        ));
    }

    #[test]
    fn rejects_non_digit_index() {
        let p = json!([{"op":"replace","path":"/spec/rules/abc/then/decision","value":"STOP"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpPathNotAllowed { .. })
        ));
    }

    #[test]
    fn rejects_negative_index() {
        let p = json!([{"op":"replace","path":"/spec/rules/-1/then/decision","value":"STOP"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpPathNotAllowed { .. })
        ));
    }

    #[test]
    fn rejects_leading_zero_index() {
        let p = json!([{"op":"replace","path":"/spec/rules/01/then/decision","value":"STOP"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpPathNotAllowed { .. })
        ));
    }

    #[test]
    fn accepts_single_zero_index() {
        let p = json!([{"op":"replace","path":"/spec/rules/0/then/decision","value":"STOP"}]);
        assert!(validate(&p).is_ok());
    }

    #[test]
    fn rejects_empty_array() {
        let p = json!([]);
        assert!(matches!(validate(&p), Err(PatchValidationError::Empty)));
    }

    #[test]
    fn rejects_non_array() {
        let p = json!({"op":"replace"});
        assert!(matches!(validate(&p), Err(PatchValidationError::NotArray)));
    }

    #[test]
    fn rejects_too_many_ops() {
        // Use rule-replace ops since they don't require pinning.
        let ops: Vec<Value> = (0..9)
            .map(|i| json!({"op":"replace","path":format!("/spec/rules/{}/then/decision", i),"value":"STOP"}))
            .collect();
        let p = Value::Array(ops);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::TooManyOps { .. })
        ));
    }

    #[test]
    fn rejects_missing_value() {
        let p = json!([
            {"op":"test","path":"/spec/budgets/0/id","value":UUID_A},
            {"op":"replace","path":"/spec/budgets/0/limit_amount_atomic"}
        ]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpMissingValue { .. })
        ));
    }

    // --------- value-schema gate tests ---------

    #[test]
    fn rejects_decision_value_not_in_enum() {
        let p = json!([{"op":"replace","path":"/spec/rules/0/then/decision","value":"wat"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpValueSchema { .. })
        ));
    }

    #[test]
    fn rejects_decision_value_null() {
        let p = json!([{"op":"replace","path":"/spec/rules/0/then/decision","value":null}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpValueSchema { .. })
        ));
    }

    #[test]
    fn rejects_atomic_amount_non_digit() {
        let p = pinned_budget_replace(0, "limit_amount_atomic", json!("1.5"));
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpValueSchema { .. })
        ));
    }

    #[test]
    fn rejects_atomic_amount_too_long() {
        let p = pinned_budget_replace(0, "limit_amount_atomic", json!("1".repeat(39)));
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpValueSchema { .. })
        ));
    }

    #[test]
    fn rejects_ttl_zero() {
        let p = pinned_budget_replace(0, "reservation_ttl_seconds", json!(0));
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpValueSchema { .. })
        ));
    }

    #[test]
    fn rejects_ttl_too_large() {
        let p = pinned_budget_replace(0, "reservation_ttl_seconds", json!(86401));
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpValueSchema { .. })
        ));
    }

    #[test]
    fn rejects_approver_role_with_bad_chars() {
        let p = json!([{"op":"replace","path":"/spec/rules/0/then/approver_role","value":"finance team!"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpValueSchema { .. })
        ));
    }

    #[test]
    fn rejects_approver_role_empty_string() {
        let p = json!([{"op":"replace","path":"/spec/rules/0/then/approver_role","value":""}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpValueSchema { .. })
        ));
    }

    // --------- CA-P3.1 test-op + same-index pinning tests ---------

    #[test]
    fn valid_test_then_replace_patch() {
        let p = json!([
            {"op":"test","path":"/spec/budgets/0/id","value":UUID_A},
            {"op":"replace","path":"/spec/budgets/0/reservation_ttl_seconds","value":60}
        ]);
        assert!(validate(&p).is_ok());
    }

    #[test]
    fn rejects_test_with_non_uuid_value() {
        let p = json!([
            {"op":"test","path":"/spec/budgets/0/id","value":"not-a-uuid"}
        ]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpValueSchema { .. })
        ));
    }

    #[test]
    fn rejects_test_with_uppercase_uuid() {
        let p = json!([
            {"op":"test","path":"/spec/budgets/0/id","value":"A1B2C3D4-E5F6-7890-ABCD-EF0123456789"}
        ]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpValueSchema { .. })
        ));
    }

    #[test]
    fn rejects_test_on_disallowed_path() {
        let p = json!([
            {"op":"test","path":"/metadata/owner","value":UUID_A}
        ]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpPathNotAllowed { .. })
        ));
    }

    #[test]
    fn rejects_test_on_replace_path() {
        // /spec/budgets/<i>/reservation_ttl_seconds is a replace path, not a test path.
        let p = json!([
            {"op":"test","path":"/spec/budgets/0/reservation_ttl_seconds","value":UUID_A}
        ]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpPathNotAllowed { .. })
        ));
    }

    #[test]
    fn rejects_test_with_leading_zero_index() {
        let p = json!([
            {"op":"test","path":"/spec/budgets/01/id","value":UUID_A}
        ]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpPathNotAllowed { .. })
        ));
    }

    #[test]
    fn rejects_test_missing_value() {
        let p = json!([{"op":"test","path":"/spec/budgets/0/id"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpMissingValue { .. })
        ));
    }

    // --------- same-index pinning invariant tests (codex CA-P3.1 r1 P2) ---------

    #[test]
    fn rejects_budget_replace_without_preceding_test() {
        // Plain replace on /spec/budgets/<i>/X without a preceding
        // test op is the positional-mutation hazard. Reject.
        let p = json!([
            {"op":"replace","path":"/spec/budgets/0/reservation_ttl_seconds","value":60}
        ]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::BudgetReplaceMissingTestPin { idx: 0, .. })
        ));
    }

    #[test]
    fn rejects_budget_replace_with_test_at_different_index() {
        let p = json!([
            {"op":"test","path":"/spec/budgets/0/id","value":UUID_A},
            {"op":"replace","path":"/spec/budgets/1/reservation_ttl_seconds","value":60}
        ]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::BudgetReplaceMissingTestPin { idx: 1, .. })
        ));
    }

    #[test]
    fn accepts_two_budgets_each_pinned() {
        // Both budgets get their own preceding test op.
        let p = json!([
            {"op":"test","path":"/spec/budgets/0/id","value":UUID_A},
            {"op":"test","path":"/spec/budgets/1/id","value":"b1c2d3e4-f567-8901-bcde-f01234567890"},
            {"op":"replace","path":"/spec/budgets/0/reservation_ttl_seconds","value":60},
            {"op":"replace","path":"/spec/budgets/1/limit_amount_atomic","value":"100"}
        ]);
        assert!(validate(&p).is_ok());
    }

    #[test]
    fn rejects_replace_then_test_wrong_order() {
        // Replace comes BEFORE the test op — invariant says test
        // must precede.
        let p = json!([
            {"op":"replace","path":"/spec/budgets/0/reservation_ttl_seconds","value":60},
            {"op":"test","path":"/spec/budgets/0/id","value":UUID_A}
        ]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::BudgetReplaceMissingTestPin { idx: 0, .. })
        ));
    }

    #[test]
    fn rejects_path_traversal_attempt() {
        let p = json!([{"op":"replace","path":"/spec/budgets/0/limit_amount_atomic/../metadata","value":"x"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpPathNotAllowed { .. })
        ));
    }

    #[test]
    fn rejects_trailing_slash() {
        let p = json!([{"op":"replace","path":"/spec/budgets/0/limit_amount_atomic/","value":"x"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpPathNotAllowed { .. })
        ));
    }

    #[test]
    fn rejects_empty_index() {
        let p = json!([{"op":"replace","path":"/spec/budgets//limit_amount_atomic","value":"x"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpPathNotAllowed { .. })
        ));
    }
}
