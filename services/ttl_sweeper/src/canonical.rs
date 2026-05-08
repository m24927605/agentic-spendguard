//! Canonical request_hash for ledger.Release.
//!
//! BYTE-EXACT match with services/ledger/src/handlers/release.rs:217-225.
//! Receiver passes via Idempotency.request_hash; ledger handler validates
//! equality (Codex TTL r1 P1.2).

use sha2::{Digest, Sha256};
use uuid::Uuid;

pub fn release_hash(
    tenant_id: &Uuid,
    reservation_set_id: &Uuid,
    decision_id: &Uuid,
    reason_str: &str,
) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"v1:release:business_intent:");
    h.update(tenant_id.to_string().as_bytes());
    h.update(b"|reservation_set|");
    h.update(reservation_set_id.to_string().as_bytes());
    h.update(b"|decision|");
    h.update(decision_id.to_string().as_bytes());
    h.update(b"|reason|");
    h.update(reason_str.as_bytes());
    h.finalize().into()
}

/// Mirror services/ledger/src/handlers/reserve_set.rs:482-492:
/// sha256(decision_id_bytes || ":reservation_set")[0..16] →
/// UUIDv4-style (set version + RFC variant bits).
pub fn derive_reservation_set_id(decision_id: &Uuid) -> Uuid {
    let mut h = Sha256::new();
    h.update(decision_id.as_bytes()); // RAW UUID bytes (not string)
    h.update(b":reservation_set");
    let bytes: [u8; 32] = h.finalize().into();
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&bytes[..16]);
    buf[6] = (buf[6] & 0x0f) | 0x40; // v4
    buf[8] = (buf[8] & 0x3f) | 0x80; // RFC variant
    Uuid::from_bytes(buf)
}
