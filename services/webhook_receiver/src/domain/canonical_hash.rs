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
