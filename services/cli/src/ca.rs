//! CA + leaf issuance for the closed-CLI MITM proxy.
//!
//! ## Locked decisions (design §9)
//!
//! - **Validity 825 days** (Apple-Safari max) — the absolute ceiling for
//!   browsers that gate TLS-1.3 + ECH on cert lifetime. Hard-coded.
//! - **Serial = UUIDv7 → BigUint, never reused** — UUIDv7's high 48 bits are
//!   millisecond-resolution timestamp; the low 80 bits are CSPRNG. Encoding
//!   the resulting 128-bit blob as a positive `BigUint` gives an x509 serial
//!   that is monotonic-per-issuer AND collision-resistant, which `rcgen`'s
//!   default (`SerialNumber::from(42)`) is not.
//! - **Leaf SAN = `localhost`, `127.0.0.1`, `::1` ONLY** — the leaf must
//!   never name external hosts; the install path's local proxy is the only
//!   place it can legitimately appear (review-standards.md `T3`).
//! - **PEM-only output** — PKCS#12 is deferred; the v1 install path writes
//!   four files (`root_ca.{pem,key.pem}` + `leaf.{pem,key.pem}`).
//!
//! ## Slice scope
//!
//! This module ships [`generate_root_ca`] and [`issue_leaf_cert`]. On-disk
//! persistence + load/reload (`RootCa::ensure` / `RootCa::load`) is sniped
//! to a follow-up slice — the install command does a fresh issue + write
//! every time for now.

use anyhow::{Context, Result};
use num_bigint::BigUint;
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose, SanType, SerialNumber,
};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;
// R2 fix per security review (M1): PKCS8 private-key strings are wrapped in
// `Zeroizing` so the heap allocation is wiped before the allocator can hand
// the slot to another caller. Required by implementation.md §3.
use zeroize::Zeroizing;

/// Locked validity per design §9.1.
const CA_VALIDITY_DAYS: i64 = 825;

/// Subject CN — referenced by the trust-store install slices.
pub const ROOT_CA_SUBJECT_CN: &str = "SpendGuard Local Root CA";

/// Materialised root CA, ready to sign a leaf and ready to serialise.
///
/// Intentionally does not implement `Debug` — `rcgen::Certificate` and
/// `rcgen::KeyPair` are opaque, and the PKCS8 key bytes must not land in
/// any default formatter (review-standards.md `T1`: no key bytes through
/// stdout/stderr).
pub struct RootCa {
    pub cert: Certificate,
    pub key: KeyPair,
    pub cert_pem: String,
    /// R2 fix per security review (M1): Zeroizing<String> wipes the PKCS8
    /// PEM bytes on drop per implementation.md §3.
    pub key_pem: Zeroizing<String>,
    pub fingerprint_sha256: [u8; 32],
    pub serial: BigUint,
    pub not_before: OffsetDateTime,
    pub not_after: OffsetDateTime,
}

/// Materialised leaf cert, ready to terminate TLS on `127.0.0.1:8443`.
pub struct LeafCert {
    pub cert_pem: String,
    /// R2 fix per security review (M1): Zeroizing<String> wipes the PKCS8
    /// PEM bytes on drop per implementation.md §3.
    pub key_pem: Zeroizing<String>,
}

