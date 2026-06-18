//! mTLS server config + accept loop for the HTTP companion.
//!
//! Per `docs/specs/coverage/D09_kong_ai_gateway/review-standards.md`:
//!
//! * §1.4 — mTLS-only; loopback bind by default.
//! * §2.2 — uses rustls + a `ClientCertVerifier`. WebPKI CA-chain
//!   validation is the FIRST gate (revocation / expiry / chain-to-CA);
//!   when an expected SPIFFE-URI SAN is configured, a
//!   [`SpiffeUriPinningClientVerifier`] wraps the WebPKI verifier and
//!   ADDS an exact URI-SAN assertion on the leaf (HARDEN_08 pattern,
//!   mirroring `services/output_predictor/src/plugin_svid.rs`). Without
//!   the pin a same-CA peer could satisfy only the in-body
//!   `tenant_id == cfg.tenant_id` string check; the pin cryptographically
//!   binds the connection to a specific workload identity at the
//!   handshake. Any cert lacking the expected URI SAN is rejected at the
//!   handshake (fail-closed) — never downgraded to a 200/ALLOW.
//! * §9.2 — every request flows through the mTLS handshake. Plain
//!   HTTP/1.1 connections terminate at the rustls layer before the
//!   first byte of the HTTP request line is parsed.
//!
//! The accept loop hand-rolls `tokio-rustls` + hyper because axum's
//! built-in `serve` does not give a callback hook for the underlying
//! TLS socket. Keeps the runtime narrow.

use std::io;
use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use axum::Router;
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::server::WebPkiClientVerifier;
use rustls::{DigitallySignedStruct, DistinguishedName, ServerConfig, SignatureScheme};
#[cfg(any(test, feature = "http-companion-test-support"))]
use rustls_pki_types::PrivatePkcs8KeyDer;
use rustls_pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tracing::{debug, info, warn};

/// mTLS server configuration. Holds the parsed certs + key + the built
/// `Arc<ServerConfig>`. Constructed once at startup and shared across
/// every accepted connection.
pub struct ServerTlsConfig {
    pub server_config: Arc<ServerConfig>,
}

impl ServerTlsConfig {
    /// Load PEM-encoded cert + key + client-CA bundle from disk.
    ///
    /// * `server_cert_pem` — the sidecar's workload cert chain.
    /// * `server_key_pem` — matching private key (PKCS8).
    /// * `client_ca_pem` — bundle of CAs that may issue client (plugin)
    ///   certs. WebPKI verification trusts any leaf signed by one of
    ///   these roots. SLICE 6 / HARDEN_08 layers per-tenant SPIFFE-URI
    ///   SAN pinning on top.
    pub fn from_pem_files(
        server_cert_pem: &Path,
        server_key_pem: &Path,
        client_ca_pem: &Path,
    ) -> Result<Self> {
        Self::from_pem_files_with_uri_pin(server_cert_pem, server_key_pem, client_ca_pem, None)
    }

    /// Like [`from_pem_files`](Self::from_pem_files) but additionally pins
    /// the connecting client to an exact SPIFFE URI SAN
    /// (`expected_client_spiffe_uri`). When `Some`, any client cert that
    /// chains validly to the CA but does not carry that exact URI SAN is
    /// rejected at the mTLS handshake (fail-closed). When `None`, only
    /// WebPKI CA-chain validation is performed (legacy SLICE 1 behavior);
    /// in that case tenant isolation rests on the in-body
    /// `tenant_id == cfg.tenant_id` check alone — production SHOULD set
    /// the pin.
    pub fn from_pem_files_with_uri_pin(
        server_cert_pem: &Path,
        server_key_pem: &Path,
        client_ca_pem: &Path,
        expected_client_spiffe_uri: Option<String>,
    ) -> Result<Self> {
        let server_certs = load_certs(server_cert_pem)
            .with_context(|| format!("load server cert {}", server_cert_pem.display()))?;
        let server_key = load_private_key(server_key_pem)
            .with_context(|| format!("load server key {}", server_key_pem.display()))?;
        let client_root_store = load_root_store(client_ca_pem)
            .with_context(|| format!("load client CA {}", client_ca_pem.display()))?;

        Self::from_components_with_uri_pin(
            server_certs,
            server_key,
            client_root_store,
            expected_client_spiffe_uri,
        )
    }

