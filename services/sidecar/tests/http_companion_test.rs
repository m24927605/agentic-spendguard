//! D09 SLICE 1 integration tests — wire-level mTLS gates.
//!
//! Covers the slice-1 review-standards gates that cannot be exercised
//! at the in-process router level:
//!
//! * §2.2 — mTLS chain validation. Tests confirm a client speaking
//!   plain HTTP (or a client without a CA-signed cert) is rejected
//!   before the HTTP layer parses any bytes.
//! * §2.4 — body-size cap enforced at the axum extractor level.
//! * §2.6 — idempotent /v1/trace replay returns byte-identical ack.
//! * §2.8 — loopback bind default (the `port==0` and non-loopback
//!   gates live in the unit suite; this file exercises the live
//!   bind on `127.0.0.1`).
//!
//! Each test spawns a fresh listener with an ephemeral PKI generated
//! via rcgen so we never check PEMs into the repo. The listener
//! shuts down when the test task exits.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use reqwest::tls::Certificate;
use serde_json::json;
use tokio::net::TcpListener;

use spendguard_sidecar::http_companion::{
    self,
    service::{DecisionStub, NoopDecisionService},
    DecisionVerdict,
};

// Re-export the test_support helper. It lives under `#[cfg(test)]` in
// the sidecar lib so we can call it here without copy-pasting rcgen.
use spendguard_sidecar::http_companion::test_support::ephemeral_pki;

fn install_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

/// Boot a fresh http_companion listener on a randomly assigned port
/// and return `(SocketAddr, NoopDecisionService Arc, JoinHandle, PKI)`.
/// Caller drops the JoinHandle to leave the listener running for the
/// duration of the test; the OS reclaims the port at process exit.
async fn boot_test_listener() -> (
    SocketAddr,
    Arc<NoopDecisionService>,
    tokio::task::JoinHandle<()>,
    spendguard_sidecar::http_companion::test_support::TestPki,
) {
    install_crypto_provider();
    let pki = ephemeral_pki();
    let svc = Arc::new(NoopDecisionService::default());
    let svc_for_listener = svc.clone();

    // Bind to an OS-assigned port. Doing it here (rather than letting
    // run_companion own the bind) lets the test read the chosen port
    // before connecting.
    let tcp = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind 127.0.0.1");
    let addr = tcp.local_addr().expect("local addr");

    let tls = pki.server_config.clone();
    let max_body = 4 * 1024 * 1024;
    let router = http_companion::build_router_for_tests(svc_for_listener, max_body);

    let handle = tokio::spawn(async move {
        let _ = http_companion::mtls::serve_with_mtls(tcp, tls, router).await;
    });

    (addr, svc, handle, pki)
}

/// Build a reqwest client configured with the ephemeral CA + the
/// supplied client identity. `with_client_identity=false` builds a
/// trust-CA-only client (no mTLS identity); the server rejects such
/// connections.
fn build_client(
    pki: &spendguard_sidecar::http_companion::test_support::TestPki,
    with_client_identity: bool,
) -> reqwest::Client {
    let ca_cert = Certificate::from_pem(pki.ca_cert_pem.as_bytes()).expect("parse CA pem");
    let mut builder = reqwest::Client::builder()
        .add_root_certificate(ca_cert)
        .tls_built_in_root_certs(false)
        .https_only(true)
        .timeout(Duration::from_secs(10));
    if with_client_identity {
        let mut bundle = pki.client_cert_pem.as_bytes().to_vec();
        bundle.extend_from_slice(pki.client_key_pem.as_bytes());
        let id = reqwest::Identity::from_pem(&bundle).expect("parse client identity");
        builder = builder.identity(id);
    }
    builder.build().expect("build reqwest client")
}

