//! Shared parser for `approval_requests.approver_policy` JSONB.
//!
//! Lifted out of `services/control_plane/src/main.rs` so the same fail-closed
//! Malformed-vs-NoRestriction-vs-Restrict semantics are enforced by every
//! HTTP surface that exposes approval resolution — both control_plane's
//! `POST /v1/approvals/:id/resolve` and dashboard's
//! `POST /api/approvals/:id/resolve`.
//!
//! The schema only enforces `JSONB NOT NULL DEFAULT '{}'`, so the parser
//! is the authoritative security boundary. A policy that *looks
//! restrictive* but is malformed (wrong types, empty array, empty
//! string) is treated as fail-closed: the operator clearly *intended*
//! to restrict but the data is unusable, so silently widening access
//! would be unsafe.
//!
//! Accepted restrictive keys:
//!   * `roles` / `required_roles`   — array of role-name strings
//!   * `role`  / `approver_role`    — single role-name string OR array
//!                                     of role-name strings
//!
//! `approver_role` matches the canonical contract.yaml /
//! `ApprovalDecision.approver_role` field name (Codex round-2 P1).

use serde_json::Value;

/// Three-way result for [`parse_approver_policy`]. The third arm is what
/// makes the gate a real security boundary — see module docs.
#[derive(Debug, PartialEq, Eq)]
pub enum ApproverPolicyParse {
    /// Empty `{}`, JSON null, or an object that carries only
    /// non-restrictive metadata (e.g. `{"description": "..."}`).
    /// Permission gate is the only check.
    NoRestriction,
    /// Restrictive policy with at least one valid role name. Caller
    /// intersects against `principal.roles`.
    Restrict(Vec<String>),
    /// One or more restrictive keys are present but the value is
    /// malformed (non-array where array expected, wrong element type,
    /// empty list, empty string). Treat as fail-closed.
    Malformed,
}

/// Parse `approval_requests.approver_policy` JSONB into a typed
/// outcome. See module docs for the recognized key set + semantics.
pub fn parse_approver_policy(policy: &Value) -> ApproverPolicyParse {
    if policy.is_null() {
        return ApproverPolicyParse::NoRestriction;
    }
    let Some(obj) = policy.as_object() else {
        // Non-object, non-null shape (array, scalar) — operator likely
        // intended *something*; fail closed.
        return ApproverPolicyParse::Malformed;
    };
    if obj.is_empty() {
        return ApproverPolicyParse::NoRestriction;
    }

    const ARRAY_KEYS: &[&str] = &["roles", "required_roles"];
    const STRING_OR_ARRAY_KEYS: &[&str] = &["role", "approver_role"];

    let any_restrictive = ARRAY_KEYS
        .iter()
        .chain(STRING_OR_ARRAY_KEYS.iter())
        .any(|k| obj.contains_key(*k));
    if !any_restrictive {
        // Object has only metadata-style keys. No restriction.
        return ApproverPolicyParse::NoRestriction;
    }

    let mut roles: Vec<String> = Vec::new();

    for key in ARRAY_KEYS {
        let Some(v) = obj.get(*key) else { continue };
        let Some(arr) = v.as_array() else {
            return ApproverPolicyParse::Malformed;
        };
        if arr.is_empty() {
            return ApproverPolicyParse::Malformed;
        }
        for item in arr {
            match item.as_str() {
                Some(s) if !s.is_empty() => roles.push(s.to_string()),
                _ => return ApproverPolicyParse::Malformed,
            }
        }
    }

    for key in STRING_OR_ARRAY_KEYS {
        let Some(v) = obj.get(*key) else { continue };
        if let Some(s) = v.as_str() {
            if s.is_empty() {
                return ApproverPolicyParse::Malformed;
            }
            roles.push(s.to_string());
        } else if let Some(arr) = v.as_array() {
            if arr.is_empty() {
                return ApproverPolicyParse::Malformed;
            }
            for item in arr {
                match item.as_str() {
                    Some(s) if !s.is_empty() => roles.push(s.to_string()),
                    _ => return ApproverPolicyParse::Malformed,
                }
            }
        } else {
            return ApproverPolicyParse::Malformed;
        }
    }

    if roles.is_empty() {
        // Restrictive keys present but we somehow extracted no roles.
        // Defensive: fail closed.
        return ApproverPolicyParse::Malformed;
    }
    ApproverPolicyParse::Restrict(roles)
}

