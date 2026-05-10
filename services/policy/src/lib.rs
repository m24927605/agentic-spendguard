//! Phase 5 GA hardening S22: fail-open / fail-closed policy matrix.
//!
//! Defines what every component does when a dependency is unhealthy.
//! Default stance is **fail-closed for monetary workflows** — a budget
//! debit cannot land without ledger evidence. Fail-open is allowed
//! only with:
//!
//!   1. **Explicit per-(dependency, workflow_class) opt-in** — operators
//!      configure `FailPolicyMatrix` via env JSON; the default matrix
//!      blocks on every dependency × workflow_class combination.
//!   2. **An audit marker emitted on every admit** — when sidecar
//!      admits a decision under fail-open, it records a typed
//!      `AuditMarker` row so reconciliation has unambiguous evidence
//!      that the decision didn't go through normal verification.
//!
//! Spec acceptance criteria (review standard):
//!
//!   * "Verify fail-open requires explicit tenant/workflow config and
//!      is visible in audit logs."
//!   * "Verify no fail-open path can debit budget without later
//!      reconciliation evidence."
//!   * "Verify operator docs state blast radius and rollback behavior."
//!
//! Out of scope for S22 (later slices wire the actual hot-path checks):
//!
//!   * Real-time dependency-health probing in the sidecar's decision
//!     loop (S23 metrics layer drives this).
//!   * Hot-reload of the matrix without process restart (S22 reads at
//!     boot; S22-followup adds /admin/reload endpoint).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;
use tracing::{info, warn};
use uuid::Uuid;

// ============================================================================
// Public types
// ============================================================================

/// Workflow classification — drives the default fail-closed-vs-open
/// stance. The classification flows from the contract bundle that
/// defines a given decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowClass {
    /// LLM call with token-cost or USD impact. Default fail-closed
    /// EVERYWHERE — a debit cannot happen without ledger evidence.
    Monetary,
    /// Non-monetary tool calls (e.g. RAG search, embedding lookup).
    /// Operators may opt these into fail-open per dependency since
    /// budget integrity isn't at risk.
    NonMonetaryTool,
    /// Observability-route audits — sidecar emits but adapter
    /// doesn't gate on. Default fail-open since these are not
    /// admission decisions.
    ObservabilityOnly,
}

impl WorkflowClass {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "monetary" => Some(Self::Monetary),
            "non_monetary_tool" => Some(Self::NonMonetaryTool),
            "observability_only" => Some(Self::ObservabilityOnly),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Monetary => "monetary",
            Self::NonMonetaryTool => "non_monetary_tool",
            Self::ObservabilityOnly => "observability_only",
        }
    }
}

/// Components whose unhealth the matrix governs. Mirrors the spec's
/// list: "sidecar, ledger, canonical ingest, pricing authority,
/// signing, provider reconciliation, approval, dashboard, and export."
/// The sidecar is omitted (it's the decider, not a dependency); the
/// service-mesh dependencies are what the matrix actually addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dependency {
    Ledger,
    CanonicalIngest,
    Pricing,
    Signing,
    ProviderReconciliation,
    Approval,
    Dashboard,
    Export,
}

impl Dependency {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ledger" => Some(Self::Ledger),
            "canonical_ingest" => Some(Self::CanonicalIngest),
            "pricing" => Some(Self::Pricing),
            "signing" => Some(Self::Signing),
            "provider_reconciliation" => Some(Self::ProviderReconciliation),
            "approval" => Some(Self::Approval),
            "dashboard" => Some(Self::Dashboard),
            "export" => Some(Self::Export),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ledger => "ledger",
            Self::CanonicalIngest => "canonical_ingest",
            Self::Pricing => "pricing",
            Self::Signing => "signing",
            Self::ProviderReconciliation => "provider_reconciliation",
            Self::Approval => "approval",
            Self::Dashboard => "dashboard",
            Self::Export => "export",
        }
    }
}

/// Policy stance for one (dependency, workflow_class) pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailPolicy {
    /// Block the decision with a typed error. Default for monetary
    /// workflows under every dependency.
    FailClosed,
    /// Admit the decision but emit an explicit audit marker so
    /// reconciliation can identify the row downstream.
    FailOpenWithMarker,
}

