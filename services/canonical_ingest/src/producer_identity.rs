//! Producer-identity binding for the canonical-ingest mTLS trust
//! boundary.
//!
//! ## Why this exists (auth-trust)
//!
//! `build_server_tls_config` (main.rs) configures `client_ca_root`, so
//! the gRPC server only completes a handshake with a client cert signed
//! by the configured CA. But CA issuance alone is NOT identity: any
//! workload holding *any* CA-valid client cert could submit
//! `AppendEvents` claiming an arbitrary `producer_id` / `tenant_id`, and
//! the handler trusted those fields verbatim. That permits cross-tenant
//! audit injection / producer spoofing at the trust boundary — the trust
//! collapses to "has a cert from our CA".
//!
//! This module closes the gap on the ingest path by extracting the
//! client leaf cert's SPIFFE URI SAN (the per-workload identity authority)
//! and asserting it binds to the declared `producer_id`. It mirrors the
//! HARDEN_08 SVID parsing used by `output_predictor::plugin_svid`.
//!
//! ## Rollout (fail-closed when enabled)
//!
//! Existing in-cluster forwarders may present certs that do not yet carry
//! a SPIFFE URI SAN, so enforcement is gated behind
//! `Config::require_producer_spiffe_san` (default OFF). Operators issue
//! `spiffe://spendguard.platform/audit-producer/<producer_id>` SANs to
//! every producer first, then flip the flag. Once enabled the binding is
//! strictly fail-closed: a missing / unparseable / mismatched SAN is
//! rejected with `Unauthenticated` — never admitted.
//!
//! ## Scope / defense-in-depth note
//!
//! A perfect SAN check still admits anyone presenting *some* CA-valid
//! cert whose SAN happens to match the claimed producer_id. The deeper
//! authorization binding — a `key_id -> {producer_id, allowed_tenants}`
//! map in the shared signing trust store — lives in the
//! `spendguard-signing` crate (outside this service) and is tracked
//! separately; this module is the ingest-side enforcement point it would
//! plug into.

use x509_parser::extensions::GeneralName;
use x509_parser::prelude::*;

/// SPIFFE trust-domain prefix for audit producers. The URI SAN suffix
/// after this prefix is the declared `producer_id`.
pub const AUDIT_PRODUCER_SVID_PREFIX: &str = "spiffe://spendguard.platform/audit-producer/";

/// The expected SPIFFE URI SAN for a given producer_id.
pub fn subject_uri_for_producer(producer_id: &str) -> String {
    format!("{AUDIT_PRODUCER_SVID_PREFIX}{producer_id}")
}

/// Why a producer-identity binding check failed. The caller maps every
/// variant to gRPC `Unauthenticated` — the binding is fail-closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingError {
    /// mTLS enforcement is required but no peer client certificate was
    /// presented (or the chain was empty).
    NoPeerCert,
    /// The leaf certificate could not be parsed.
    UnparseableCert(String),
    /// The leaf certificate carries no usable SPIFFE URI SAN.
    NoSpiffeSan,
    /// The certificate's SPIFFE URI SAN does not bind to the declared
    /// producer_id.
    ProducerMismatch { expected: String, found: String },
}

impl std::fmt::Display for BindingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BindingError::NoPeerCert => {
                write!(f, "no client certificate presented on mTLS connection")
            }
            BindingError::UnparseableCert(e) => {
                write!(f, "client certificate could not be parsed: {e}")
            }
            BindingError::NoSpiffeSan => write!(
                f,
                "client certificate has no `{AUDIT_PRODUCER_SVID_PREFIX}*` SPIFFE URI SAN"
            ),
            BindingError::ProducerMismatch { expected, found } => write!(
                f,
                "client certificate identity `{found}` does not match declared producer_id (expected SAN `{expected}`)"
            ),
        }
    }
}

impl std::error::Error for BindingError {}

