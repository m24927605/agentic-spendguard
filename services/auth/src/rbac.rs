//! Phase 5 GA hardening S18: RBAC + tenant isolation.
//!
//! Five roles + a small permission set, group→roles policy loaded
//! from env (or defaulted for demo profile), and helpers that
//! handlers use to assert "this principal can do this action against
//! this tenant".
//!
//! Builds directly on S17. The `Principal::roles` field that S17
//! left empty is now populated by `GroupPolicy::roles_for_groups`
//! during `Authenticator::authenticate`. Permissions are derived
//! from roles via a static mapping table — keeps the policy compact
//! and unit-testable.
//!
//! Resource-scope assertions:
//!
//!   * `Principal::assert_tenant("tenant-uuid")` — returns
//!     `AuthzError::CrossTenant` if the principal isn't authorized
//!     for that tenant. Handlers MUST call this before any
//!     tenant-scoped DB query (per S18 spec: "every query must
//!     include tenant scope derived from auth context").

use crate::Principal;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

/// Five roles per the spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Viewer,
    Operator,
    Approver,
    Admin,
    Auditor,
}

impl Role {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "viewer" => Some(Self::Viewer),
            "operator" => Some(Self::Operator),
            "approver" => Some(Self::Approver),
            "admin" => Some(Self::Admin),
            "auditor" => Some(Self::Auditor),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Operator => "operator",
            Self::Approver => "approver",
            Self::Admin => "admin",
            Self::Auditor => "auditor",
        }
    }
}

/// Permissions guard concrete actions. Keep this enum small and
/// orthogonal — granular per-resource permissions are out of scope
/// for S18 (S20-S22 expand).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// Read-only views (dashboard / control_plane GETs).
    ReadView,
    /// Mutating tenant operations (create / tombstone tenant).
    TenantWrite,
    /// Resolve approval requests (Approver flow).
    ApprovalResolve,
    /// Trigger / read audit export.
    AuditExport,
    /// Mutate budget (operator / admin).
    BudgetWrite,
}

impl Permission {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ReadView => "read_view",
            Self::TenantWrite => "tenant_write",
            Self::ApprovalResolve => "approval_resolve",
            Self::AuditExport => "audit_export",
            Self::BudgetWrite => "budget_write",
        }
    }
}

/// Static role → permission table. Operators do not get to override
/// this — keeping the matrix in code means every code review that
/// touches it gets visibility on the security boundary.
pub fn permissions_for_role(role: Role) -> &'static [Permission] {
    use Permission::*;
    match role {
        Role::Viewer => &[ReadView],
        Role::Operator => &[ReadView, BudgetWrite],
        Role::Approver => &[ReadView, ApprovalResolve],
        Role::Admin => &[ReadView, TenantWrite, BudgetWrite, ApprovalResolve, AuditExport],
        Role::Auditor => &[ReadView, AuditExport],
    }
}

/// Group → roles policy. Loaded from env at startup as JSON. Each
/// group string can map to multiple roles (typical for "engineering
/// admins" who get viewer + operator + admin).
#[derive(Debug, Clone, Default)]
pub struct GroupPolicy {
    pub mapping: HashMap<String, Vec<Role>>,
    /// When true, an authenticated principal whose groups don't
    /// match any policy entry gets the `Viewer` role by default.
    /// Useful for orgs that gate "is this user a SpendGuard user
    /// at all" via the OIDC issuer rather than via group claims.
    pub default_viewer_on_miss: bool,
}

impl GroupPolicy {
    /// Empty policy. Authenticator will populate Principal.roles =
    /// empty unless configured — handlers will then 403 every
    /// authenticated request that isn't a /healthz / read-no-perm
    /// path. This is intentional fail-closed default.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Demo policy — used only in `SPENDGUARD_PROFILE=demo` so the
    /// existing demo flows keep working without operator config.
    pub fn demo_default() -> Self {
        let mut mapping = HashMap::new();
        // Static-token demo subjects map to admin (full power).
        mapping.insert(
            "demo-admins".to_string(),
            vec![Role::Admin, Role::Operator, Role::Auditor, Role::Approver, Role::Viewer],
        );
        Self {
            mapping,
            default_viewer_on_miss: true,
        }
    }