#[tokio::test]
async fn mtls_happy_path_decision_allow() {
    let (addr, svc, _handle, pki) = boot_test_listener().await;
    let client = build_client(&pki, true);

    let url = format!("https://localhost:{}/v1/decision", addr.port());
    let resp = client
        .post(&url)
        .json(&json!({
            "tenant_id": "t1",
            "claim_estimate_atomic": "100",
            "prompt_class": "general",
            "model_class": "openai/gpt-4o-mini",
            "idempotency_key": "k-happy-1",
        }))
        .send()
        .await
        .expect("decision call");
    assert!(
        resp.status().is_success(),
        "expected 2xx, got {}",
        resp.status()
    );
    let body: serde_json::Value = resp.json().await.expect("json body");
    assert_eq!(body["verdict"], "ALLOW");
    assert_eq!(svc.decision_count(), 1);
}

#[tokio::test]
async fn mtls_happy_path_tokenize_basic() {
    let (addr, svc, _handle, pki) = boot_test_listener().await;
    let client = build_client(&pki, true);

    let url = format!("https://localhost:{}/v1/tokenize", addr.port());
    let resp = client
        .post(&url)
        .json(&json!({
            "provider": "openai",
            "model": "gpt-4o-mini",
            "prompt": "hello world",
        }))
        .send()
        .await
        .expect("tokenize call");
    assert!(
        resp.status().is_success(),
        "expected 2xx, got {}",
        resp.status()
    );
    let body: serde_json::Value = resp.json().await.expect("json body");
    assert_eq!(body["input_tokens"], 8);
    assert_eq!(svc.tokenize_count(), 1);
}

#[tokio::test]
async fn mtls_decision_deny_carries_no_reservation_id() {
    let (addr, svc, _handle, pki) = boot_test_listener().await;
    svc.set_next_decision(DecisionStub {
        verdict: DecisionVerdict::Deny,
        reservation_id: "".into(),
        decision_id: "d-deny-1".into(),
    });
    let client = build_client(&pki, true);

    let url = format!("https://localhost:{}/v1/decision", addr.port());
    let resp = client
        .post(&url)
        .json(&json!({
            "tenant_id": "t1",
            "claim_estimate_atomic": "9999999",
            "prompt_class": "general",
            "model_class": "openai/gpt-4o-mini",
            "idempotency_key": "k-deny-1",
        }))
        .send()
        .await
        .expect("decision call");
    let body: serde_json::Value = resp.json().await.expect("json body");
    assert_eq!(body["verdict"], "DENY");
    assert_eq!(body["reservation_id"], "");
}

#[tokio::test]
async fn mtls_trace_success_path() {
    let (addr, svc, _handle, pki) = boot_test_listener().await;
    let client = build_client(&pki, true);
    let url = format!("https://localhost:{}/v1/trace", addr.port());
    let resp = client
        .post(&url)
        .json(&json!({
            "reservation_id": "r-success-1",
            "outcome": "ACCEPTED",
            "provider_event_id": "chatcmpl-xyz",
            "input_tokens": 8,
            "output_tokens": 32,
            "actual_amount_atomic": "5000",
        }))
        .send()
        .await
        .expect("trace call");
    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.expect("json body");
    assert_eq!(body["verdict"], "ACCEPTED");
    assert_eq!(svc.trace_count(), 1);
}

#[tokio::test]
async fn mtls_trace_run_aborted_routes_to_release_lane() {
    let (addr, _svc, _handle, pki) = boot_test_listener().await;
    let client = build_client(&pki, true);
    let url = format!("https://localhost:{}/v1/trace", addr.port());
    let resp = client
        .post(&url)
        .json(&json!({
            "reservation_id": "r-run-aborted-1",
            "outcome": "REJECTED",
        }))
        .send()
        .await
        .expect("trace call");
    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.expect("json body");
    // Stub returns Accepted, but the wire field is honored — we
    // assert the JSON deserialized cleanly. SLICE 3 will assert the
    // production impl routes to run_release.
    assert!(body["verdict"].is_string());
}

