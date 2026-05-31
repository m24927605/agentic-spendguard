//! Per-tenant predictor-client SVID helpers for Strategy C plugin mTLS.
//!
//! HARDEN_08 closes the SLICE_07 deferral: SpendGuard presents a
//! tenant-bound client certificate to customer plugins. The SVID URI SAN
//! is the tenant binding authority; request metadata is only defense in
//! depth.

use std::path::{Path, PathBuf};

use anyhow::Context;
use sha2::Digest;
use uuid::Uuid;
use x509_parser::extensions::GeneralName;
use x509_parser::prelude::*;

pub const PREDICTOR_CLIENT_SVID_PREFIX: &str = "spiffe://spendguard.platform/predictor-client/";

#[derive(Debug, Clone)]
pub struct TenantSvidPaths {
    pub cert_pem: PathBuf,
    pub key_pem: PathBuf,
    pub trust_ca_pem: PathBuf,
}

#[derive(Debug, Clone)]
pub struct TenantSvidMaterials {
    pub cert_pem: Vec<u8>,
    pub key_pem: Vec<u8>,
    pub ca_pem: Vec<u8>,
    pub subject_uri: String,
    pub fingerprint_hex: String,
}

pub fn subject_uri_for_tenant(tenant: &Uuid) -> String {
    format!("{PREDICTOR_CLIENT_SVID_PREFIX}{tenant}")
}

pub fn tenant_from_subject_uri(uri: &str) -> Result<Uuid, anyhow::Error> {
    let suffix = uri
        .strip_prefix(PREDICTOR_CLIENT_SVID_PREFIX)
        .ok_or_else(|| anyhow::anyhow!("SVID URI has wrong prefix: `{uri}`"))?;
    Uuid::parse_str(suffix).with_context(|| format!("SVID URI tenant is not a UUID: `{uri}`"))
}

pub fn validate_client_cert_id(client_cert_id: &str) -> Result<(), anyhow::Error> {
    if client_cert_id.is_empty() {
        anyhow::bail!("client_cert_id is empty");
    }
    if client_cert_id == "." || client_cert_id == ".." {
        anyhow::bail!("client_cert_id `{client_cert_id}` is path traversal");
    }
    if client_cert_id
        .bytes()
        .any(|b| !(b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')))
    {
        anyhow::bail!(
            "client_cert_id `{client_cert_id}` contains unsupported characters; allowed: [A-Za-z0-9_-]"
        );
    }
    Ok(())
}

pub fn paths_for_client_cert_id(
    svid_dir: &Path,
    client_cert_id: &str,
) -> Result<TenantSvidPaths, anyhow::Error> {
    validate_client_cert_id(client_cert_id)?;
    let dir = svid_dir.join(client_cert_id);
    Ok(TenantSvidPaths {
        cert_pem: dir.join("tls.crt"),
        key_pem: dir.join("tls.key"),
        trust_ca_pem: dir.join("ca.crt"),
    })
}

pub fn load_tenant_svid_materials(
    svid_dir: &Path,
    client_cert_id: &str,
    tenant: &Uuid,
) -> Result<TenantSvidMaterials, anyhow::Error> {
    let paths = paths_for_client_cert_id(svid_dir, client_cert_id)?;
    let cert_pem = std::fs::read(&paths.cert_pem)
        .with_context(|| format!("read tenant SVID cert {}", paths.cert_pem.display()))?;
    let key_pem = std::fs::read(&paths.key_pem)
        .with_context(|| format!("read tenant SVID key {}", paths.key_pem.display()))?;
    let ca_pem = std::fs::read(&paths.trust_ca_pem).with_context(|| {
        format!(
            "read tenant plugin trust CA {}",
            paths.trust_ca_pem.display()
        )
    })?;

    let expected = subject_uri_for_tenant(tenant);
    let subject_uri = extract_spiffe_uri_from_cert_pem(&cert_pem)?;
    if subject_uri != expected {
        anyhow::bail!(
            "tenant SVID subject mismatch for client_cert_id `{client_cert_id}`: expected `{expected}`, got `{subject_uri}`"
        );
    }

    let mut hasher = sha2::Sha256::new();
    hasher.update(&cert_pem);
    hasher.update(&key_pem);
    hasher.update(&ca_pem);
    Ok(TenantSvidMaterials {
        cert_pem,
        key_pem,
        ca_pem,
        subject_uri,
        fingerprint_hex: hex::encode(hasher.finalize()),
    })
}

pub fn extract_spiffe_uri_from_cert_pem(cert_pem: &[u8]) -> Result<String, anyhow::Error> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(cert_pem).context("parse SVID PEM block")?;
    let (_, cert) = parse_x509_certificate(&pem.contents).context("parse SVID x509 certificate")?;
    let san = cert
        .tbs_certificate
        .subject_alternative_name()
        .context("parse SVID subjectAltName extension")?
        .ok_or_else(|| anyhow::anyhow!("SVID certificate missing subjectAltName"))?;

    let uris: Vec<&str> = san
        .value
        .general_names
        .iter()
        .filter_map(|name| match name {
            GeneralName::URI(uri) => Some(*uri),
            _ => None,
        })
        .collect();

    let matching: Vec<&str> = uris
        .into_iter()
        .filter(|uri| uri.starts_with(PREDICTOR_CLIENT_SVID_PREFIX))
        .collect();
    match matching.as_slice() {
        [uri] => Ok((*uri).to_string()),
        [] => anyhow::bail!(
            "SVID certificate has no URI SAN with `{PREDICTOR_CLIENT_SVID_PREFIX}` prefix"
        ),
        _ => anyhow::bail!("SVID certificate has multiple SpendGuard predictor-client URI SANs"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_uri_round_trips_uuid() {
        let tenant = Uuid::parse_str("018fcf9a-3d2d-7b37-9f21-0f27de0b20c1").unwrap();
        let uri = subject_uri_for_tenant(&tenant);
        assert_eq!(
            uri,
            "spiffe://spendguard.platform/predictor-client/018fcf9a-3d2d-7b37-9f21-0f27de0b20c1"
        );
        assert_eq!(tenant_from_subject_uri(&uri).unwrap(), tenant);
    }

    #[test]
    fn tenant_from_subject_uri_rejects_wrong_prefix() {
        let err = tenant_from_subject_uri("spiffe://other/predictor-client/not-a-uuid")
            .expect_err("wrong prefix rejected");
        assert!(format!("{err:#}").contains("wrong prefix"));
    }

    #[test]
    fn tenant_from_subject_uri_rejects_non_uuid() {
        let err =
            tenant_from_subject_uri("spiffe://spendguard.platform/predictor-client/not-a-uuid")
                .expect_err("non uuid rejected");
        assert!(format!("{err:#}").contains("not a UUID"));
    }

    #[test]
    fn client_cert_id_blocks_path_traversal() {
        for bad in [
            "",
            ".",
            "..",
            "../tenant-a",
            "tenant/a",
            "tenant.a",
            "tenant a",
        ] {
            assert!(
                validate_client_cert_id(bad).is_err(),
                "bad client_cert_id should fail: {bad}"
            );
        }
        for good in ["tenant-a", "tenant_a", "tenantA01"] {
            validate_client_cert_id(good).expect("safe client_cert_id accepted");
        }
    }
}