/// Extract the single SpendGuard audit-producer SPIFFE URI SAN from a DER
/// leaf certificate.
///
/// Fail-closed: requires exactly one URI SAN that starts with the
/// audit-producer prefix. Multiple URI SANs, a wrong-prefix URI, or no
/// URI SAN at all are all rejected (a cert that smuggles a second
/// identity must not be ambiguously accepted).
pub fn extract_producer_spiffe_uri(leaf_der: &[u8]) -> Result<String, BindingError> {
    let (_, cert) = parse_x509_certificate(leaf_der)
        .map_err(|e| BindingError::UnparseableCert(e.to_string()))?;

    let san = cert
        .tbs_certificate
        .subject_alternative_name()
        .map_err(|e| BindingError::UnparseableCert(e.to_string()))?
        .ok_or(BindingError::NoSpiffeSan)?;

    let uris: Vec<&str> = san
        .value
        .general_names
        .iter()
        .filter_map(|name| match name {
            GeneralName::URI(uri) => Some(*uri),
            _ => None,
        })
        .collect();

    select_exact_audit_producer_uri(&uris)
}

fn select_exact_audit_producer_uri(uris: &[&str]) -> Result<String, BindingError> {
    // Exactly one URI SAN, and it must carry our prefix. Any deviation is
    // rejected so a cert cannot present two identities ambiguously.
    let matching: Vec<&&str> = uris
        .iter()
        .filter(|u| u.starts_with(AUDIT_PRODUCER_SVID_PREFIX))
        .collect();
    match (uris.len(), matching.as_slice()) {
        (1, [only]) => Ok((**only).to_string()),
        _ => Err(BindingError::NoSpiffeSan),
    }
}

/// Assert the leaf certificate's SPIFFE identity binds to `producer_id`.
///
/// `leaf_der` is `Some(..)` when the connection presented a client cert,
/// `None` otherwise. With enforcement on, a missing cert is fail-closed
/// (`NoPeerCert`). The comparison against the declared `producer_id` is
/// exact.
pub fn assert_producer_binding(
    leaf_der: Option<&[u8]>,
    producer_id: &str,
) -> Result<(), BindingError> {
    let der = leaf_der.ok_or(BindingError::NoPeerCert)?;
    let found = extract_producer_spiffe_uri(der)?;
    let expected = subject_uri_for_producer(producer_id);
    if found != expected {
        return Err(BindingError::ProducerMismatch { expected, found });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_uri_round_trips_producer_id() {
        assert_eq!(
            subject_uri_for_producer("outbox-forwarder:demo:v1"),
            "spiffe://spendguard.platform/audit-producer/outbox-forwarder:demo:v1"
        );
    }

    #[test]
    fn exact_uri_san_requires_single_prefixed_uri() {
        let good = subject_uri_for_producer("sidecar:test");
        assert_eq!(
            select_exact_audit_producer_uri(&[&good]).unwrap(),
            good
        );
        // No URI SAN.
        assert_eq!(
            select_exact_audit_producer_uri(&[]),
            Err(BindingError::NoSpiffeSan)
        );
        // Wrong prefix.
        assert_eq!(
            select_exact_audit_producer_uri(&["spiffe://other/audit-producer/x"]),
            Err(BindingError::NoSpiffeSan)
        );
        // Smuggled second identity is rejected (ambiguous).
        assert_eq!(
            select_exact_audit_producer_uri(&[&good, "spiffe://other/id"]),
            Err(BindingError::NoSpiffeSan)
        );
    }

    #[test]
    fn missing_peer_cert_fails_closed() {
        assert_eq!(
            assert_producer_binding(None, "outbox-forwarder:demo:v1"),
            Err(BindingError::NoPeerCert)
        );
    }

    #[test]
    fn binding_error_maps_to_messages() {
        let e = BindingError::ProducerMismatch {
            expected: "spiffe://spendguard.platform/audit-producer/a".into(),
            found: "spiffe://spendguard.platform/audit-producer/b".into(),
        };
        assert!(format!("{e}").contains("does not match declared producer_id"));
    }
}