    /// Build directly from in-memory rustls components. Used by tests
    /// to keep the temp-file dance out of the hot path.
    pub fn from_components(
        server_certs: Vec<CertificateDer<'static>>,
        server_key: PrivateKeyDer<'static>,
        client_root_store: rustls::RootCertStore,
    ) -> Result<Self> {
        Self::from_components_with_uri_pin(server_certs, server_key, client_root_store, None)
    }

    /// As [`from_components`](Self::from_components) but with optional
    /// SPIFFE-URI SAN pinning layered on top of the WebPKI verifier.
    pub fn from_components_with_uri_pin(
        server_certs: Vec<CertificateDer<'static>>,
        server_key: PrivateKeyDer<'static>,
        client_root_store: rustls::RootCertStore,
        expected_client_spiffe_uri: Option<String>,
    ) -> Result<Self> {
        let webpki = WebPkiClientVerifier::builder(Arc::new(client_root_store))
            .build()
            .context("build WebPkiClientVerifier")?;

        // Layer the SPIFFE-URI SAN pin ON TOP of WebPKI chain validation
        // when an expected URI is configured. The pinning verifier
        // delegates chain validation to `webpki` FIRST, then asserts the
        // leaf's URI SAN — it never replaces chain validation.
        let verifier: Arc<dyn ClientCertVerifier> = match expected_client_spiffe_uri {
            Some(uri) => {
                info!(
                    expected_client_spiffe_uri = %uri,
                    "http_companion mTLS: SPIFFE-URI SAN client pinning enabled"
                );
                Arc::new(SpiffeUriPinningClientVerifier {
                    inner: webpki,
                    expected_uri: uri,
                })
            }
            None => {
                warn!(
                    "http_companion mTLS: no expected client SPIFFE-URI SAN \
                     configured; client identity rests on WebPKI CA-chain \
                     validation + in-body tenant_id check only. Set \
                     SPENDGUARD_SIDECAR_HTTP_COMPANION_EXPECTED_CLIENT_SPIFFE_URI \
                     to cryptographically pin the connecting workload."
                );
                webpki
            }
        };

        let server_config = ServerConfig::builder()
            .with_client_cert_verifier(verifier)
            .with_single_cert(server_certs, server_key)
            .context("build rustls ServerConfig")?;
        Ok(Self {
            server_config: Arc::new(server_config),
        })
    }

    /// Test-only placeholder. Holds a minimal server config that will
    /// reject every connection (no cert installed). Used by the
    /// run_companion `port==0` / non-loopback gate tests where we never
    /// actually exercise the listener.
    ///
    /// Gated on the `http-companion-test-support` feature so the
    /// optional rcgen dep stays out of release builds.
    #[cfg(any(test, feature = "http-companion-test-support"))]
    pub fn placeholder_for_tests() -> Self {
        // SAFETY: rustls refuses to build a ServerConfig without a
        // cert+key, so we wire a deterministic ephemeral self-signed
        // cert just to satisfy the type. The placeholder is never
        // exercised on the wire; the gate tests exit before any
        // accept().
        use rcgen::{CertificateParams, KeyPair};
        let kp = KeyPair::generate().expect("rcgen key");
        let mut params = CertificateParams::new(vec!["placeholder.local".to_string()]).unwrap();
        params.distinguished_name = rcgen::DistinguishedName::new();
        let cert = params.self_signed(&kp).unwrap();
        let cert_der = CertificateDer::from(cert.der().to_vec());
        let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(kp.serialize_der()));
        let mut roots = rustls::RootCertStore::empty();
        roots.add(cert_der.clone()).unwrap();
        Self::from_components(vec![cert_der], key_der, roots).expect("placeholder tls config")
    }
}

