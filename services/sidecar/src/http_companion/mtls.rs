//! mTLS server config + accept loop for the HTTP companion.
//!
//! Per `docs/specs/coverage/D09_kong_ai_gateway/review-standards.md`:
//!
//! * §1.4 — mTLS-only; loopback bind by default.
//! * §2.2 — uses rustls + a `ClientCertVerifier` (WebPKI in SLICE 1;
//!   SLICE 6 / HARDEN_08 layers per-tenant SPIFFE-URI SAN pinning on
//!   top of the same primitive). This SLICE 1 surface ships the
//!   shared `ServerTlsConfig` constructor so SLICE 6 only has to
//!   inject the pinned verifier; no copy-paste.
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
use rustls::server::WebPkiClientVerifier;
use rustls::ServerConfig;
#[cfg(any(test, feature = "http-companion-test-support"))]
use rustls_pki_types::PrivatePkcs8KeyDer;
use rustls_pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer, ServerName};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tracing::{debug, warn};

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
        let server_certs = load_certs(server_cert_pem)
            .with_context(|| format!("load server cert {}", server_cert_pem.display()))?;
        let server_key = load_private_key(server_key_pem)
            .with_context(|| format!("load server key {}", server_key_pem.display()))?;
        let client_root_store = load_root_store(client_ca_pem)
            .with_context(|| format!("load client CA {}", client_ca_pem.display()))?;

        Self::from_components(server_certs, server_key, client_root_store)
    }

    /// Build directly from in-memory rustls components. Used by tests
    /// to keep the temp-file dance out of the hot path.
    pub fn from_components(
        server_certs: Vec<CertificateDer<'static>>,
        server_key: PrivateKeyDer<'static>,
        client_root_store: rustls::RootCertStore,
    ) -> Result<Self> {
        let verifier = WebPkiClientVerifier::builder(Arc::new(client_root_store))
            .build()
            .context("build WebPkiClientVerifier")?;
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
