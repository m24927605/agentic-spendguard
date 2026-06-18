//! Inbound caller SVID URI-SAN pinning interceptor.
//!
//! Closes the auth-trust gap where the ledger gRPC handlers trust the
//! wire-supplied `tenant_id` / `workload_instance_id` with no
//! application-layer identity binding. A prior pass made production
//! REFUSE to start without mTLS (see `main.rs`); this module adds the
//! actual inbound SVID check on top of WebPKI CA-chain validation.
//!
//! Model (mirrors the sidecar HTTP companion's
//! `expected_client_spiffe_uri`, see
//! `services/sidecar/src/http_companion/mtls.rs`, and the per-tenant SVID
//! convention in `services/output_predictor/src/plugin_svid.rs`):
//!
//! * tonic's `ServerTlsConfig` with `client_ca_root` already performs
//!   WebPKI client-cert verification at the handshake. That proves only
//!   that the peer holds *some* cert issued by the shared SpendGuard CA.
//!   Any same-CA workload could then claim any wire `tenant_id`.
//! * When the operator sets `expected_caller_spiffe_uri` (env
//!   `SPENDGUARD_LEDGER_EXPECTED_CALLER_SPIFFE_URI`), this interceptor
//!   extracts the peer leaf cert's URI SAN(s) — exposed by tonic through
//!   request extensions (`request.peer_certs()`) — and REQUIRES an exact
//!   match. A missing cert, a parse failure, or a mismatch all fail
//!   closed with `Status::unauthenticated`, before any handler runs.
//! * When UNSET (`None`): NO-OP with a one-time `warn!`. The demo, which
//!   does not configure the pin, stays green. Fail-closed is not
//!   weakened: production additionally refuses to start without mTLS, and
//!   the operator is expected to set the pin in production.
//!
//! The interceptor binds the *connection* identity (the caller workload
//! SVID), which is the trust anchor; the in-body `tenant_id` is then only
//! defense in depth (this ledger instance is per-tenant — see
//! `Config::tenant_id`). This matches the HARDEN_08 stance that "the SVID
//! URI SAN is the tenant binding authority; request metadata is only
//! defense in depth".

use std::sync::Arc;

use rustls::pki_types::CertificateDer;
use tonic::{Request, Status};
use tracing::warn;
use x509_parser::extensions::GeneralName;

/// Clonable interceptor state. Cheap to clone (one `Arc<Option<String>>`)
/// so it can be installed via `InterceptedService` / `interceptor()`,
/// both of which require `Clone`.
#[derive(Clone)]
pub struct CallerSvidPin {
    expected_uri: Arc<Option<String>>,
}

impl CallerSvidPin {
    /// Build the interceptor from the operator-configured expected URI.
    ///
    /// `None` => no-op (with a startup warn emitted by [`Self::log_mode`]).
    pub fn new(expected_caller_spiffe_uri: Option<String>) -> Self {
        Self {
            expected_uri: Arc::new(expected_caller_spiffe_uri),
        }
    }

    /// Emit a single startup log describing the active mode. Called once
    /// from `main()` so the operator sees, at boot, whether the pin is
    /// enforced or the connection rests on WebPKI CA-chain validation
    /// alone. Mirrors the one-time `warn!` in the sidecar HTTP companion.
    pub fn log_mode(&self) {
        match self.expected_uri.as_ref() {
            Some(uri) => {
                tracing::info!(
                    expected_caller_spiffe_uri = %uri,
                    "ledger mTLS: inbound caller SVID URI-SAN pinning ENABLED"
                );
            }
            None => {
                warn!(
                    "ledger mTLS: no expected caller SPIFFE-URI SAN configured; \
                     caller identity rests on WebPKI CA-chain validation + \
                     in-body tenant_id only. Any workload holding a same-CA \
                     cert can claim any tenant_id/workload_instance_id. Set \
                     SPENDGUARD_LEDGER_EXPECTED_CALLER_SPIFFE_URI to \
                     cryptographically pin the calling workload."
                );
            }
        }
    }

