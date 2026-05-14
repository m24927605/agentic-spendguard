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
//! Allowed RFC-6902 ops: `replace` only.
//! Allowed JSON Pointer paths (each `<i>` is an RFC 6901 array index
//! — `0` or `[1-9][0-9]*`, no leading zeros):
//!   1. `/budgets/<i>/limit_amount_atomic`
//!   2. `/budgets/<i>/reservation_ttl_seconds`
//!   3. `/rules/<i>/when/claim_amount_atomic_gt`
//!   4. `/rules/<i>/then/decision`
//!   5. `/rules/<i>/then/approver_role`
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
    #[error("op #{index}: `op` must be `replace`, got `{op}`")]
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
    for (i, op) in arr.iter().enumerate() {
        validate_op(i, op)?;
    }
    Ok(())
}

fn validate_op(index: usize, op: &Value) -> Result<(), PatchValidationError> {
    let obj = op
        .as_object()
        .ok_or(PatchValidationError::OpNotObject { index })?;

    let op_kind = obj
        .get("op")
        .and_then(Value::as_str)
        .ok_or(PatchValidationError::OpMissingOp { index })?;
    if op_kind != "replace" {
        return Err(PatchValidationError::OpNotReplace {
            index,
            op: op_kind.to_string(),
        });
    }

    let path = obj
        .get("path")
        .and_then(Value::as_str)
        .ok_or(PatchValidationError::OpMissingPath { index })?;
    let Some(leaf) = path_leaf_if_allowed(path) else {
        return Err(PatchValidationError::OpPathNotAllowed {
            index,
            path: path.to_string(),
        });
    };

    let value = obj
        .get("value")
        .ok_or(PatchValidationError::OpMissingValue { index })?;

    validate_value(index, leaf, value)?;

    Ok(())
}

