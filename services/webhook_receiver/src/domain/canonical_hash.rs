//! Canonical request_hash for webhook → ledger transition.
//!
//! BYTE-EXACT match with services/ledger/src/handlers/provider_report.rs:260-280
//! and services/ledger/src/handlers/invoice_reconcile.rs:342-362.
//! Receiver hashes locally for webhook_dedupe + sends same hash to
//! ledger via Idempotency.request_hash; ledger handler recomputes and
//! validates equality (existing handler behavior).
//!
//! Test vectors for byte-exactness (Codex r3 grounding):
//!   provider_report:    e67d3ea13bd57d41e005ae47f3470adfcc5ab60a46b934befb787261c1660942
//!   invoice_reconcile:  2703d5982eb2d9e3dcff01ab42652adf5250e62d053ebe1385169beda8cbcc39

use num_bigint::BigInt;
use sha2::{Digest, Sha256};

#[allow(clippy::too_many_arguments)]
pub fn provider_report_hash(
    tenant_id: &str,
    reservation_id: &str,
    provider_amount: &BigInt,
    unit_id: &str,
    pricing_version: &str,
    price_snapshot_hash: &[u8], // RAW bytes (NOT hex)
    fx_rate_version: &str,
    unit_conversion_version: &str,
    provenance: &str, // namespaced provider_response_metadata
) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"v1:provider_report:business_intent:");
    h.update(tenant_id.as_bytes());
    h.update(b"|reservation|");
    h.update(reservation_id.as_bytes());
    h.update(b"|provider_amount|");
    h.update(provider_amount.to_string().as_bytes());
    h.update(b"|unit|");
    h.update(unit_id.as_bytes());
    h.update(b"|pricing|");
    h.update(pricing_version.as_bytes());
    h.update(price_snapshot_hash);
    h.update(fx_rate_version.as_bytes());
    h.update(unit_conversion_version.as_bytes());
    h.update(b"|provenance|");
    h.update(provenance.as_bytes());
    h.finalize().into()
}

#[allow(clippy::too_many_arguments)]
pub fn invoice_reconcile_hash(
    tenant_id: &str,
    reservation_id: &str,
    invoice_amount: &BigInt,
    unit_id: &str,
    pricing_version: &str,
    price_snapshot_hash: &[u8], // RAW bytes
    fx_rate_version: &str,
    unit_conversion_version: &str,
    provenance: &str, // namespaced provider_invoice_id
) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"v1:invoice_reconcile:business_intent:");
    h.update(tenant_id.as_bytes());
    h.update(b"|reservation|");
    h.update(reservation_id.as_bytes());
    h.update(b"|invoice_amount|");
    h.update(invoice_amount.to_string().as_bytes());
    h.update(b"|unit|");
    h.update(unit_id.as_bytes());
    h.update(b"|pricing|");
    h.update(pricing_version.as_bytes());
    h.update(price_snapshot_hash);
    h.update(fx_rate_version.as_bytes());
    h.update(unit_conversion_version.as_bytes());
    h.update(b"|provenance|");
    h.update(provenance.as_bytes());
    h.finalize().into()
}

// ============================================================================
// Phase 5 GA hardening S10: provider usage record idempotency
// ============================================================================
//
// Per-provider, per-record idempotency. Used as the
// `provider_usage_records.idempotency_key` UNIQUE column. Two
// observations of the same provider event (e.g. duplicate webhook
// delivery, OR poller re-fetching the same window) produce the same
// hash and a UNIQUE-violation rejection.
//
// IMPORTANT: this is a DIFFERENT scope from provider_report_hash.
// provider_report_hash is reservation-scoped (one report per
// reservation per amount). This is record-scoped (one observation per
// provider event id) — multiple records may flow into one report
// after matching.

pub fn provider_usage_record_hash(
    provider: &str,
    provider_account: &str,
    provider_event_id: &str,
    event_kind: &str,
) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"v1:provider_usage_record:idempotency:");
    h.update(provider.as_bytes());
    h.update(b"|account|");
    h.update(provider_account.as_bytes());
    h.update(b"|event_id|");
    h.update(provider_event_id.as_bytes());
    h.update(b"|kind|");
    h.update(event_kind.as_bytes());
    h.finalize().into()
}

#[cfg(test)]
mod s10_tests {
    use super::*;

    #[test]
    fn provider_usage_record_hash_is_deterministic() {
        let a = provider_usage_record_hash(
            "openai",
            "acct-1",
            "evt-abc",
            "completion",
        );
        let b = provider_usage_record_hash(
            "openai",
            "acct-1",
            "evt-abc",
            "completion",
        );
        assert_eq!(a, b);
    }

    #[test]
    fn provider_usage_record_hash_changes_when_any_field_changes() {
        let base = provider_usage_record_hash("openai", "a", "e", "k");
        for variant in [
            provider_usage_record_hash("anthropic", "a", "e", "k"),
            provider_usage_record_hash("openai", "b", "e", "k"),
            provider_usage_record_hash("openai", "a", "f", "k"),
            provider_usage_record_hash("openai", "a", "e", "l"),
        ] {
            assert_ne!(base, variant);
        }
    }
}