/// Decision the sidecar acts on — opaque "what to do next".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum FailMode {
    /// Block the decision. The error message is operator-readable
    /// but doesn't reveal anything sensitive.
    Block { reason: String },
    /// Admit the decision but the sidecar MUST emit the marker.
    Admit { marker: AuditMarker },
}

/// Typed shape of the row sidecar writes when admitting under
/// fail-open. The CloudEvent envelope wraps this as the `data` field
/// with `type = "spendguard.audit.fail_policy_admit"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditMarker {
    pub marker_id: Uuid,
    pub decision_id: String,
    pub tenant_id: String,
    pub dependency: Dependency,
    pub workflow_class: WorkflowClass,
    pub reason: String,
    pub policy_version: String,
    pub admitted_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum PolicyError {
    #[error("invalid policy JSON: {0}")]
    ParseError(String),
    #[error("production profile rejected fail-open without explicit ack")]
    FailOpenWithoutAck,
}

// ============================================================================
// Matrix
// ============================================================================

/// One row of operator-configured policy. Stored in the matrix keyed
/// by (Dependency, WorkflowClass).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatrixEntry {
    pub policy: FailPolicy,
}

/// The complete matrix. Two ways to construct:
///
///   * `default_fail_closed()` — every cell is FailClosed. Used as
///     the safety baseline.
///   * `from_json(raw)` — operator overrides selected cells. JSON
///     shape: `{"<dep>": {"<workflow_class>": "fail_open_with_marker"}}`.
#[derive(Debug, Clone, Serialize)]
pub struct FailPolicyMatrix {
    /// Frozen at boot. Hot-reload deferred to S22-followup.
    cells: HashMap<(Dependency, WorkflowClass), FailPolicy>,
    /// Stable id for this matrix instance — embedded in every audit
    /// marker so an investigator can reproduce the policy that
    /// admitted a decision.
    pub policy_version: String,
}

impl FailPolicyMatrix {
    /// Safety baseline: every cell is FailClosed. Production with no
    /// `FAIL_POLICY_JSON` configured uses this — and that's correct.
    pub fn default_fail_closed() -> Self {
        let mut cells = HashMap::new();
        for dep in [
            Dependency::Ledger,
            Dependency::CanonicalIngest,
            Dependency::Pricing,
            Dependency::Signing,
            Dependency::ProviderReconciliation,
            Dependency::Approval,
            Dependency::Dashboard,
            Dependency::Export,
        ] {
            for wf in [
                WorkflowClass::Monetary,
                WorkflowClass::NonMonetaryTool,
                WorkflowClass::ObservabilityOnly,
            ] {
                cells.insert((dep, wf), FailPolicy::FailClosed);
            }
        }
        Self {
            cells,
            policy_version: "default-fail-closed".into(),
        }
    }

    /// Demo / test matrix: ObservabilityOnly is fail-open across the
    /// board (since by definition not gating). Monetary always fail-
    /// closed; NonMonetaryTool fail-closed by default.
    pub fn observability_open() -> Self {
        let mut m = Self::default_fail_closed();
        for dep in [
            Dependency::Ledger,
            Dependency::CanonicalIngest,
            Dependency::Pricing,
            Dependency::Signing,
            Dependency::ProviderReconciliation,
            Dependency::Approval,
            Dependency::Dashboard,
            Dependency::Export,
        ] {
            m.cells.insert(
                (dep, WorkflowClass::ObservabilityOnly),
                FailPolicy::FailOpenWithMarker,
            );
        }
        m.policy_version = "observability-open-v1".into();
        m
    }

    /// Parse operator overrides on top of the fail-closed baseline.
    /// JSON: `{"<dep>": {"<workflow_class>": "<policy>"}, ...}` plus
    /// optional top-level `"_version": "v1.2.3"`.
    pub fn from_json(raw: &str, profile: &str) -> Result<Self, PolicyError> {
        let mut value: serde_json::Value = serde_json::from_str(raw)
            .map_err(|e| PolicyError::ParseError(format!("not valid JSON: {e}")))?;

        let policy_version = value
            .as_object_mut()
            .and_then(|m| m.remove("_version"))
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "operator-supplied-unversioned".into());

