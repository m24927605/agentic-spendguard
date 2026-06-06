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

use std::sync::Arc;
use std::time::Duration;

use spendguard_envoy_extproc::proto::envoy::config::core::v3::{HeaderMap, HeaderValue};
use spendguard_envoy_extproc::proto::envoy::service::ext_proc::v3::{
    common_response::ResponseStatus, external_processor_client::ExternalProcessorClient,
    external_processor_server::ExternalProcessorServer, processing_request::Request as PReq,
    processing_response::Response as PResp, HttpBody, HttpHeaders, ProcessingRequest,
};
use spendguard_envoy_extproc::server::ExtProcService;
use spendguard_provider_routing::{init_extractors_for_test, RoutingExtractors, UsageMetrics};
use spendguard_tokenizer::Tokenizer;
use tokio_stream::StreamExt;
use tonic::transport::Server;

/// SLICE 2 helper — install no-op routing extractors so `parse::route`
/// can resolve in this test process. The provider-routing crate's
/// `init_extractors_for_test` is idempotent across parallel test
/// invocations.
fn install_test_extractors_once() {
    fn noop(_: &serde_json::Value) -> UsageMetrics {
        UsageMetrics::default()
    }
    init_extractors_for_test(RoutingExtractors {
        openai: noop,
        anthropic: noop,
        bedrock: noop,
        vertex: noop,
        azure_openai: noop,
    });
}

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

    // Boot the gRPC server in a task. SLICE 2: `ExtProcService::new`
    // now boots the embedded tokenizer; for the SLICE 1 regression we
    // route via `with_tokenizer` so the assets load once for the whole
    // file (~50ms) and subsequent SLICE 2 tests reuse the same handle.
    install_test_extractors_once();
    let tokenizer = Arc::new(Tokenizer::new_with_embedded_assets().expect("tokenizer loads"));
    let svc = ExtProcService::with_tokenizer("00000000-0000-4000-8000-000000000099", tokenizer);
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

/// SLICE 2 integration test — feed a Request-Headers + Request-Body
/// pair through the `Process` stream with a real OpenAI chat-completions
/// body and assert the per-stream state map has a `ClaimEstimate` with
/// `input_tokens > 0` for the test stream id (`x-request-id`).
///
/// Spec ref: docs/slices/COV_02_envoy_extproc_token_counter.md
/// §"Test/verification plan" item 2 (integration test extending this
/// file).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_body_phase_stashes_claim_estimate_in_state_map() {
    install_test_extractors_once();

    // Boot the service in-process; we don't need the network round trip
    // for the state-map assertion. The state map is exposed via
    // `state_map()` for tests; production callers never reach it.
    let tokenizer = Arc::new(Tokenizer::new_with_embedded_assets().expect("tokenizer loads"));
    let svc = ExtProcService::with_tokenizer("00000000-0000-4000-8000-000000000099", tokenizer);
    let state_map = svc.state_map().clone();

    // Boot the gRPC server on an ephemeral port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind to ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    drop(listener);

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server_handle = tokio::spawn(async move {
        Server::builder()
            .add_service(ExternalProcessorServer::new(svc))
            .serve_with_shutdown(addr, async {
                let _ = shutdown_rx.await;
            })
            .await
    });

    // Connect.
    let endpoint = format!("http://{addr}");
    let mut client = None;
    for _ in 0..20 {
        if let Ok(c) = ExternalProcessorClient::connect(endpoint.clone()).await {
            client = Some(c);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let mut client = client.expect("client connects within 500ms");

    // Send a 2-frame stream: RequestHeaders (carrying :path + x-request-id),
    // then RequestBody (OpenAI chat-completions JSON).
    let stream_id = "test-slice2-request-id";
    let headers_frame = ProcessingRequest {
        request: Some(PReq::RequestHeaders(HttpHeaders {
            headers: Some(HeaderMap {
                headers: vec![
                    HeaderValue {
                        key: ":path".into(),
                        value: "/v1/chat/completions".into(),
                        raw_value: Default::default(),
                    },
                    HeaderValue {
                        key: "x-request-id".into(),
                        value: stream_id.into(),
                        raw_value: Default::default(),
                    },
                ],
            }),
            attributes: Default::default(),
            end_of_stream: false,
        })),
        ..Default::default()
    };
    let body_json = serde_json::json!({
        "model": "gpt-4o-mini",
        "messages": [
            {"role": "system", "content": "You are concise."},
            {"role": "user", "content": "Translate hello to French and Spanish."}
        ]
    });
    let body_bytes = serde_json::to_vec(&body_json).expect("encode body");
    let body_frame = ProcessingRequest {
        request: Some(PReq::RequestBody(HttpBody {
            body: body_bytes.into(),
            end_of_stream: true,
            ..Default::default()
        })),
        ..Default::default()
    };

    let (tx, rx) = tokio::sync::mpsc::channel(2);
    tx.send(headers_frame).await.expect("send headers");
    tx.send(body_frame).await.expect("send body");
    drop(tx); // Half-close.

    let response = client
        .process(tokio_stream::wrappers::ReceiverStream::new(rx))
        .await
        .expect("server accepts stream");
    let mut response_stream = response.into_inner();

    // Both frames must be CONTINUE-ACKed. The SLICE 2 contract still
    // ships CONTINUE on Request-Body; SLICE 3 will replace with decision.
    let first = response_stream
        .next()
        .await
        .expect("first response")
        .expect("frame parses");
    match first.response.expect("oneof set") {
        PResp::RequestHeaders(hr) => {
            assert_eq!(
                hr.response.expect("common").status,
                ResponseStatus::Continue as i32
            );
        }
        other => panic!("expected RequestHeaders ACK, got {other:?}"),
    }
    let second = response_stream
        .next()
        .await
        .expect("second response")
        .expect("frame parses");
    match second.response.expect("oneof set") {
        PResp::RequestBody(br) => {
            assert_eq!(
                br.response.expect("common").status,
                ResponseStatus::Continue as i32
            );
        }
        other => panic!("expected RequestBody ACK, got {other:?}"),
    }

    // Pull state via the shared map. Note the server task spawns work in
    // a background tokio::spawn; the state mutation runs strictly before
    // the response is sent to the channel, so by the time we read the
    // second response above, the state mutation has already landed.
    let state = state_map.get(stream_id).await.expect("state present");
    assert_eq!(state.path, "/v1/chat/completions");
    let parsed = state.parsed.expect("parsed populated");
    assert_eq!(parsed.provider_str, "openai");
    assert_eq!(parsed.model_id, "gpt-4o-mini");
    let estimate = state.estimate.expect("estimate populated");
    assert!(
        estimate.input_tokens > 0,
        "Tier 2 estimate must yield > 0 tokens for non-empty body, got {}",
        estimate.input_tokens
    );
    assert_eq!(estimate.tokenizer_tier, "T2", "OpenAI model is Tier 2");
    assert_eq!(estimate.provider, "openai");
    assert_eq!(estimate.reserved_strategy, "A");
    assert_eq!(estimate.predicted_b_tokens, 0);
    assert_eq!(estimate.predicted_c_tokens, 0);

    let _ = shutdown_tx.send(());
    let server_result = tokio::time::timeout(Duration::from_secs(5), server_handle).await;
    assert!(server_result.is_ok());
}