/// If `path` matches one of the 5 allowlisted JSON Pointer patterns,
/// returns the leaf segment so the value-schema gate can dispatch.
/// Otherwise returns None.
///
/// Implemented by structural parse rather than regex so a future
/// relaxation is a code addition with a clear branch.
fn path_leaf_if_allowed(path: &str) -> Option<&'static str> {
    let rest = path.strip_prefix('/')?;
    let segments: Vec<&str> = rest.split('/').collect();

    match segments.as_slice() {
        ["budgets", idx, "limit_amount_atomic"] if is_array_index(idx) => {
            Some("limit_amount_atomic")
        }
        ["budgets", idx, "reservation_ttl_seconds"] if is_array_index(idx) => {
            Some("reservation_ttl_seconds")
        }
        ["rules", idx, "when", "claim_amount_atomic_gt"] if is_array_index(idx) => {
            Some("claim_amount_atomic_gt")
        }
        ["rules", idx, "then", "decision"] if is_array_index(idx) => Some("decision"),
        ["rules", idx, "then", "approver_role"] if is_array_index(idx) => Some("approver_role"),
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

    #[test]
    fn valid_budget_limit_patch() {
        let p = json!([{"op":"replace","path":"/budgets/0/limit_amount_atomic","value":"1000000000"}]);
        assert!(validate(&p).is_ok());
    }

    #[test]
    fn valid_budget_ttl_patch() {
        let p = json!([{"op":"replace","path":"/budgets/0/reservation_ttl_seconds","value":60}]);
        assert!(validate(&p).is_ok());
    }

    #[test]
    fn valid_rule_decision_patch() {
        let p = json!([{"op":"replace","path":"/rules/2/then/decision","value":"REQUIRE_APPROVAL"}]);
        assert!(validate(&p).is_ok());
    }

    #[test]
    fn valid_multi_op_patch() {
        let p = json!([
            {"op":"replace","path":"/rules/0/when/claim_amount_atomic_gt","value":"500000000"},
            {"op":"replace","path":"/rules/0/then/approver_role","value":"finance"}
        ]);
        assert!(validate(&p).is_ok());
    }

    #[test]
    fn rejects_add_op() {
        let p = json!([{"op":"add","path":"/budgets/0/limit_amount_atomic","value":"x"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpNotReplace { .. })
        ));
    }

    #[test]
    fn rejects_remove_op() {
        let p = json!([{"op":"remove","path":"/rules/0/then/decision"}]);
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
    fn rejects_nested_old_style_path() {
        // The original allowlist used /when/claim/amount_atomic_gt
        // (nested); real DSL uses flat claim_amount_atomic_gt. The
        // old shape MUST be rejected (codex CA-P3 r1 P1).
        let p = json!([{"op":"replace","path":"/rules/0/when/claim/amount_atomic_gt","value":"1"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpPathNotAllowed { .. })
        ));
    }

    #[test]
    fn rejects_non_digit_index() {
        let p = json!([{"op":"replace","path":"/rules/abc/then/decision","value":"STOP"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpPathNotAllowed { .. })
        ));
    }

    #[test]
    fn rejects_negative_index() {
        let p = json!([{"op":"replace","path":"/rules/-1/then/decision","value":"STOP"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpPathNotAllowed { .. })
        ));
    }

    #[test]
    fn rejects_leading_zero_index() {
        // RFC 6901 §4: array indices have no leading zeros (codex
        // CA-P3 r1 P2 tightening).
        let p = json!([{"op":"replace","path":"/rules/01/then/decision","value":"STOP"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpPathNotAllowed { .. })
        ));
    }

    #[test]
    fn accepts_single_zero_index() {
        // `0` alone IS a valid RFC 6901 array index.
        let p = json!([{"op":"replace","path":"/rules/0/then/decision","value":"STOP"}]);
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
        let ops: Vec<Value> = (0..9)
            .map(|i| json!({"op":"replace","path":format!("/budgets/{}/limit_amount_atomic", i),"value":"1"}))
            .collect();
        let p = Value::Array(ops);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::TooManyOps { .. })
        ));
    }

    #[test]
    fn rejects_missing_value() {
        let p = json!([{"op":"replace","path":"/budgets/0/limit_amount_atomic"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpMissingValue { .. })
        ));
    }

    // --------- value-schema gate tests (codex CA-P3 r1 P2) ---------

    #[test]
    fn rejects_decision_value_not_in_enum() {
        let p = json!([{"op":"replace","path":"/rules/0/then/decision","value":"wat"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpValueSchema { .. })
        ));
    }

    #[test]
    fn rejects_decision_value_null() {
        let p = json!([{"op":"replace","path":"/rules/0/then/decision","value":null}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpValueSchema { .. })
        ));
    }

    #[test]
    fn rejects_atomic_amount_non_digit() {
        let p = json!([{"op":"replace","path":"/budgets/0/limit_amount_atomic","value":"1.5"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpValueSchema { .. })
        ));
    }

    #[test]
    fn rejects_atomic_amount_too_long() {
        let p = json!([{"op":"replace","path":"/budgets/0/limit_amount_atomic","value":"1".repeat(39)}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpValueSchema { .. })
        ));
    }

    #[test]
    fn rejects_ttl_zero() {
        let p = json!([{"op":"replace","path":"/budgets/0/reservation_ttl_seconds","value":0}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpValueSchema { .. })
        ));
    }

    #[test]
    fn rejects_ttl_too_large() {
        let p = json!([{"op":"replace","path":"/budgets/0/reservation_ttl_seconds","value":86401}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpValueSchema { .. })
        ));
    }

    #[test]
    fn rejects_approver_role_with_bad_chars() {
        let p = json!([{"op":"replace","path":"/rules/0/then/approver_role","value":"finance team!"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpValueSchema { .. })
        ));
    }

    #[test]
    fn rejects_approver_role_empty_string() {
        let p = json!([{"op":"replace","path":"/rules/0/then/approver_role","value":""}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpValueSchema { .. })
        ));
    }

    #[test]
    fn rejects_path_traversal_attempt() {
        let p = json!([{"op":"replace","path":"/budgets/0/limit_amount_atomic/../metadata","value":"x"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpPathNotAllowed { .. })
        ));
    }

    #[test]
    fn rejects_trailing_slash() {
        let p = json!([{"op":"replace","path":"/budgets/0/limit_amount_atomic/","value":"x"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpPathNotAllowed { .. })
        ));
    }

    #[test]
    fn rejects_empty_index() {
        let p = json!([{"op":"replace","path":"/budgets//limit_amount_atomic","value":"x"}]);
        assert!(matches!(
            validate(&p),
            Err(PatchValidationError::OpPathNotAllowed { .. })
        ));
    }
}
