//! CA-P3: INSERT into `approval_requests` for cost_advisor proposals.
//!
//! Closes the cost_advisor → operator-review loop by writing a row
//! with `proposal_source='cost_advisor'`. The row carries:
//!   * `proposing_finding_id` — FK into `cost_findings_id_keys`
//!     (CA-P1.6); enforces RESTRICT semantics on retention.
//!   * `proposed_dsl_patch` — the RFC-6902 patch from
//!     [`runtime::build_proposed_patch_for_rule`]. Validated by
//!     [`patch_validator::validate`] BEFORE the INSERT; the
//!     `approval_requests_cost_advisor_patch_allowlist` CHECK
//!     constraint is the authoritative gate.
//!
//! Idempotency: `decision_id` is derived deterministically as
//! `uuid_v5(COST_ADVISOR_DECISION_NS, finding_id || rule_version)`.
//! The existing UNIQUE `(tenant_id, decision_id)` on approval_requests
//! means a re-run of the same (finding, rule_version) is a no-op
//! INSERT (`ON CONFLICT DO NOTHING`). Spec §11.5 A1 — cost_advisor
//! rules must be re-runnable.
//!
//! Note: `ON CONFLICT DO NOTHING` semantics mean a denied/expired
//! historical proposal blocks any future proposal for the same
//! (tenant, finding_id, rule_version) tuple. That's intentional for
//! v0.1 — re-proposing requires either a new finding (different
//! fingerprint → different finding_id) or a rule bump. Codex CA-P3
//! r1 P2 flagged this so it's now explicit.
//!
//! TTL: 30 days from creation by default; configurable via
//! `ProposalConfig.ttl_days`.
//!
//! Write path: calls the `cost_advisor_create_proposal` SECURITY
//! DEFINER SP in migration 0043 — NOT a direct INSERT — so the
//! caller cannot bypass the state model (codex CA-P3 r1 P1: a direct
//! INSERT with `state='approved'` + populated `resolved_*` fields
//! would skip approval_events + the pg_notify trigger entirely).

use chrono::{Duration, Utc};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::patch_validator;

/// Configuration for proposal writes. Default values match v0.1
/// spec; operators tune via the bin's CLI flags.
#[derive(Debug, Clone)]
pub struct ProposalConfig {
    pub ttl_days: i64,
}

impl Default for ProposalConfig {
    fn default() -> Self {
        Self { ttl_days: 30 }
    }
}

/// Outcome of a proposal-write attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposalOutcome {
    /// New approval_requests row created.
    Inserted { approval_id: Uuid, decision_id: Uuid },
    /// Row with the same (tenant_id, decision_id) already exists.
    /// Spec §11.5 A1 idempotency — re-fire of the same finding.
    AlreadyExists { decision_id: Uuid },
}

#[derive(Debug, thiserror::Error)]
pub enum ProposalError {
    #[error("patch validation failed: {0}")]
    PatchInvalid(#[from] patch_validator::PatchValidationError),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

/// UUID namespace for cost_advisor decision_id derivation. Stable
/// across the v0.1 codebase so re-runs converge on the same id.
///
/// Uses `Uuid::new_v5` (SHA-1) over `<finding_id>:<rule_version>`.
/// The namespace bytes are a fixed project sentinel — NOT a v4
/// random nor any of the RFC-registered namespaces (DNS/URL/OID/X500).
/// RFC 4122 allows any UUID as a v5 namespace; the resulting v5
/// output is still well-formed regardless. The mnemonic hex pattern
/// makes the namespace recognizable in logs as "cost_advisor".
const COST_ADVISOR_DECISION_NS: Uuid = Uuid::from_bytes([
    0xc0, 0x57, 0xad, 0x71, // "cost ad" prefix as mnemonic
    0x71, 0x50, 0xa4, 0x71, // "vi sor"
    0x71, 0x44, 0xec, 0x15, // "dec is"
    0x10, 0x10, 0x10, 0x10, // pad
]);

/// Derive the deterministic decision_id for a (finding_id, rule_version)
/// pair. Same input → same output, so the existing UNIQUE
/// `(tenant_id, decision_id)` on approval_requests dedupes re-fires.
pub fn derive_decision_id(finding_id: Uuid, rule_version: i32) -> Uuid {
    let name = format!("{}:{}", finding_id, rule_version);
    Uuid::new_v5(&COST_ADVISOR_DECISION_NS, name.as_bytes())
}

/// Write (or no-op on conflict) an approval_requests row for the given
/// cost_advisor finding.
///
/// Pre-conditions:
///   * `finding_id` must exist in `cost_findings_id_keys` (enforced by
///     the FK chain from CA-P1.6). The runtime ensures this by
///     calling `cost_findings_upsert` before this function.
///   * `proposed_patch` must pass [`patch_validator::validate`]. The
///     check is run here (fail-fast) AND at the DB CHECK constraint
///     (authoritative).
pub async fn write_proposal(
    ledger: &PgPool,
    tenant_id: Uuid,
    finding_id: Uuid,
    rule_version: i32,
    proposed_patch: &Value,
    config: &ProposalConfig,
) -> Result<ProposalOutcome, ProposalError> {
    patch_validator::validate(proposed_patch)?;

    let decision_id = derive_decision_id(finding_id, rule_version);
    let ttl = Utc::now() + Duration::days(config.ttl_days);

    // Call the SECURITY DEFINER SP (migration 0043). The SP hard-codes
    // state='pending' + NULL resolution fields so a buggy caller
    // cannot bypass the resolve_approval_request transition.
    let row: (Option<Uuid>, String) = sqlx::query_as(
        r#"
        SELECT approval_id, outcome
          FROM cost_advisor_create_proposal($1::uuid, $2::uuid, $3::jsonb, $4::uuid, $5::timestamptz)
        "#,
    )
    .bind(tenant_id)
    .bind(decision_id)
    .bind(proposed_patch)
    .bind(finding_id)
    .bind(ttl)
    .fetch_one(ledger)
    .await?;

    Ok(match row {
        (Some(approval_id), ref outcome) if outcome == "inserted" => ProposalOutcome::Inserted {
            approval_id,
            decision_id,
        },
        (_, ref outcome) if outcome == "already_exists" => {
            ProposalOutcome::AlreadyExists { decision_id }
        }
        (_, other) => {
            // Defensive: the SP only returns the two outcomes above.
            return Err(ProposalError::Db(sqlx::Error::Protocol(format!(
                "unexpected outcome from cost_advisor_create_proposal: {}",
                other
            ))));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_id_is_deterministic() {
        let fid = Uuid::parse_str("12345678-1234-1234-1234-123456789012").unwrap();
        let a = derive_decision_id(fid, 1);
        let b = derive_decision_id(fid, 1);
        assert_eq!(a, b);
    }

    #[test]
    fn decision_id_changes_with_rule_version() {
        let fid = Uuid::parse_str("12345678-1234-1234-1234-123456789012").unwrap();
        let v1 = derive_decision_id(fid, 1);
        let v2 = derive_decision_id(fid, 2);
        assert_ne!(v1, v2);
    }

    #[test]
    fn decision_id_changes_with_finding_id() {
        let fid_a = Uuid::parse_str("aaaaaaaa-1234-1234-1234-123456789012").unwrap();
        let fid_b = Uuid::parse_str("bbbbbbbb-1234-1234-1234-123456789012").unwrap();
        assert_ne!(derive_decision_id(fid_a, 1), derive_decision_id(fid_b, 1));
    }
}