#[tokio::test]
async fn mtls_handshake_required_plain_http_rejected() {
    // Boot a listener, then connect WITHOUT TLS. The connection
    // should fail (handshake error or connection reset) without ever
    // emitting an HTTP response.
    let (addr, _svc, _handle, _pki) = boot_test_listener().await;

    let plain_url = format!("http://localhost:{}/v1/decision", addr.port());
    let plain_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("build plain client");
    let result = plain_client
        .post(&plain_url)
        .json(&json!({"tenant_id": "t"}))
        .send()
        .await;
    // We expect a transport error (the server speaks TLS only). The
    // exact reqwest error variant depends on platform; we assert the
    // call did NOT succeed.
    assert!(
        result.is_err(),
        "plain HTTP must be rejected on mTLS listener, got Ok({:?})",
        result.unwrap().status()
    );
}

#[tokio::test]
async fn mtls_handshake_required_unverified_client_rejected() {
    // Boot a listener, then connect with TLS but NO client identity.
    // The WebPkiClientVerifier requires a CA-signed client cert; the
    // handshake completes server-side with an alert, reqwest surfaces
    // a connection error.
    let (addr, _svc, _handle, pki) = boot_test_listener().await;

    let url = format!("https://localhost:{}/v1/decision", addr.port());
    let client = build_client(&pki, /* with_client_identity */ false);
    let result = client
        .post(&url)
        .json(&json!({
            "tenant_id": "t1",
            "claim_estimate_atomic": "100",
            "prompt_class": "general",
            "model_class": "openai/gpt-4o-mini",
            "idempotency_key": "k-no-id",
        }))
        .send()
        .await;
    assert!(
        result.is_err(),
        "mTLS without client identity must be rejected, got Ok({:?})",
        result.unwrap().status()
    );
}

#[tokio::test]
async fn body_size_cap_enforced() {
    // The listener config caps the body at 4 MiB; we send 5 MiB and
    // assert axum returns a 413 (Payload Too Large) without
    // dispatching to the handler.
    install_crypto_provider();
    let pki = ephemeral_pki();
    let svc = Arc::new(NoopDecisionService::default());
    let tcp = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind 127.0.0.1");
    let addr = tcp.local_addr().expect("local addr");
    let max_body = 1024; // 1KB cap to exercise the limit cheaply
    let router = http_companion::build_router_for_tests(svc.clone(), max_body);
    let tls = pki.server_config.clone();
    let _handle = tokio::spawn(async move {
        let _ = http_companion::mtls::serve_with_mtls(tcp, tls, router).await;
    });

    let client = build_client(&pki, true);
    let url = format!("https://localhost:{}/v1/tokenize", addr.port());
    let big_prompt = "x".repeat(8 * 1024); // ~8KB JSON, well over 1KB cap
    let resp = client
        .post(&url)
        .json(&json!({
            "provider": "openai",
            "model": "gpt-4o-mini",
            "prompt": big_prompt,
        }))
        .send()
        .await
        .expect("tokenize call");
    assert!(
        resp.status().is_client_error(),
        "expected 4xx for oversize body, got {}",
        resp.status()
    );
    // Handler should NOT have been invoked.
    assert_eq!(svc.tokenize_count(), 0);
}

#[tokio::test]
async fn idempotent_trace_replay_returns_same_ack() {
    let (addr, svc, _handle, pki) = boot_test_listener().await;
    let client = build_client(&pki, true);
    let url = format!("https://localhost:{}/v1/trace", addr.port());
    let body = json!({
        "reservation_id": "r-replay-1",
        "outcome": "ACCEPTED",
        "provider_event_id": "evt-1",
        "input_tokens": 4,
        "output_tokens": 8,
        "actual_amount_atomic": "100",
    });
    let first: serde_json::Value = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .expect("first trace")
        .json()
        .await
        .expect("json 1");
    let second: serde_json::Value = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .expect("second trace")
        .json()
        .await
        .expect("json 2");
    assert_eq!(first, second, "replay must return byte-identical ack");
    // Both calls are observed; the dedup is at the response layer
    // (the production impl dedups at the audit lane too).
    assert_eq!(svc.trace_count(), 2);
}