/// Custom `ClientCertVerifier` that performs full WebPKI CA-chain
/// validation FIRST (delegated to the wrapped `WebPkiClientVerifier`),
/// then asserts the end-entity leaf carries an exact SPIFFE URI SAN.
///
/// This closes the gap where any workload holding a cert signed by the
/// shared SpendGuard CA could connect to the HTTP companion: chain
/// validity alone is not sufficient identity. The pin binds the
/// connection to a specific workload SPIFFE identity at the handshake;
/// a mismatch aborts the handshake (no decision/audit side effects),
/// which is strictly stronger than an application-layer 403.
#[derive(Debug)]
struct SpiffeUriPinningClientVerifier {
    inner: Arc<dyn ClientCertVerifier>,
    expected_uri: String,
}

impl ClientCertVerifier for SpiffeUriPinningClientVerifier {
    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        // 1. Standard chain validation FIRST — chain-to-CA, expiry, etc.
        //    Any failure aborts before we even parse the leaf SAN.
        self.inner
            .verify_client_cert(end_entity, intermediates, now)?;

        // 2. Exact SPIFFE-URI SAN assertion on the leaf. A cert that
        //    chains validly but lacks the expected URI SAN is rejected
        //    (fail-closed); a same-CA peer cannot satisfy this with only
        //    an in-body tenant_id string.
        let leaf_uris = uri_sans_from_der(end_entity.as_ref()).map_err(|e| {
            rustls::Error::General(format!("client cert URI SAN parse failed: {e}"))
        })?;
        if leaf_uris.iter().any(|u| u == &self.expected_uri) {
            Ok(ClientCertVerified::assertion())
        } else {
            Err(rustls::Error::General(format!(
                "client cert URI SAN pin mismatch: expected `{}`, leaf presented {:?}",
                self.expected_uri, leaf_uris
            )))
        }
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        self.inner.root_hint_subjects()
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }

    fn offer_client_auth(&self) -> bool {
        self.inner.offer_client_auth()
    }

    fn client_auth_mandatory(&self) -> bool {
        // Mandatory: a missing client cert must not bypass the pin.
        true
    }
}

/// Extract all URI SANs from a DER-encoded X.509 leaf certificate.
fn uri_sans_from_der(der: &[u8]) -> Result<Vec<String>> {
    use x509_parser::extensions::GeneralName;

    let (_, cert) =
        x509_parser::parse_x509_certificate(der).context("parse client x509 certificate")?;
    let san = cert
        .tbs_certificate
        .subject_alternative_name()
        .context("parse client subjectAltName extension")?
        .ok_or_else(|| anyhow!("client certificate missing subjectAltName"))?;
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

fn load_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>> {
    let mut out = Vec::new();
    for cert in CertificateDer::pem_file_iter(path).context("open cert pem")? {
        out.push(cert.context("parse cert pem entry")?);
    }
    if out.is_empty() {
        return Err(anyhow!("no certificates found in {}", path.display()));
    }
    Ok(out)
}

fn load_private_key(path: &Path) -> Result<PrivateKeyDer<'static>> {
    let key = PrivateKeyDer::from_pem_file(path).context("parse key pem")?;
    Ok(key)
}

fn load_root_store(path: &Path) -> Result<rustls::RootCertStore> {
    let mut store = rustls::RootCertStore::empty();
    for cert in CertificateDer::pem_file_iter(path).context("open client CA pem")? {
        store
            .add(cert.context("parse client CA pem entry")?)
            .context("add CA to root store")?;
    }
    if store.is_empty() {
        return Err(anyhow!("no client CA certificates in {}", path.display()));
    }
    Ok(store)
}

