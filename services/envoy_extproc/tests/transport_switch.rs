//! SLICE 6 integration tests for the transport hard-switch.
//!
//! These smoke tests exercise the [`SidecarClient::connect_transport`]
//! dispatch path end-to-end without standing up a full mTLS cert
//! authority — production-deploy verification happens via the Helm
//! template `helm lint` + `helm template` gates plus the live mTLS
//! handshake exercised by the docker-compose demo in SLICE 7.
//!
//! Spec refs:
//!   - docs/specs/coverage/D01_envoy_extproc/design.md §3.3 (transport hard-switch)
//!   - docs/specs/coverage/D01_envoy_extproc/review-standards.md §7.1 (Blocker class)
//!   - docs/internal/slices/COV_06_envoy_extproc_helm.md

use std::path::PathBuf;
use std::time::Duration;

use spendguard_envoy_extproc::config::{Transport, SIDECAR_SVID_PREFIX};
use spendguard_envoy_extproc::sidecar_client::{SidecarClient, SidecarError};

/// SLICE 6 — `Transport::Tcp` with a missing SVID cert file errors at
/// the file-read step rather than panicking. Defense in depth so an
/// SVID Secret mount race at pod start surfaces as a typed transport
/// error → fail-closed exit, not a `panic!` that obscures the cause.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tcp_transport_missing_cert_errors_at_file_read() {
    let transport = Transport::Tcp {
        sidecar_url: "https://127.0.0.1:1".into(),
        client_cert_pem: PathBuf::from(format!(
            "/tmp/spendguard-extproc-svid-missing-{}/tls.crt",
            uuid::Uuid::new_v4().simple()
        )),
        client_key_pem: PathBuf::from("/tmp/spendguard-extproc-svid-missing/tls.key"),
        ca_bundle_pem: PathBuf::from("/tmp/spendguard-extproc-svid-missing/ca.crt"),
        expected_sidecar_svid_prefix: SIDECAR_SVID_PREFIX.into(),
    };
    let result = SidecarClient::connect_transport(
        &transport,
        "00000000-0000-4000-8000-000000000001",
        Duration::from_millis(50),
    )
    .await;
    let err = result.expect_err("missing cert must error");
    assert!(
        matches!(err, SidecarError::Transport { .. }),
        "got: {err:?}"
    );
}

/// SLICE 6 — Malformed sidecar URL surfaces typed error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tcp_transport_malformed_url_errors_typed() {
    let transport = Transport::Tcp {
        sidecar_url: "not://valid".into(),
        client_cert_pem: PathBuf::from("/dev/null"),
        client_key_pem: PathBuf::from("/dev/null"),
        ca_bundle_pem: PathBuf::from("/dev/null"),
        expected_sidecar_svid_prefix: SIDECAR_SVID_PREFIX.into(),
    };
    let result = SidecarClient::connect_transport(
        &transport,
        "00000000-0000-4000-8000-000000000001",
        Duration::from_millis(50),
    )
    .await;
    let err = result.expect_err("bad url must error");
    assert!(
        matches!(err, SidecarError::Transport { .. }),
        "got: {err:?}"
    );
}

/// SLICE 6 — UDS transport dispatch reaches the SLICE 1-5 connect path
/// when the binary is built with the `uds-dev` feature (default).
#[cfg(feature = "uds-dev")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn uds_transport_missing_socket_errors_typed() {
    let tmp = std::env::temp_dir().join(format!(
        "spendguard-extproc-slice6-missing-{}.sock",
        uuid::Uuid::new_v4().simple()
    ));
    let transport = Transport::Uds { socket_path: tmp };
    let result = SidecarClient::connect_transport(
        &transport,
        "00000000-0000-4000-8000-000000000001",
        Duration::from_millis(50),
    )
    .await;
    let err = result.expect_err("missing socket must error");
    assert!(
        matches!(err, SidecarError::Transport { .. }),
        "got: {err:?}"
    );
}