/// Render a redacted shape descriptor for an `approver_policy`. The
/// malformed-fail-closed log path uses this instead of dumping the full
/// JSONB, because the policy can carry operator-supplied metadata
/// (e.g. `description`, contract context) that may include sensitive
/// strings. Operators only need the top-level type + key list to debug
/// "why was this rejected" (Codex round-3 P2).
pub fn approver_policy_shape(policy: &Value) -> String {
    match policy {
        Value::Null => "null".to_string(),
        Value::Bool(_) => "bool".to_string(),
        Value::Number(_) => "number".to_string(),
        Value::String(_) => "string".to_string(),
        Value::Array(a) => format!("array(len={})", a.len()),
        Value::Object(m) => {
            let mut keys: Vec<&str> = m.keys().map(|s| s.as_str()).collect();
            keys.sort();
            format!("object(keys=[{}])", keys.join(","))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn restrict(items: &[&str]) -> ApproverPolicyParse {
        ApproverPolicyParse::Restrict(items.iter().map(|s| s.to_string()).collect())
    }

    // ---- NoRestriction ----------------------------------------------

    #[test]
    fn empty_object_no_restriction() {
        assert_eq!(
            parse_approver_policy(&json!({})),
            ApproverPolicyParse::NoRestriction
        );
    }

    #[test]
    fn json_null_no_restriction() {
        assert_eq!(
            parse_approver_policy(&Value::Null),
            ApproverPolicyParse::NoRestriction
        );
    }

    #[test]
    fn metadata_only_object_no_restriction() {
        let p = json!({"description": "billing-team approval flow"});
        assert_eq!(
            parse_approver_policy(&p),
            ApproverPolicyParse::NoRestriction
        );
    }

    // ---- Restrict ----------------------------------------------------

    #[test]
    fn roles_array_extracts_each_role() {
        let p = json!({"roles": ["finance", "audit"]});
        assert_eq!(parse_approver_policy(&p), restrict(&["finance", "audit"]));
    }

    #[test]
    fn required_roles_array_extracts_each_role() {
        let p = json!({"required_roles": ["finance"]});
        assert_eq!(parse_approver_policy(&p), restrict(&["finance"]));
    }

    #[test]
    fn approver_role_string_extracts_single_role() {
        let p = json!({"approver_role": "finance"});
        assert_eq!(parse_approver_policy(&p), restrict(&["finance"]));
    }

    #[test]
    fn approver_role_array_extracts_each_role() {
        let p = json!({"approver_role": ["finance", "audit"]});
        assert_eq!(parse_approver_policy(&p), restrict(&["finance", "audit"]));
    }

    // ---- Malformed (fail-closed) -------------------------------------

    #[test]
    fn array_top_level_malformed() {
        assert_eq!(
            parse_approver_policy(&json!(["finance"])),
            ApproverPolicyParse::Malformed
        );
    }

    #[test]
    fn scalar_top_level_malformed() {
        assert_eq!(
            parse_approver_policy(&json!("finance")),
            ApproverPolicyParse::Malformed
        );
    }

    #[test]
    fn empty_roles_array_malformed() {
        assert_eq!(
            parse_approver_policy(&json!({"roles": []})),
            ApproverPolicyParse::Malformed
        );
    }

    #[test]
    fn roles_array_with_non_string_malformed() {
        assert_eq!(
            parse_approver_policy(&json!({"roles": ["finance", 42]})),
            ApproverPolicyParse::Malformed
        );
    }

    #[test]
    fn empty_string_role_malformed() {
        assert_eq!(
            parse_approver_policy(&json!({"approver_role": ""})),
            ApproverPolicyParse::Malformed
        );
    }

    #[test]
    fn roles_non_array_value_malformed() {
        assert_eq!(
            parse_approver_policy(&json!({"roles": "finance"})),
            ApproverPolicyParse::Malformed
        );
    }

    // ---- approver_policy_shape redaction -----------------------------

    #[test]
    fn shape_redacts_object_keys_only() {
        let p = json!({"description": "secret reason", "roles": ["finance"]});
        let shape = approver_policy_shape(&p);
        assert!(shape.starts_with("object(keys=["), "got: {shape}");
        assert!(shape.contains("description"));
        assert!(shape.contains("roles"));
        // No content leakage.
        assert!(!shape.contains("secret reason"));
        assert!(!shape.contains("finance"));
    }

    #[test]
    fn shape_distinguishes_array_from_object() {
        assert_eq!(approver_policy_shape(&json!(["a", "b"])), "array(len=2)");
        assert_eq!(approver_policy_shape(&json!({})), "object(keys=[])");
        assert_eq!(approver_policy_shape(&Value::Null), "null");
    }
}