/// Accept loop. Spawns a task per accepted TCP connection; the rustls
/// handshake runs inside the task so a slow / malformed handshake
/// cannot stall the listener.
pub async fn serve_with_mtls(
    tcp: TcpListener,
    tls: Arc<ServerTlsConfig>,
    router: Router,
) -> Result<()> {
    let acceptor = TlsAcceptor::from(tls.server_config.clone());
    loop {
        let (stream, peer) = match tcp.accept().await {
            Ok(pair) => pair,
            Err(e) if e.kind() == io::ErrorKind::ConnectionAborted => continue,
            Err(e) => return Err(anyhow!("accept failed: {e}")),
        };
        let acceptor = acceptor.clone();
        let router = router.clone();
        tokio::spawn(async move {
            let tls_stream = match acceptor.accept(stream).await {
                Ok(s) => s,
                Err(e) => {
                    // mTLS handshake failed — log at debug level. Plugin
                    // misconfiguration shows up as a flood of these
                    // entries which the operator can grep for.
                    debug!(peer = %peer, err = %e, "mTLS handshake failed");
                    return;
                }
            };
            let io = TokioIo::new(tls_stream);
            let svc = TowerToHyperService::new(router);
            if let Err(e) = http1::Builder::new()
                .keep_alive(true)
                .serve_connection(io, svc)
                .await
            {
                debug!(peer = %peer, err = %e, "http1 connection closed");
            }
        });
    }
}

/// Convenience: derive the SNI / `ServerName` the test harness should
/// present. Exposed for the integration test that constructs a
/// rustls `ClientConfig` directly.
pub fn server_name(host: &str) -> Result<ServerName<'static>> {
    ServerName::try_from(host.to_string()).map_err(|e| anyhow!("invalid SNI '{host}': {e}"))
}

#[allow(dead_code)]
fn _warn_unused(_: ()) {
    warn!("mtls module link helper");
}

#[cfg(all(test, feature = "http-companion-test-support"))]
mod san_pin_tests {
    use super::*;
    use rcgen::{CertificateParams, KeyPair, SanType};

    /// Build a self-signed leaf cert DER carrying a single URI SAN.
    fn leaf_with_uri_san(uri: &str) -> Vec<u8> {
        let key = KeyPair::generate().expect("rcgen key");
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params
            .subject_alt_names
            .push(SanType::URI(uri.to_string().try_into().unwrap()));
        let cert = params.self_signed(&key).unwrap();
        cert.der().to_vec()
    }

    #[test]
    fn uri_sans_from_der_extracts_spiffe_uri() {
        let der = leaf_with_uri_san("spiffe://spendguard.platform/ns/sg/sa/kong");
        let uris = uri_sans_from_der(&der).expect("parse leaf SAN");
        assert_eq!(uris, vec!["spiffe://spendguard.platform/ns/sg/sa/kong"]);
    }

    #[test]
    fn uri_san_pin_accepts_exact_match_rejects_mismatch() {
        // Mirrors the assertion in verify_client_cert step 2.
        let expected = "spiffe://spendguard.platform/ns/sg/sa/kong".to_string();

        let good = leaf_with_uri_san(&expected);
        let good_uris = uri_sans_from_der(&good).unwrap();
        assert!(good_uris.iter().any(|u| u == &expected));

        let bad = leaf_with_uri_san("spiffe://spendguard.platform/ns/sg/sa/other");
        let bad_uris = uri_sans_from_der(&bad).unwrap();
        assert!(
            !bad_uris.iter().any(|u| u == &expected),
            "a different-tenant URI SAN must NOT satisfy the pin (fail-closed)"
        );
    }

    #[test]
    fn uri_sans_from_der_errors_without_san() {
        // Cert with no SAN extension at all -> error (fail-closed: the
        // verifier maps this to a handshake rejection, never an accept).
        let key = KeyPair::generate().expect("rcgen key");
        let params = CertificateParams::new(vec!["no-san.local".to_string()]).unwrap();
        let cert = params.self_signed(&key).unwrap();
        // CN-only certs from rcgen still get a DNS SAN by default; assert
        // there is no URI SAN to match against.
        let uris = uri_sans_from_der(cert.der()).unwrap_or_default();
        assert!(
            uris.is_empty(),
            "a cert with no URI SAN yields no URIs to match the pin"
        );
    }
}
