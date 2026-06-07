//! Test-only helpers: ephemeral CA + workload + client cert generator.
//!
//! Lives inside `src/` (gated on `#[cfg(test)]`) so both the unit
//! tests in `mod tests` blocks and the integration tests under
//! `services/sidecar/tests/` can call it without duplicating the
//! rcgen boilerplate. SLICE 6 uses real cert-manager-issued certs;
//! this helper exists solely for SLICE 1 wire-handshake gates.

use std::sync::Arc;

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, KeyUsagePurpose,
};
use rustls::RootCertStore;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

use super::mtls::ServerTlsConfig;

/// Generated test PKI suitable for one mTLS handshake. CA root signs
/// both the server (sidecar) cert and the client (Kong plugin) cert.
pub struct TestPki {
    pub ca_cert_pem: String,
    pub server_cert_chain_pem: String,
    pub server_key_pem: String,
    pub client_cert_pem: String,
    pub client_key_pem: String,
    /// `(server_certs, server_key, root_store)` ready to feed
    /// [`ServerTlsConfig::from_components`] directly.
    pub server_config: Arc<ServerTlsConfig>,
}

/// Build a fresh CA + server cert (CN `localhost`, SAN `127.0.0.1`) +
/// client cert (SAN URI `spiffe://example.org/ns/sg/sa/kong`).
pub fn ephemeral_pki() -> TestPki {
    // 1) CA root
    let ca_key = KeyPair::generate().expect("rcgen ca key");
    let mut ca_params = CertificateParams::new(vec!["sg-test-ca".to_string()]).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.distinguished_name = {
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "spendguard-test-ca");
        dn
    };
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();
    let ca_cert_pem = ca_cert.pem();

    // 2) Server cert — signed by CA, SAN `127.0.0.1` + `localhost`
    let server_key = KeyPair::generate().expect("rcgen server key");
    let mut server_params =
        CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()]).unwrap();
    server_params.distinguished_name = {
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "spendguard-http-companion");
        dn
    };
    let server_cert = server_params
        .signed_by(&server_key, &ca_cert, &ca_key)
        .unwrap();
    let server_cert_pem = server_cert.pem();
    let server_chain_pem = format!("{server_cert_pem}{ca_cert_pem}");
    let server_key_pem = server_key.serialize_pem();

    // 3) Client cert — signed by CA, SAN URI for SVID-style SAN.
    let client_key = KeyPair::generate().expect("rcgen client key");
    let mut client_params =
        CertificateParams::new(vec!["spendguard-kong-plugin".to_string()]).unwrap();
    client_params.distinguished_name = {
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "spendguard-kong-plugin");
        dn
    };
    let client_cert = client_params
        .signed_by(&client_key, &ca_cert, &ca_key)
        .unwrap();
    let client_cert_pem = client_cert.pem();
    let client_key_pem = client_key.serialize_pem();

    // 4) Build the rustls ServerConfig the listener will use.
    let server_certs_der: Vec<CertificateDer<'static>> = vec![
        CertificateDer::from(server_cert.der().to_vec()),
        CertificateDer::from(ca_cert.der().to_vec()),
    ];
    let server_key_der: PrivateKeyDer<'static> =
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(server_key.serialize_der()));

    let mut root_store = RootCertStore::empty();
    root_store
        .add(CertificateDer::from(ca_cert.der().to_vec()))
        .unwrap();

    let tls_config =
        ServerTlsConfig::from_components(server_certs_der, server_key_der, root_store).unwrap();

    TestPki {
        ca_cert_pem,
        server_cert_chain_pem: server_chain_pem,
        server_key_pem,
        client_cert_pem,
        client_key_pem,
        server_config: Arc::new(tls_config),
    }
}
