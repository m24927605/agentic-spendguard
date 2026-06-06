//! SLICE 1 acceptance gate #4 — smoke test that boots the gRPC server
//! on a random port, opens a mock ExtProc client, sends a Handshake
//! frame (RequestHeaders), expects a CONTINUE ACK, closes.
//!
//! Slice doc: docs/slices/COV_01_envoy_extproc_skeleton.md §"Test/verification plan" item 4.
//! Acceptance contract: bind via `TcpListener::bind("127.0.0.1:0")`,
//! serve the gRPC server, open a client, send a Handshake-shaped first
//! frame, expect 200 / valid response back.
//!
//! Note: this test does NOT touch the sidecar UDS — `dial_sidecar_with_retry`
//! is not invoked here so the smoke test can run in any environment
//! (CI, dev laptop, no postgres, no kind).

use std::time::Duration;

use spendguard_envoy_extproc::proto::envoy::config::core::v3::{HeaderMap, HeaderValue};
use spendguard_envoy_extproc::proto::envoy::service::ext_proc::v3::{
    common_response::ResponseStatus, external_processor_client::ExternalProcessorClient,
    external_processor_server::ExternalProcessorServer, processing_request::Request as PReq,
    processing_response::Response as PResp, HttpHeaders, ProcessingRequest,
};
use spendguard_envoy_extproc::server::ExtProcService;
use tokio_stream::StreamExt;
use tonic::transport::Server;

/// Envoy v0.6 ExtProc has no distinct Handshake message; the first
/// ProcessingRequest IS the handshake. This test boots the gRPC server,
/// sends a single `RequestHeaders` frame, and asserts the server replies
/// with a `RequestHeaders` ACK carrying CommonResponse.status = CONTINUE
/// — i.e. the first inbound frame is treated as the handshake and
/// continues the stream.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_headers_first_frame_treated_as_handshake_returns_continue() {
    // Random port. tokio's TcpListener::bind("127.0.0.1:0") returns a
    // listener bound to an OS-assigned port; we read .local_addr() to
    // discover it, then drop the listener (tonic's Server::builder
    // re-binds via SocketAddr). Slightly racy but adequate for SLICE 1
    // verification — production binds to a fixed port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind to ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    drop(listener);

    // Boot the gRPC server in a task.
    let svc = ExtProcService::new("00000000-0000-4000-8000-000000000099");
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server_handle = tokio::spawn(async move {
        Server::builder()
            .add_service(ExternalProcessorServer::new(svc))
            .serve_with_shutdown(addr, async {
                let _ = shutdown_rx.await;
            })
            .await
    });

    // Give the server a moment to bind. tonic's transport doesn't
    // expose a "ready" signal so we poll with a short retry.
    let endpoint = format!("http://{addr}");
    let mut client = None;
    for _ in 0..20 {
        match ExternalProcessorClient::connect(endpoint.clone()).await {
            Ok(c) => {
                client = Some(c);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(25)).await,
        }
    }
    let mut client = client.expect("client connects within 500ms");

    // Build a single Handshake-shaped ProcessingRequest (RequestHeaders).
    let req = ProcessingRequest {
        request: Some(PReq::RequestHeaders(HttpHeaders {
            headers: Some(HeaderMap {
                headers: vec![HeaderValue {
                    key: ":path".into(),
                    value: "/v1/chat/completions".into(),
                    raw_value: Default::default(),
                }],
            }),
            attributes: Default::default(),
            end_of_stream: false,
        })),
        ..Default::default()
    };

    // Send + receive one frame.
    let (tx, rx) = tokio::sync::mpsc::channel(2);
    tx.send(req).await.expect("send request");
    drop(tx); // Half-close so the server stream sees Ok(None) after the first frame.

    let response = client
        .process(tokio_stream::wrappers::ReceiverStream::new(rx))
        .await
        .expect("server accepts stream");
    let mut response_stream = response.into_inner();

    let first = response_stream
        .next()
        .await
        .expect("at least one response frame")
        .expect("response frame parses");

    // Validate the response: must be a RequestHeaders + CommonResponse.status = CONTINUE.
    match first.response.expect("response oneof set") {
        PResp::RequestHeaders(hr) => {
            let common = hr.response.expect("common set");
            assert_eq!(
                common.status,
                ResponseStatus::Continue as i32,
                "handshake frame must be ACKed with CONTINUE"
            );
        }
        other => panic!("expected RequestHeaders ACK, got {other:?}"),
    }

    // Trigger server shutdown + await its task.
    let _ = shutdown_tx.send(());
    let server_result = tokio::time::timeout(Duration::from_secs(5), server_handle).await;
    assert!(
        server_result.is_ok(),
        "server shuts down within 5s of signal"
    );
}
