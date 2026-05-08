//! Storage-class routing per Trace §10.2.
//!
//! Audit events go to immutable_audit_log (7yr SOX retention).
//! Ledger lifecycle / approval / rollback / tombstone events go to
//! canonical_raw_log (7yr; hashes only — full text omitted).
//! Verbose runtime payloads go to profile_payload_blob (tenant policy
//! retention; RTBF deletable).
//!
//! Phase 1 POC stores all classes in a single Postgres table with a
//! `storage_class` column. Phase 1 後段 splits into per-class backends
//! with separate retention / RTBF / cost tiers.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageClass {
    ImmutableAuditLog,
    CanonicalRawLog,
    ProfilePayloadBlob,
}

impl StorageClass {
    pub fn as_db_str(self) -> &'static str {
        match self {
            StorageClass::ImmutableAuditLog => "immutable_audit_log",
            StorageClass::CanonicalRawLog => "canonical_raw_log",
            StorageClass::ProfilePayloadBlob => "profile_payload_blob",
        }
    }
}

/// Map a CloudEvents `type` to its canonical storage class (per Trace §10.2).
/// Unknown types default to canonical_raw_log; ingest will quarantine for
/// schema validation if the bundle does not declare the type.
pub fn classify(event_type: &str) -> StorageClass {
    if event_type.starts_with("spendguard.audit.")
        || event_type == "spendguard.tombstone"
    {
        StorageClass::ImmutableAuditLog
    } else if event_type.starts_with("spendguard.ledger.")
        || event_type.starts_with("spendguard.approval.")
        || event_type.starts_with("spendguard.dispute.")
        || event_type.starts_with("spendguard.refund.")
        || event_type == "spendguard.decision"
        || event_type == "spendguard.rollback"
        || event_type == "spendguard.region_failover_promoted"
    {
        StorageClass::CanonicalRawLog
    } else {
        StorageClass::ProfilePayloadBlob
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_events_go_to_immutable_log() {
        assert_eq!(classify("spendguard.audit.decision"), StorageClass::ImmutableAuditLog);
        assert_eq!(classify("spendguard.audit.outcome"), StorageClass::ImmutableAuditLog);
        assert_eq!(classify("spendguard.tombstone"), StorageClass::ImmutableAuditLog);
    }

    #[test]
    fn ledger_lifecycle_go_to_raw_log() {
        assert_eq!(classify("spendguard.ledger.reservation"), StorageClass::CanonicalRawLog);
        assert_eq!(classify("spendguard.ledger.commit"), StorageClass::CanonicalRawLog);
        assert_eq!(classify("spendguard.refund.credit_received"), StorageClass::CanonicalRawLog);
    }

    #[test]
    fn unknown_defaults_to_payload_blob() {
        assert_eq!(classify("unknown.event"), StorageClass::ProfilePayloadBlob);
    }
}
