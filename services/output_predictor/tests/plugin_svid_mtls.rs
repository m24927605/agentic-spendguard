//! HARDEN_08 per-tenant SVID contract tests.

use spendguard_output_predictor::plugin_svid::{
    paths_for_client_cert_id, subject_uri_for_tenant, tenant_from_subject_uri,
    validate_client_cert_id,
};
use uuid::Uuid;

#[test]
fn per_tenant_svid_subject_is_exact_spiffe_uri() {
    let tenant = Uuid::parse_str("018fcf9a-3d2d-7b37-9f21-0f27de0b20c1").unwrap();
    let subject = subject_uri_for_tenant(&tenant);
    assert_eq!(
        subject,
        "spiffe://spendguard.platform/predictor-client/018fcf9a-3d2d-7b37-9f21-0f27de0b20c1"
    );
    assert_eq!(tenant_from_subject_uri(&subject).unwrap(), tenant);
}

#[test]
fn client_cert_id_maps_to_bounded_subdirectory() {
    let base = std::path::Path::new("/etc/spendguard/plugin-client-svid");
    let paths = paths_for_client_cert_id(base, "tenant-a_01").unwrap();
    assert_eq!(
        paths.cert_pem,
        std::path::Path::new("/etc/spendguard/plugin-client-svid/tenant-a_01/tls.crt")
    );
    assert_eq!(
        paths.key_pem,
        std::path::Path::new("/etc/spendguard/plugin-client-svid/tenant-a_01/tls.key")
    );
    assert_eq!(
        paths.trust_ca_pem,
        std::path::Path::new("/etc/spendguard/plugin-client-svid/tenant-a_01/ca.crt")
    );
}

#[test]
fn client_cert_id_path_traversal_fails_closed() {
    for bad in ["../tenant", "tenant/one", "tenant.one", "tenant one", ".."] {
        assert!(
            validate_client_cert_id(bad).is_err(),
            "unsafe client_cert_id must be rejected: {bad}"
        );
    }
}