    /// Whether the pin is enforced (test/introspection helper).
    pub fn is_enforced(&self) -> bool {
        self.expected_uri.is_some()
    }

    /// tonic [`Interceptor`](tonic::service::Interceptor) entry point.
    ///
    /// `request` is a `Request<()>`; tonic copies the connection
    /// extensions onto it before invoking the interceptor (see
    /// `InterceptedService::call`), so `peer_certs()` returns the
    /// handshake's verified peer chain.
    pub fn intercept(&self, request: Request<()>) -> Result<Request<()>, Status> {
        // mTLS already happened at the transport; `peer_certs()` reads the
        // CA-validated chain (`Arc<Vec<_>>`) from the connection
        // extensions. Deref to a slice for the pure check.
        let peer_certs = request.peer_certs();
        self.check_peer_certs(peer_certs.as_deref().map(Vec::as_slice))?;
        Ok(request)
    }

    /// Pure pin-check over the (optional) peer cert chain. Split out from
    /// [`Self::intercept`] so it is unit-testable without fabricating
    /// tonic's non-constructible `TlsConnectInfo` extension.
    ///
    /// * pin unset            -> `Ok(())` (no-op).
    /// * pin set, no chain    -> `Status::unauthenticated` (fail closed).
    /// * pin set, empty chain -> `Status::unauthenticated` (fail closed).
    /// * pin set, parse error -> `Status::unauthenticated` (fail closed).
    /// * pin set, no match    -> `Status::unauthenticated` (fail closed).
    /// * pin set, exact match -> `Ok(())`.
    pub fn check_peer_certs(
        &self,
        peer_certs: Option<&[CertificateDer<'static>]>,
    ) -> Result<(), Status> {
        let expected = match self.expected_uri.as_ref() {
            // No pin configured: explicit no-op. (Production hardening is
            // enforced by the no-mTLS-in-production startup refusal in
            // main.rs, not here.)
            None => return Ok(()),
            Some(uri) => uri,
        };

        // `None` means the connection carried no client cert (e.g. the
        // server was somehow started without `client_ca_root`); fail
        // closed when a pin is set.
        let peer_certs = peer_certs.ok_or_else(|| {
            Status::unauthenticated(
                "caller SVID pin configured but connection presented no client certificate",
            )
        })?;

        let leaf = peer_certs.first().ok_or_else(|| {
            Status::unauthenticated("caller SVID pin configured but peer certificate chain is empty")
        })?;

        let leaf_uris = uri_sans_from_der(leaf.as_ref()).map_err(|e| {
            // A leaf that chains to the CA but cannot be parsed for its
            // URI SAN is rejected (fail-closed), never admitted.
            Status::unauthenticated(format!("caller cert URI SAN parse failed: {e}"))
        })?;

        if leaf_uris.iter().any(|u| u == expected) {
            Ok(())
        } else {
            Err(Status::unauthenticated(format!(
                "caller SVID URI SAN pin mismatch: expected `{expected}`, \
                 leaf presented {leaf_uris:?}"
            )))
        }
    }
}

/// Extract all URI SANs from a DER-encoded X.509 leaf certificate.
///
/// Same extraction approach as
/// `services/sidecar/src/http_companion/mtls.rs::uri_sans_from_der` and
/// `services/output_predictor/src/plugin_svid.rs` (x509-parser 0.16). A
/// cert with no `subjectAltName` extension is an error (fail-closed);
/// a cert whose SANs contain no URI entry yields an empty vec, which the
/// caller treats as a pin mismatch.
fn uri_sans_from_der(der: &[u8]) -> Result<Vec<String>, anyhow::Error> {
    use anyhow::{anyhow, Context};

    let (_, cert) =
        x509_parser::parse_x509_certificate(der).context("parse caller x509 certificate")?;
    let san = cert
        .tbs_certificate
        .subject_alternative_name()
        .context("parse caller subjectAltName extension")?
        .ok_or_else(|| anyhow!("caller certificate missing subjectAltName"))?;
    Ok(san
        .value
        .general_names
        .iter()
        .filter_map(|name| match name {
            GeneralName::URI(uri) => Some((*uri).to_string()),
            _ => None,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{CertificateParams, KeyPair, SanType};

    const EXPECTED: &str = "spiffe://spendguard.platform/sidecar/ns/sg/sa/egress";

    /// Build a self-signed leaf cert DER carrying a single URI SAN.
    fn leaf_with_uri_san(uri: &str) -> CertificateDer<'static> {
        let key = KeyPair::generate().expect("rcgen key");
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params
            .subject_alt_names
            .push(SanType::URI(uri.to_string().try_into().unwrap()));
        let cert = params.self_signed(&key).unwrap();
        CertificateDer::from(cert.der().to_vec())
    }

    /// Build a leaf cert DER with a DNS SAN only (no URI SAN).
    fn leaf_with_dns_san_only(dns: &str) -> CertificateDer<'static> {
        let key = KeyPair::generate().expect("rcgen key");
        let params = CertificateParams::new(vec![dns.to_string()]).unwrap();
        let cert = params.self_signed(&key).unwrap();
        CertificateDer::from(cert.der().to_vec())
    }

    #[test]
    fn uri_sans_from_der_extracts_uri() {
        let der = leaf_with_uri_san(EXPECTED);
        let uris = uri_sans_from_der(der.as_ref()).expect("parse leaf SAN");
        assert_eq!(uris, vec![EXPECTED.to_string()]);
    }

    #[test]
    fn unset_pin_is_noop_passes_through_without_cert() {
        // None => no-op even when there is no peer cert at all.
        let pin = CallerSvidPin::new(None);
        assert!(!pin.is_enforced());
        // intercept() path: passes the request straight through.
        assert!(pin.intercept(Request::new(())).is_ok());
        // check_peer_certs() path: no-op regardless of input.
        assert!(pin.check_peer_certs(None).is_ok());
        assert!(pin.check_peer_certs(Some(&[])).is_ok());
    }

    #[test]
    fn enforced_pin_accepts_exact_match() {
        let pin = CallerSvidPin::new(Some(EXPECTED.to_string()));
        assert!(pin.is_enforced());
        let chain = [leaf_with_uri_san(EXPECTED)];
        assert!(
            pin.check_peer_certs(Some(&chain)).is_ok(),
            "exact URI SAN match must pass"
        );
    }

    #[test]
    fn enforced_pin_rejects_uri_mismatch() {
        let pin = CallerSvidPin::new(Some(EXPECTED.to_string()));
        let other = "spiffe://spendguard.platform/sidecar/ns/sg/sa/other";
        let chain = [leaf_with_uri_san(other)];
        let err = pin
            .check_peer_certs(Some(&chain))
            .expect_err("mismatched URI must fail closed");
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
        assert!(err.message().contains("pin mismatch"));
    }

    #[test]
    fn enforced_pin_rejects_missing_client_cert() {
        // Pin configured but the connection carries no peer cert => fail closed.
        let pin = CallerSvidPin::new(Some(EXPECTED.to_string()));
        let err = pin
            .check_peer_certs(None)
            .expect_err("missing client cert must fail closed");
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
        assert!(err.message().contains("no client certificate"));
    }

    #[test]
    fn enforced_pin_rejects_empty_chain() {
        let pin = CallerSvidPin::new(Some(EXPECTED.to_string()));
        let err = pin
            .check_peer_certs(Some(&[]))
            .expect_err("empty chain must fail closed");
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
        assert!(err.message().contains("chain is empty"));
    }

    #[test]
    fn enforced_pin_rejects_cert_without_uri_san() {
        // Cert parses but carries only a DNS SAN, no URI SAN => no URI to
        // match the pin => mismatch (fail closed).
        let pin = CallerSvidPin::new(Some(EXPECTED.to_string()));
        let chain = [leaf_with_dns_san_only("ledger.local")];
        let err = pin
            .check_peer_certs(Some(&chain))
            .expect_err("cert without a URI SAN must fail closed");
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
        assert!(err.message().contains("pin mismatch"));
    }
}