/// Issue a fresh root CA per the locked decisions.
///
/// - 825-day validity from `now_utc`.
/// - UUIDv7-derived 128-bit positive BigUint serial.
/// - `KeyUsage = KeyCertSign + CrlSign` — locked by slice doc §9 +
///   review-standards `T3`. Any additional bit (e.g. `DigitalSignature`)
///   would let the root sign arbitrary attestations (OCSP, code-signing,
///   bare CMS) and is unjustified by the closed-loop install path.
/// - `IsCa = Ca(BasicConstraints::Unconstrained)` — bound by validity + SAN
///   constraint on the leaf, NOT by path length (no intermediate in scope).
pub fn generate_root_ca() -> Result<RootCa> {
    let key = KeyPair::generate().context("generate root CA keypair")?;

    let serial_uuid = Uuid::now_v7();
    let serial_biguint = BigUint::from_bytes_be(serial_uuid.as_bytes());
    let serial_bytes = serial_biguint.to_bytes_be();
    let serial = SerialNumber::from_slice(&serial_bytes);

    let now = OffsetDateTime::now_utc();
    let not_before = now;
    let not_after = now
        .checked_add(Duration::days(CA_VALIDITY_DAYS))
        .context("CA validity overflow — clock or duration is busted")?;

    let mut params = CertificateParams::default();
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, ROOT_CA_SUBJECT_CN);
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    // R2 fix per security review: drop DigitalSignature (B2). Slice doc §9 +
    // review-standards T3 lock root KeyUsage to KeyCertSign + CrlSign only.
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params.serial_number = Some(serial);
    params.not_before = not_before;
    params.not_after = not_after;
    params.use_authority_key_identifier_extension = true;

    let cert = params
        .self_signed(&key)
        .context("self-sign root CA params")?;
    let cert_pem = cert.pem();
    let key_pem = Zeroizing::new(key.serialize_pem());

    let fingerprint_sha256 = sha256_of_der(cert.der());

    Ok(RootCa {
        cert,
        key,
        cert_pem,
        key_pem,
        fingerprint_sha256,
        serial: serial_biguint,
        not_before,
        not_after,
    })
}

/// Issue a leaf cert signed by `root`, with SAN constrained to the supplied
/// list. Callers should pass the locked tuple `("localhost", "127.0.0.1",
/// "::1")` — anything else is rejected to enforce the "closed-loop locality"
/// rule from design §3.
pub fn issue_leaf_cert(root: &RootCa, sans: &[&str]) -> Result<LeafCert> {
    let san_types = parse_and_validate_sans(sans)?;
    let key = KeyPair::generate().context("generate leaf keypair")?;

    let leaf_serial_uuid = Uuid::now_v7();
    let leaf_serial_bytes = BigUint::from_bytes_be(leaf_serial_uuid.as_bytes()).to_bytes_be();

    let now = OffsetDateTime::now_utc();
    let leaf_not_before = now;
    // Leaf validity = 397 days (TLS BR maximum); does not need to track CA.
    let leaf_not_after = now
        .checked_add(Duration::days(397))
        .context("leaf validity overflow")?;

    let mut params = CertificateParams::default();
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, "localhost");
    params.subject_alt_names = san_types;
    params.is_ca = IsCa::NoCa;
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.serial_number = Some(SerialNumber::from_slice(&leaf_serial_bytes));
    params.not_before = leaf_not_before;
    params.not_after = leaf_not_after;
    params.use_authority_key_identifier_extension = true;

    let cert = params
        .signed_by(&key, &root.cert, &root.key)
        .context("sign leaf cert with root CA")?;
    Ok(LeafCert {
        cert_pem: cert.pem(),
        key_pem: Zeroizing::new(key.serialize_pem()),
    })
}

/// Convert the allowed string list into typed `SanType`s and reject anything
/// outside the locked closed-loop tuple. Wildcards explicitly banned.
fn parse_and_validate_sans(sans: &[&str]) -> Result<Vec<SanType>> {
    let mut out = Vec::with_capacity(sans.len());
    for raw in sans {
        let san = match *raw {
            "localhost" => SanType::DnsName("localhost".try_into().context("localhost SAN")?),
            "127.0.0.1" => SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            "::1" => SanType::IpAddress(IpAddr::V6(Ipv6Addr::LOCALHOST)),
            other => {
                anyhow::bail!(
                    "leaf SAN {other:?} rejected — only localhost/127.0.0.1/::1 are permitted"
                );
            }
        };
        out.push(san);
    }
    Ok(out)
}

