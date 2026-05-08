//! mTLS channel configuration shared by Ledger + Canonical Ingest clients.
//!
//! POC: cert-manager external issuer (per Stage 2 §12.1) populates
//! /var/run/secrets/spendguard/{tls.crt,tls.key,ca.crt} on the sidecar pod.
//! On startup we load these into a tonic ClientTlsConfig.

use anyhow::{Context, Result};
use tonic::transport::{Certificate, ClientTlsConfig, Identity};

#[derive(Debug, Clone)]
pub struct MTlsPaths {
    pub workload_cert_pem: String, // path
    pub workload_key_pem: String,  // path
    pub trust_ca_pem: String,      // path or inline content
    pub trust_ca_inline: bool,
}

impl Default for MTlsPaths {
    fn default() -> Self {
        Self {
            workload_cert_pem: "/var/run/secrets/spendguard/tls.crt".into(),
            workload_key_pem: "/var/run/secrets/spendguard/tls.key".into(),
            trust_ca_pem: "/var/run/secrets/spendguard/ca.crt".into(),
            trust_ca_inline: false,
        }
    }
}

/// Build a tonic TLS config from on-disk PEM files. The workload cert is
/// short-lived (24h default per Stage 2 §12.1) and re-issued by cert-manager
/// before expiry; tonic re-reads via fresh Channel construction on each
/// reconnect.
pub fn build_client_tls(
    paths: &MTlsPaths,
    sni_domain: &str,
) -> Result<ClientTlsConfig> {
    let cert_pem = std::fs::read_to_string(&paths.workload_cert_pem)
        .with_context(|| format!("read workload cert {}", paths.workload_cert_pem))?;
    let key_pem = std::fs::read_to_string(&paths.workload_key_pem)
        .with_context(|| format!("read workload key {}", paths.workload_key_pem))?;
    let ca_pem = if paths.trust_ca_inline {
        paths.trust_ca_pem.clone()
    } else {
        std::fs::read_to_string(&paths.trust_ca_pem)
            .with_context(|| format!("read trust CA {}", paths.trust_ca_pem))?
    };

    let identity = Identity::from_pem(cert_pem, key_pem);
    let ca = Certificate::from_pem(ca_pem);

    Ok(ClientTlsConfig::new()
        .identity(identity)
        .ca_certificate(ca)
        .domain_name(sni_domain.to_string()))
}