    /// Parse from a JSON env var.
    /// Format: `{"<group-name>": ["admin", "operator"], ...}` plus
    /// optional `"_default_viewer_on_miss": true`.
    pub fn parse_json(raw: &str) -> Result<Self, AuthzError> {
        if raw.trim().is_empty() {
            return Ok(Self::empty());
        }
        let mut value: serde_json::Value = serde_json::from_str(raw)
            .map_err(|e| AuthzError::PolicyParse(format!("not valid JSON: {e}")))?;

        let default_viewer = value
            .as_object_mut()
            .and_then(|m| m.remove("_default_viewer_on_miss"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let map: HashMap<String, Vec<String>> = serde_json::from_value(value)
            .map_err(|e| AuthzError::PolicyParse(format!("expected map<string, [string]>: {e}")))?;

        let mut mapping = HashMap::with_capacity(map.len());
        for (group, role_strs) in map {
            let roles = role_strs
                .into_iter()
                .map(|s| {
                    Role::parse(&s)
                        .ok_or_else(|| AuthzError::PolicyParse(format!("unknown role: {s:?}")))
                })
                .collect::<Result<Vec<_>, _>>()?;
            mapping.insert(group, roles);
        }
        Ok(Self {
            mapping,
            default_viewer_on_miss: default_viewer,
        })
    }

    /// Resolve an authenticated principal's groups into the union
    /// of mapped roles. Stable order (sorted by Role enum) so audit
    /// logs are deterministic.
    pub fn roles_for_groups(&self, groups: &[String]) -> Vec<String> {
        let mut roles: HashSet<Role> = HashSet::new();
        let mut hit_any = false;
        for g in groups {
            if let Some(rs) = self.mapping.get(g) {
                hit_any = true;
                roles.extend(rs.iter().copied());
            }
        }
        if !hit_any && self.default_viewer_on_miss {
            roles.insert(Role::Viewer);
        }
        let mut sorted: Vec<Role> = roles.into_iter().collect();
        sorted.sort_by_key(|r| r.as_str());
        sorted.into_iter().map(|r| r.as_str().to_string()).collect()
    }
}

#[derive(Debug, Error)]
pub enum AuthzError {
    #[error("invalid policy: {0}")]
    PolicyParse(String),
    #[error("forbidden: principal lacks {needed:?}")]
    MissingPermission { needed: Permission },
    #[error("forbidden: cross-tenant access {requested}")]
    CrossTenant { requested: String },
    #[error("forbidden: principal has no tenant scope")]
    NoTenantScope,
}

impl Principal {
    /// True if this principal carries the role.
    pub fn has_role(&self, role: Role) -> bool {
        self.roles.iter().any(|r| r == role.as_str())
    }

    /// True if this principal carries any role that grants the
    /// permission. Static-token mode without role mapping returns
    /// false for every permission — the demo's policy
    /// (`demo-admins`) is what makes the demo work.
    pub fn has_permission(&self, p: Permission) -> bool {
        for role_str in &self.roles {
            if let Some(role) = Role::parse(role_str) {
                if permissions_for_role(role).iter().any(|q| *q == p) {
                    return true;
                }
            }
        }
        false
    }

    /// Assert principal has permission OR return AuthzError.
    pub fn require(&self, p: Permission) -> Result<(), AuthzError> {
        if self.has_permission(p) {
            Ok(())
        } else {
            Err(AuthzError::MissingPermission { needed: p })
        }
    }

    /// Assert principal is authorized for this tenant. If
    /// `tenant_ids` is empty (typical for static-token demo
    /// principals), returns NoTenantScope so handlers fail-closed
    /// — operators must configure the tenant_ids claim explicitly.
    pub fn assert_tenant(&self, tenant_id: &str) -> Result<(), AuthzError> {
        if self.tenant_ids.is_empty() {
            return Err(AuthzError::NoTenantScope);
        }
        if self.tenant_ids.iter().any(|t| t == tenant_id) {
            Ok(())
        } else {
            Err(AuthzError::CrossTenant {
                requested: tenant_id.to_string(),
            })
        }
    }

    /// Override the principal's tenant scope — used at startup for
    /// the static-token demo principal so the demo flow keeps
    /// working. Mutating method (not exposed in S18 production
    /// path; static_token bootstrap calls it).
    pub fn override_tenant_scope(&mut self, tenant_ids: Vec<String>) {
        self.tenant_ids = tenant_ids;
    }

    /// Override the principal's roles. Used by Authenticator after
    /// authentication to apply the GroupPolicy. Internal API.
    pub(crate) fn set_roles(&mut self, roles: Vec<String>) {
        self.roles = roles;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn principal_with(roles: Vec<&str>, tenants: Vec<&str>) -> Principal {
        Principal {
            issuer: "test".into(),
            subject: "user@example.com".into(),
            groups: vec![],
            tenant_ids: tenants.into_iter().map(String::from).collect(),
            roles: roles.into_iter().map(String::from).collect(),
            mode: "jwt".into(),
        }
    }

    #[test]
    fn role_parse_known_values() {
        for s in ["viewer", "operator", "approver", "admin", "auditor"] {
            assert!(Role::parse(s).is_some());
        }
        assert!(Role::parse("nonsense").is_none());
        assert!(Role::parse("").is_none());
    }

    #[test]
    fn permissions_for_admin_include_all_others_minus_none() {
        let admin = permissions_for_role(Role::Admin);
        for p in [
            Permission::ReadView,
            Permission::TenantWrite,
            Permission::BudgetWrite,
            Permission::ApprovalResolve,
            Permission::AuditExport,
        ] {
            assert!(admin.contains(&p), "admin should have {p:?}");
        }
    }

    #[test]
    fn viewer_can_read_but_not_approve_or_mutate() {
        let p = principal_with(vec!["viewer"], vec!["t1"]);
        assert!(p.has_permission(Permission::ReadView));
        assert!(!p.has_permission(Permission::ApprovalResolve));
        assert!(!p.has_permission(Permission::TenantWrite));
        assert!(!p.has_permission(Permission::BudgetWrite));
    }

    #[test]
    fn approver_can_resolve_but_not_create_tenant() {
        let p = principal_with(vec!["approver"], vec!["t1"]);
        assert!(p.has_permission(Permission::ApprovalResolve));
        assert!(!p.has_permission(Permission::TenantWrite));
    }

    #[test]
    fn auditor_can_export_but_not_mutate_budgets() {
        let p = principal_with(vec!["auditor"], vec!["t1"]);
        assert!(p.has_permission(Permission::AuditExport));
        assert!(p.has_permission(Permission::ReadView));
        assert!(!p.has_permission(Permission::BudgetWrite));
        assert!(!p.has_permission(Permission::TenantWrite));
        assert!(!p.has_permission(Permission::ApprovalResolve));
    }

    #[test]
    fn require_permission_returns_typed_error_when_missing() {
        let p = principal_with(vec!["viewer"], vec!["t1"]);
        let err = p.require(Permission::TenantWrite).unwrap_err();
        assert!(matches!(
            err,
            AuthzError::MissingPermission {
                needed: Permission::TenantWrite
            }
        ));
    }

    #[test]
    fn assert_tenant_passes_when_in_scope() {
        let p = principal_with(vec!["admin"], vec!["t1", "t2"]);
        p.assert_tenant("t1").unwrap();
        p.assert_tenant("t2").unwrap();
    }

    #[test]
    fn assert_tenant_rejects_cross_tenant() {
        let p = principal_with(vec!["admin"], vec!["t1"]);
        let err = p.assert_tenant("t2").unwrap_err();
        match err {
            AuthzError::CrossTenant { requested } => assert_eq!(requested, "t2"),
            other => panic!("expected CrossTenant, got {other:?}"),
        }
    }

    #[test]
    fn assert_tenant_rejects_principal_with_no_scope() {
        let p = principal_with(vec!["admin"], vec![]);
        let err = p.assert_tenant("t1").unwrap_err();
        assert!(matches!(err, AuthzError::NoTenantScope));
    }

    #[test]
    fn group_policy_parse_round_trips_known_roles() {
        let raw = r#"{"sg-admins":["admin","operator"],"sg-readers":["viewer"]}"#;
        let p = GroupPolicy::parse_json(raw).unwrap();
        assert_eq!(p.mapping.len(), 2);
        assert!(p.mapping.contains_key("sg-admins"));
    }

    #[test]
    fn group_policy_rejects_unknown_role() {
        let raw = r#"{"sg-admins":["wizard"]}"#;
        let err = GroupPolicy::parse_json(raw).unwrap_err();
        assert!(matches!(err, AuthzError::PolicyParse(_)));
    }

    #[test]
    fn group_policy_rejects_malformed_json() {
        let err = GroupPolicy::parse_json("not-json").unwrap_err();
        assert!(matches!(err, AuthzError::PolicyParse(_)));
    }

    #[test]
    fn group_policy_resolves_groups_to_role_union() {
        let raw = r#"{"a":["viewer"],"b":["operator"],"c":["admin"]}"#;
        let p = GroupPolicy::parse_json(raw).unwrap();
        let roles =
            p.roles_for_groups(&["a".into(), "b".into(), "missing".into()]);
        // Order should be sorted by role name.
        assert_eq!(roles, vec!["operator".to_string(), "viewer".to_string()]);
    }

    #[test]
    fn group_policy_default_viewer_on_miss_when_configured() {
        let raw = r#"{"a":["admin"],"_default_viewer_on_miss":true}"#;
        let p = GroupPolicy::parse_json(raw).unwrap();
        let roles = p.roles_for_groups(&["unknown".into()]);
        assert_eq!(roles, vec!["viewer".to_string()]);
    }

    #[test]
    fn group_policy_no_default_viewer_when_not_configured() {
        let raw = r#"{"a":["admin"]}"#;
        let p = GroupPolicy::parse_json(raw).unwrap();
        let roles = p.roles_for_groups(&["unknown".into()]);
        assert!(roles.is_empty());
    }

    #[test]
    fn demo_default_policy_grants_admin_to_demo_admins_group() {
        let p = GroupPolicy::demo_default();
        let roles = p.roles_for_groups(&["demo-admins".into()]);
        assert!(roles.contains(&"admin".to_string()));
    }

    #[test]
    fn demo_default_policy_falls_through_to_viewer_for_unmapped_groups() {
        let p = GroupPolicy::demo_default();
        let roles = p.roles_for_groups(&["random-group".into()]);
        assert_eq!(roles, vec!["viewer".to_string()]);
    }

    #[test]
    fn empty_policy_grants_no_roles_so_handlers_fail_closed() {
        let p = GroupPolicy::empty();
        let roles = p.roles_for_groups(&["any-group".into()]);
        assert!(roles.is_empty());
    }
}