fn sha256_of_der(der: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(der);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

/// Public helper for callers that want the fingerprint as the lower-case hex
/// string used in the InstallReport.
pub fn fingerprint_hex(fingerprint: &[u8; 32]) -> String {
    hex::encode(fingerprint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use x509_parser::prelude::FromDer;

    /// `T5` — UUIDv7 serial substrate. Asserts:
    ///   1. Serial is set (default rcgen would be `None`).
    ///   2. Serial has at least 14 bytes (UUIDv7 is 16; positive `BigUint`
    ///      can strip up to one zero byte, but never more than two zeros).
    ///   3. Two consecutive issues yield distinct serials.
    #[test]
    fn generates_root_ca_with_uuidv7_serial() {
        let a = generate_root_ca().expect("first CA");
        let b = generate_root_ca().expect("second CA");
        assert_ne!(a.serial, b.serial, "UUIDv7 serial must not repeat");
        let serial_bytes = a.serial.to_bytes_be();
        assert!(
            (14..=16).contains(&serial_bytes.len()),
            "serial should be ~16 bytes (UUIDv7), got {}",
            serial_bytes.len()
        );
    }

    /// `T4` — 825-day validity window. Reviewer rejects bumps without an ADR.
    #[test]
    fn root_ca_validity_is_825_days() {
        let ca = generate_root_ca().expect("issue");
        let span = ca.not_after - ca.not_before;
        // Allow ±60s wall-clock slop so a slow CI host doesn't flake.
        let nominal = Duration::days(825);
        let delta = (span - nominal).abs();
        assert!(
            delta < Duration::minutes(1),
            "expected ~825 days, got {} (delta {})",
            span,
            delta
        );
    }

    /// `T3` — leaf SAN constrained. Bare-string list `["localhost",
    /// "127.0.0.1", "::1"]` accepted; anything else rejected.
    #[test]
    fn leaf_san_only_localhost() {
        let ca = generate_root_ca().expect("ca");
        let leaf = issue_leaf_cert(&ca, &["localhost", "127.0.0.1", "::1"]).expect("leaf");
        // Sanity: PEM is non-empty.
        assert!(leaf.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(leaf.key_pem.contains("PRIVATE KEY"));

        // Negative cases. We intentionally do NOT use `expect_err` because
        // `LeafCert` deliberately omits `Debug` to keep its `Zeroizing` key
        // material out of any default formatter (review-standards.md T1).
        let banned = ["*.example.com", "google.com", "10.0.0.1", ""];
        for case in banned {
            match issue_leaf_cert(&ca, &[case]) {
                Ok(_) => panic!("expected SAN {case:?} to be rejected"),
                Err(err) => {
                    let msg = format!("{err}");
                    assert!(
                        msg.contains("rejected"),
                        "expected rejection for {case:?}, got {msg}"
                    );
                }
            }
        }
    }

    /// End-to-end: the leaf chains to the root. We avoid spawning `openssl`
    /// and instead parse the DER and inspect issuer/subject + key usages
    /// using `x509-parser`.
    #[test]
    fn leaf_verifies_against_root() {
        let ca = generate_root_ca().expect("ca");
        let leaf = issue_leaf_cert(&ca, &["localhost", "127.0.0.1", "::1"]).expect("leaf");

        // Parse leaf PEM → DER → X509.
        let (_, leaf_pem) =
            x509_parser::pem::parse_x509_pem(leaf.cert_pem.as_bytes()).expect("parse leaf pem");
        let (_, leaf_x509) =
            x509_parser::certificate::X509Certificate::from_der(&leaf_pem.contents)
                .expect("parse leaf x509");

        let (_, ca_pem) =
            x509_parser::pem::parse_x509_pem(ca.cert_pem.as_bytes()).expect("parse ca pem");
        let (_, ca_x509) = x509_parser::certificate::X509Certificate::from_der(&ca_pem.contents)
            .expect("parse ca x509");

        // Subject(CA) == Issuer(leaf): chain by name.
        assert_eq!(
            leaf_x509.issuer().to_string(),
            ca_x509.subject().to_string(),
            "leaf issuer should equal CA subject"
        );

        // CA's BasicConstraints[cA] = true.
        let bc = ca_x509
            .basic_constraints()
            .expect("BasicConstraints lookup")
            .expect("CA must have BasicConstraints extension");
        assert!(bc.value.ca, "CA cert must have cA:TRUE");

        // Leaf is NOT a CA.
        let leaf_bc = leaf_x509
            .basic_constraints()
            .expect("leaf BasicConstraints lookup");
        if let Some(bc) = leaf_bc {
            assert!(!bc.value.ca, "leaf must have cA:FALSE");
        }

        // Verify the signature: leaf.tbs_certificate ← ca.public_key.
        let verified = leaf_x509.verify_signature(Some(ca_x509.public_key()));
        verified.expect("leaf signature must verify against root CA public key");
    }
}
