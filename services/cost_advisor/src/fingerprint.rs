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
/// stable form:
///   `scope_type|agent_id|run_id|tool_name|model_family|budget_id`
/// with empty strings for absent fields. New scope projections are
/// appended at the tail (proto `reserved 7..15` after CA-P3.1) so
/// old fingerprints can extend cleanly.
///
/// **CA-P3.1 migration note**: budget_id was added as the 6th field
/// for budget-scoped findings. v0.1 is greenfield — no production
/// cost_findings rows exist yet — so the trailing `|` change is a
/// safe one-time fingerprint bump. Future extensions follow the
/// same tail-append pattern + rule_version bump per spec §11.5 A6.
pub fn canonical_scope_repr(scope: &FindingScope) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}",
        scope.scope_type,
        scope.agent_id,
        scope.run_id,
        scope.tool_name,
        scope.model_family,
        scope.budget_id,
    )
}

/// Compute the canonical fingerprint hex for a candidate finding.
///
/// `tenant_id` is included in the canonical bytes so two tenants
/// computing the "same" tenant-global finding on the same day
/// produce DIFFERENT fingerprints (codex CA-P1 r1 P2). Without
/// tenant in the input, both tenants would emit fingerprint X and
/// any downstream code that looks up findings by fingerprint alone
/// (logging, CLI output, dashboard cross-tenant analytics) would
/// conflate them. The mirror table UNIQUE on (tenant_id, fingerprint)
/// still allows the collision at the DB layer, but the semantic
/// drift is hostile to operators.
///
/// **Migration note** (codex CA-P1 r3 P3): adding `tenant_id` to the
/// canonical input is a breaking change to the fingerprint algorithm.
/// Pre-CA-P1 deployments computing fingerprints WITHOUT tenant_id
/// would write rows under one fingerprint scheme; post-CA-P1 writes
/// produce different fingerprints for the same logical finding.
/// Mitigation: cost_findings + cost_findings_fingerprint_keys ship
/// FRESH in CA-P0 (commit `e52f40a`) — no pre-existing rows to
/// migrate. Future rule-version bumps (per spec §11.5 A6) cover any
/// case where existing findings would need a re-fingerprint. New
/// deployments are unaffected.
///
/// `time_bucket` is the rule's time-bucket label (e.g. an ISO-8601
/// hour string `2026-05-13T07:00:00Z` for `failed_retry_burn_v1`, or
/// a day string `2026-05-13` for `idle_reservation_rate_v1`). Bucket
/// granularity is the rule's choice (spec §11.5 A1).
pub fn compute(
    rule_id: &str,
    tenant_id: &str,
    scope: &FindingScope,
    time_bucket: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(rule_id.as_bytes());
    hasher.update(b"|");
    hasher.update(tenant_id.as_bytes());
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
            budget_id: String::new(),
        }
    }

    const T1: &str = "00000000-0000-4000-8000-000000000001";
    const T2: &str = "00000000-0000-4000-8000-000000000002";

    #[test]
    fn fingerprint_is_64_hex_chars() {
        let fp = compute(
            "idle_reservation_rate_v1",
            T1,
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
            T1,
            &scope(ScopeType::Run, "r1"),
            "2026-05-13T07:00:00Z",
        );
        let fp2 = compute(
            "failed_retry_burn_v1",
            T1,
            &scope(ScopeType::Run, "r1"),
            "2026-05-13T07:00:00Z",
        );
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn fingerprint_changes_with_time_bucket() {
        let s = scope(ScopeType::Run, "r1");
        let fp1 = compute("failed_retry_burn_v1", T1, &s, "2026-05-13T07:00:00Z");
        let fp2 = compute("failed_retry_burn_v1", T1, &s, "2026-05-13T08:00:00Z");
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn fingerprint_differs_across_budgets() {
        // CA-P3.1 r2 P3: two Budget-scoped findings with different
        // budget_ids must produce DIFFERENT fingerprints so the
        // mirror table dedupes per-budget, not per-tenant-day.
        let mk = |bid: &str| FindingScope {
            scope_type: ScopeType::Budget as i32,
            agent_id: String::new(),
            run_id: String::new(),
            tool_name: String::new(),
            model_family: String::new(),
            budget_id: bid.to_string(),
        };
        let s1 = mk("11111111-1111-4111-8111-111111111111");
        let s2 = mk("22222222-2222-4222-8222-222222222222");
        let fp1 = compute("idle_reservation_rate_v1", T1, &s1, "2026-05-14");
        let fp2 = compute("idle_reservation_rate_v1", T1, &s2, "2026-05-14");
        assert_ne!(fp1, fp2, "different budget_ids must yield distinct fingerprints");
    }

    #[test]
    fn fingerprint_differs_across_tenants() {
        // Codex CA-P1 r1 P2 fix: tenant_global findings on the same
        // day for two tenants must produce DISTINCT fingerprints so
        // logs / CLI / dashboard never conflate them.
        let s = scope(ScopeType::TenantGlobal, "");
        let fp1 = compute("idle_reservation_rate_v1", T1, &s, "2026-05-13");
        let fp2 = compute("idle_reservation_rate_v1", T2, &s, "2026-05-13");
        assert_ne!(fp1, fp2);
    }
}