        let _explicit_ack = value
            .as_object_mut()
            .and_then(|m| m.remove("_acknowledge_risk_of_fail_open"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let dep_map: HashMap<String, HashMap<String, String>> = serde_json::from_value(value)
            .map_err(|e| PolicyError::ParseError(format!("expected map<dep, map<wf, policy>>: {e}")))?;

        let mut m = Self::default_fail_closed();
        m.policy_version = policy_version;

        let mut any_fail_open = false;
        for (dep_str, wf_map) in dep_map {
            let dep = Dependency::parse(&dep_str)
                .ok_or_else(|| PolicyError::ParseError(format!("unknown dep: {dep_str}")))?;
            for (wf_str, policy_str) in wf_map {
                let wf = WorkflowClass::parse(&wf_str).ok_or_else(|| {
                    PolicyError::ParseError(format!("unknown workflow_class: {wf_str}"))
                })?;
                let policy: FailPolicy = serde_json::from_value(serde_json::Value::String(
                    policy_str.clone(),
                ))
                .map_err(|e| PolicyError::ParseError(format!("policy {policy_str}: {e}")))?;

                if matches!(policy, FailPolicy::FailOpenWithMarker) {
                    if matches!(wf, WorkflowClass::Monetary) {
                        return Err(PolicyError::ParseError(format!(
                            "fail_open_with_marker is forbidden for monetary workflows ({})",
                            dep_str
                        )));
                    }
                    any_fail_open = true;
                }
                m.cells.insert((dep, wf), policy);
            }
        }

        if any_fail_open && profile == "production" && !_explicit_ack {
            return Err(PolicyError::FailOpenWithoutAck);
        }

        Ok(m)
    }

    /// Look up the configured stance.
    pub fn lookup(&self, dep: Dependency, wf: WorkflowClass) -> FailPolicy {
        self.cells
            .get(&(dep, wf))
            .copied()
            .unwrap_or(FailPolicy::FailClosed)
    }

    /// Decide what the sidecar (or any caller) should do given a
    /// known dependency unhealth.
    pub fn decide(
        &self,
        dep: Dependency,
        wf: WorkflowClass,
        reason: impl Into<String>,
        decision_id: impl Into<String>,
        tenant_id: impl Into<String>,
    ) -> FailMode {
        let policy = self.lookup(dep, wf);
        let reason = reason.into();
        match policy {
            FailPolicy::FailClosed => {
                info!(
                    dep = dep.as_str(),
                    workflow = wf.as_str(),
                    policy_version = %self.policy_version,
                    reason = %reason,
                    "fail-policy: BLOCK"
                );
                FailMode::Block {
                    reason: format!("dependency {} unhealthy: {}", dep.as_str(), reason),
                }
            }
            FailPolicy::FailOpenWithMarker => {
                let marker = AuditMarker {
                    marker_id: Uuid::now_v7(),
                    decision_id: decision_id.into(),
                    tenant_id: tenant_id.into(),
                    dependency: dep,
                    workflow_class: wf,
                    reason,
                    policy_version: self.policy_version.clone(),
                    admitted_at: Utc::now(),
                };
                warn!(
                    dep = dep.as_str(),
                    workflow = wf.as_str(),
                    policy_version = %self.policy_version,
                    marker_id = %marker.marker_id,
                    "fail-policy: ADMIT with marker"
                );
                FailMode::Admit { marker }
            }
        }
    }
}

// ============================================================================
// Env-loader
// ============================================================================

/// Build the matrix from `<PREFIX>_FAIL_POLICY_JSON`. If the var is
/// unset, returns `default_fail_closed()` — the safe production
/// baseline.
pub fn matrix_from_env(prefix: &str, profile: &str) -> Result<FailPolicyMatrix, PolicyError> {
    let var = format!("{prefix}_FAIL_POLICY_JSON");
    match std::env::var(&var) {
        Ok(raw) if !raw.trim().is_empty() => FailPolicyMatrix::from_json(&raw, profile),
        _ => Ok(FailPolicyMatrix::default_fail_closed()),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matrix_blocks_every_combination() {
        let m = FailPolicyMatrix::default_fail_closed();
        for dep in [
            Dependency::Ledger,
            Dependency::CanonicalIngest,
            Dependency::Pricing,
            Dependency::Signing,
            Dependency::ProviderReconciliation,
            Dependency::Approval,
            Dependency::Dashboard,
            Dependency::Export,
        ] {
            for wf in [
                WorkflowClass::Monetary,
                WorkflowClass::NonMonetaryTool,
                WorkflowClass::ObservabilityOnly,
            ] {
                assert_eq!(m.lookup(dep, wf), FailPolicy::FailClosed);
            }
        }
    }

    #[test]
    fn observability_open_baseline_only_opens_observability_route() {
        let m = FailPolicyMatrix::observability_open();
        // Monetary stays fail-closed even on observability_open
        // baseline.
        assert_eq!(
            m.lookup(Dependency::Ledger, WorkflowClass::Monetary),
            FailPolicy::FailClosed
        );
        // ObservabilityOnly opens.
        assert_eq!(
            m.lookup(Dependency::Ledger, WorkflowClass::ObservabilityOnly),
            FailPolicy::FailOpenWithMarker
        );
    }

    #[test]
    fn from_json_overlays_overrides_on_baseline() {
        let raw = r#"{
            "_version": "v2024-01",
            "ledger": {"non_monetary_tool": "fail_open_with_marker"}
        }"#;
        let m = FailPolicyMatrix::from_json(raw, "demo").unwrap();
        assert_eq!(m.policy_version, "v2024-01");
        // Override
        assert_eq!(
            m.lookup(Dependency::Ledger, WorkflowClass::NonMonetaryTool),
            FailPolicy::FailOpenWithMarker
        );
        // Untouched cell still fail-closed.
        assert_eq!(
            m.lookup(Dependency::Ledger, WorkflowClass::Monetary),
            FailPolicy::FailClosed
        );
    }

    #[test]
    fn from_json_rejects_fail_open_for_monetary() {
        // Spec invariant: "no fail-open path can debit budget".
        let raw = r#"{"ledger":{"monetary":"fail_open_with_marker"}}"#;
        let err = FailPolicyMatrix::from_json(raw, "demo").unwrap_err();
        match err {
            PolicyError::ParseError(msg) => {
                assert!(msg.contains("monetary"));
                assert!(msg.contains("forbidden"));
            }
            other => panic!("expected ParseError, got {other:?}"),
        }
    }

    #[test]
    fn from_json_in_production_requires_explicit_ack_for_any_fail_open() {
        let raw = r#"{
            "ledger": {"non_monetary_tool": "fail_open_with_marker"}
        }"#;
        let err = FailPolicyMatrix::from_json(raw, "production").unwrap_err();
        assert!(matches!(err, PolicyError::FailOpenWithoutAck));

        let raw_ack = r#"{
            "_acknowledge_risk_of_fail_open": true,
            "ledger": {"non_monetary_tool": "fail_open_with_marker"}
        }"#;
        FailPolicyMatrix::from_json(raw_ack, "production").expect("ack allows fail-open");
    }

    #[test]
    fn from_json_in_demo_does_not_require_ack() {
        let raw = r#"{
            "ledger": {"non_monetary_tool": "fail_open_with_marker"}
        }"#;
        FailPolicyMatrix::from_json(raw, "demo").expect("demo does not require ack");
    }

    #[test]
    fn decide_returns_block_on_fail_closed() {
        let m = FailPolicyMatrix::default_fail_closed();
        let mode = m.decide(
            Dependency::Ledger,
            WorkflowClass::Monetary,
            "transient grpc unavailable",
            "decision-1",
            "tenant-1",
        );
        match mode {
            FailMode::Block { reason } => {
                assert!(reason.contains("ledger"));
                assert!(reason.contains("transient grpc unavailable"));
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn decide_returns_admit_with_marker_on_fail_open_path() {
        let mut m = FailPolicyMatrix::default_fail_closed();
        m.cells.insert(
            (Dependency::Pricing, WorkflowClass::NonMonetaryTool),
            FailPolicy::FailOpenWithMarker,
        );
        m.policy_version = "test-v1".into();

        let mode = m.decide(
            Dependency::Pricing,
            WorkflowClass::NonMonetaryTool,
            "pricing snapshot stale",
            "decision-2",
            "tenant-2",
        );
        match mode {
            FailMode::Admit { marker } => {
                assert_eq!(marker.dependency, Dependency::Pricing);
                assert_eq!(marker.workflow_class, WorkflowClass::NonMonetaryTool);
                assert_eq!(marker.policy_version, "test-v1");
                assert_eq!(marker.tenant_id, "tenant-2");
                assert_eq!(marker.decision_id, "decision-2");
                assert_eq!(marker.reason, "pricing snapshot stale");
                // Marker has a fresh UUID v7.
                assert_ne!(marker.marker_id, Uuid::nil());
            }
            other => panic!("expected Admit, got {other:?}"),
        }
    }

    #[test]
    fn from_json_rejects_unknown_dependency() {
        let raw = r#"{"unicorn":{"monetary":"fail_closed"}}"#;
        let err = FailPolicyMatrix::from_json(raw, "demo").unwrap_err();
        assert!(matches!(err, PolicyError::ParseError(_)));
    }

    #[test]
    fn from_json_rejects_unknown_workflow_class() {
        let raw = r#"{"ledger":{"unicorn":"fail_closed"}}"#;
        let err = FailPolicyMatrix::from_json(raw, "demo").unwrap_err();
        assert!(matches!(err, PolicyError::ParseError(_)));
    }

    #[test]
    fn from_json_rejects_unknown_policy_value() {
        let raw = r#"{"ledger":{"non_monetary_tool":"warn_only"}}"#;
        let err = FailPolicyMatrix::from_json(raw, "demo").unwrap_err();
        assert!(matches!(err, PolicyError::ParseError(_)));
    }

    #[test]
    fn audit_marker_serializes_to_stable_json() {
        let marker = AuditMarker {
            marker_id: Uuid::nil(),
            decision_id: "d1".into(),
            tenant_id: "t1".into(),
            dependency: Dependency::Ledger,
            workflow_class: WorkflowClass::NonMonetaryTool,
            reason: "transient".into(),
            policy_version: "v1".into(),
            admitted_at: chrono::DateTime::parse_from_rfc3339("2026-05-09T20:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        };
        let json = serde_json::to_value(&marker).unwrap();
        // Field names are stable so audit consumers can parse safely.
        assert_eq!(json["dependency"], "ledger");
        assert_eq!(json["workflow_class"], "non_monetary_tool");
        assert_eq!(json["tenant_id"], "t1");
        assert_eq!(json["policy_version"], "v1");
    }

    #[test]
    fn dependency_workflow_class_round_trip_through_str() {
        for d in [
            Dependency::Ledger,
            Dependency::CanonicalIngest,
            Dependency::Pricing,
            Dependency::Signing,
            Dependency::ProviderReconciliation,
            Dependency::Approval,
            Dependency::Dashboard,
            Dependency::Export,
        ] {
            assert_eq!(Dependency::parse(d.as_str()), Some(d));
        }
        for w in [
            WorkflowClass::Monetary,
            WorkflowClass::NonMonetaryTool,
            WorkflowClass::ObservabilityOnly,
        ] {
            assert_eq!(WorkflowClass::parse(w.as_str()), Some(w));
        }
        assert!(Dependency::parse("nope").is_none());
        assert!(WorkflowClass::parse("nope").is_none());
    }

    #[test]
    fn matrix_from_env_falls_back_to_default_when_var_unset() {
        std::env::remove_var("S22_TEST_FAIL_POLICY_JSON");
        let m = matrix_from_env("S22_TEST", "production").unwrap();
        assert_eq!(m.policy_version, "default-fail-closed");
    }
}
