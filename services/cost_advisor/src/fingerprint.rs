//! Fingerprint composition per spec §11.5 A1.
//!
//! ```text
//! fingerprint = SHA-256_hex(
//!     rule_id || "|" || canonical_repr(scope) || "|" || time_bucket_iso8601
//! )
//! ```
//!
//! Stable across nightly re-runs of the same underlying data so that
//! UPSERTs on `cost_findings.(tenant_id, fingerprint)` UNIQUE index
//! idempotently mark the same finding instead of inserting duplicates.

use sha2::{Digest, Sha256};

use crate::proto::cost_advisor::v1::FindingScope;

/// Canonical-form serialization of a [`FindingScope`] for fingerprinting.
///
/// proto3 lacks a canonical JSON spec strong enough for hashing
/// (field ordering / default-presence quirks). We encode our own
/// stable form: `scope_type|agent_id|run_id|tool_name|model_family`
/// with empty strings for absent fields. New scope projections are
/// added at the tail (proto `reserved 6..15`) so old fingerprints
/// remain stable across schema additions.
pub fn canonical_scope_repr(scope: &FindingScope) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        scope.scope_type,
        scope.agent_id,
        scope.run_id,
        scope.tool_name,
        scope.model_family,
    )
}

/// Compute the canonical fingerprint hex for a candidate finding.
///
/// `time_bucket` is the rule's time-bucket label (e.g. an ISO-8601
/// hour string `2026-05-13T07:00:00Z` for `failed_retry_burn_v1`, or
/// a day string `2026-05-13` for `idle_reservation_rate_v1`). Bucket
/// granularity is the rule's choice (spec §11.5 A1).
pub fn compute(rule_id: &str, scope: &FindingScope, time_bucket: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(rule_id.as_bytes());
    hasher.update(b"|");
    hasher.update(canonical_scope_repr(scope).as_bytes());
    hasher.update(b"|");
    hasher.update(time_bucket.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::cost_advisor::v1::ScopeType;

    fn scope(scope_type: ScopeType, run_id: &str) -> FindingScope {
        FindingScope {
            scope_type: scope_type as i32,
            agent_id: String::new(),
            run_id: run_id.to_string(),
            tool_name: String::new(),
            model_family: String::new(),
        }
    }

    #[test]
    fn fingerprint_is_64_hex_chars() {
        let fp = compute(
            "idle_reservation_rate_v1",
            &scope(ScopeType::TenantGlobal, ""),
            "2026-05-13",
        );
        assert_eq!(fp.len(), 64);
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn fingerprint_is_deterministic() {
        let fp1 = compute(
            "failed_retry_burn_v1",
            &scope(ScopeType::Run, "r1"),
            "2026-05-13T07:00:00Z",
        );
        let fp2 = compute(
            "failed_retry_burn_v1",
            &scope(ScopeType::Run, "r1"),
            "2026-05-13T07:00:00Z",
        );
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn fingerprint_changes_with_time_bucket() {
        let s = scope(ScopeType::Run, "r1");
        let fp1 = compute("failed_retry_burn_v1", &s, "2026-05-13T07:00:00Z");
        let fp2 = compute("failed_retry_burn_v1", &s, "2026-05-13T08:00:00Z");
        assert_ne!(fp1, fp2);
    }
}
