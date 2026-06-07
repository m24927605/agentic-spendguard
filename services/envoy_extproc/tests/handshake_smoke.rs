#![cfg(feature = "uds-dev")]

//! Integration smoke tests for the ExtProc gRPC server.
//!
//! ## SLICE 1
//! Acceptance gate #4 — boot the gRPC server on a random port, open a
//! mock ExtProc client, send a Handshake frame (RequestHeaders),
//! expect a CONTINUE ACK, close.
//!
//! Slice doc: docs/slices/COV_01_envoy_extproc_skeleton.md §"Test/verification plan" item 4.
//!
//! ## SLICE 2
//! Request-Body phase integration — server stashes a ClaimEstimate into
//! the per-stream state map and ACKs CONTINUE.
//!
//! ## SLICE 3 (this slice)
//! Wire the sidecar `RequestDecision` RPC end-to-end via a tempdir UDS
//! mock sidecar. Two new tests:
//!   * `slice3_request_body_calls_sidecar_and_continues_on_allow` — happy
//!     path: mock sidecar returns `CONTINUE` + a reservation_id; ExtProc
//!     must forward CONTINUE *and* stash the reservation_id on
//!     StreamState (SLICE 4 will read it).
//!   * `slice3_request_body_returns_429_on_deny` — mock sidecar returns
//!     `STOP`; ExtProc must produce an ImmediateResponse with HTTP 429
//!     and stash `DecisionOutcome::Deny`.
//!
//! These tests prove the full pipeline: ExtProc Request-Headers →
//! state stash → Request-Body parse + tokenize → sidecar RPC over UDS
//! → response mapping → state update.

use std::sync::Arc;
use std::time::Duration;

use spendguard_envoy_extproc::proto::envoy::config::core::v3::{HeaderMap, HeaderValue};
use spendguard_envoy_extproc::proto::envoy::r#type::v3::StatusCode;
use spendguard_envoy_extproc::proto::envoy::service::ext_proc::v3::{
    common_response::ResponseStatus, external_processor_client::ExternalProcessorClient,
    external_processor_server::ExternalProcessorServer, processing_request::Request as PReq,
    processing_response::Response as PResp, HttpBody, HttpHeaders, ProcessingRequest,
};
use spendguard_envoy_extproc::proto::spendguard::sidecar_adapter::v1::{
    decision_response::Decision, llm_call_post_payload::Outcome as PostOutcome,
    sidecar_adapter_server::SidecarAdapter, trace_event, ConsumeBudgetGrantRequest,
    ConsumeBudgetGrantResponse, DecisionRequest, DecisionResponse, DrainSignal,
    DrainSubscribeRequest, HandshakeRequest, HandshakeResponse, IssueBudgetGrantRequest,
    IssueBudgetGrantResponse, PublishOutcomeRequest, PublishOutcomeResponse,
    ReleaseReservationRequest, ReleaseReservationResponse, ResumeAfterApprovalRequest,
    ResumeAfterApprovalResponse, RevokeBudgetGrantRequest, RevokeBudgetGrantResponse, TraceEvent,
    TraceEventAck,
};
use spendguard_envoy_extproc::server::ExtProcService;
use spendguard_envoy_extproc::sidecar_client::{SidecarClient, DEFAULT_REQUEST_TIMEOUT};
use spendguard_envoy_extproc::state::DecisionOutcome;
use spendguard_provider_routing::{init_extractors_for_test, RoutingExtractors, UsageMetrics};
use spendguard_tokenizer::Tokenizer;
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
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

// =============================================================================
// SLICE 3 — mock SidecarAdapter over UDS + full ExtProc Request-Body flow.
// =============================================================================

/// Mock sidecar adapter that returns a configurable decision. Only
/// implements the RPCs the SLICE 3 server needs (`handshake` +
/// `request_decision`); every other RPC returns `unimplemented` so any
/// future drift surfaces as a clear panic.
struct MockSidecar {
    /// What decision to return on `request_decision`. Cloned per call.
    decision: Decision,
    /// Reservation id to surface when decision is CONTINUE.
    reservation_id: String,
    /// reason_codes for STOP/STOP_RUN_PROJECTION paths.
    reason_codes: Vec<String>,
    /// run_code_triggered for STOP_RUN_PROJECTION.
    run_code_triggered: String,
    /// Captures the last DecisionRequest the server received so tests
    /// can assert on the wire shape (idempotency.key, claim_estimate, etc).
    last_request: Arc<tokio::sync::Mutex<Option<DecisionRequest>>>,
    /// SLICE 4 — captures every TraceEvent the server received via
    /// EmitTraceEvents so tests can assert on the audit-emit wire shape.
    captured_trace_events: Arc<tokio::sync::Mutex<Vec<TraceEvent>>>,
}

impl MockSidecar {
    fn allow_with_reservation(reservation_id: impl Into<String>) -> Self {
        Self {
            decision: Decision::Continue,
            reservation_id: reservation_id.into(),
            reason_codes: Vec::new(),
            run_code_triggered: String::new(),
            last_request: Arc::new(tokio::sync::Mutex::new(None)),
            captured_trace_events: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }
    fn deny_with(reason_codes: Vec<String>, run_code: impl Into<String>) -> Self {
        Self {
            decision: Decision::Stop,
            reservation_id: String::new(),
            reason_codes,
            run_code_triggered: run_code.into(),
            last_request: Arc::new(tokio::sync::Mutex::new(None)),
            captured_trace_events: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    fn last_request_handle(&self) -> Arc<tokio::sync::Mutex<Option<DecisionRequest>>> {
        self.last_request.clone()
    }

    /// SLICE 4 — handle to the captured TraceEvent log for assertions.
    fn captured_trace_events_handle(&self) -> Arc<tokio::sync::Mutex<Vec<TraceEvent>>> {
        self.captured_trace_events.clone()
    }
}

#[tonic::async_trait]
impl SidecarAdapter for MockSidecar {
    async fn handshake(
        &self,
        _req: tonic::Request<HandshakeRequest>,
    ) -> Result<tonic::Response<HandshakeResponse>, tonic::Status> {
        Ok(tonic::Response::new(HandshakeResponse {
            sidecar_version: "mock-1.0".to_string(),
            session_id: "mock-session-1".to_string(),
            protocol_version: 1,
            ..Default::default()
        }))
    }

    async fn request_decision(
        &self,
        req: tonic::Request<DecisionRequest>,
    ) -> Result<tonic::Response<DecisionResponse>, tonic::Status> {
        let inner = req.into_inner();
        // Stash for test assertions.
        {
            let mut slot = self.last_request.lock().await;
            *slot = Some(inner.clone());
        }
        Ok(tonic::Response::new(DecisionResponse {
            decision_id: "dec-mock-1".to_string(),
            decision: self.decision as i32,
            reason_codes: self.reason_codes.clone(),
            matched_rule_ids: Vec::new(),
            mutation_patch_json: String::new(),
            effect_hash: bytes::Bytes::new(),
            ledger_transaction_id: "ltx-mock-1".to_string(),
            reservation_ids: if self.reservation_id.is_empty() {
                Vec::new()
            } else {
                vec![self.reservation_id.clone()]
            },
            terminal: matches!(self.decision, Decision::Stop | Decision::StopRunProjection),
            run_code_triggered: self.run_code_triggered.clone(),
            ..Default::default()
        }))
    }

    async fn confirm_publish_outcome(
        &self,
        _req: tonic::Request<PublishOutcomeRequest>,
    ) -> Result<tonic::Response<PublishOutcomeResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("MockSidecar: SLICE 4"))
    }

    type EmitTraceEventsStream =
        tokio_stream::wrappers::ReceiverStream<Result<TraceEventAck, tonic::Status>>;

    /// SLICE 4 — capture each TraceEvent into `captured_trace_events`,
    /// emit one TraceEventAck per event so the SidecarClient drain loop
    /// terminates. Mirrors the sidecar adapter's real bidi stream:
    /// one-event-in, one-ack-out, then end-of-stream.
    async fn emit_trace_events(
        &self,
        req: tonic::Request<tonic::Streaming<TraceEvent>>,
    ) -> Result<tonic::Response<Self::EmitTraceEventsStream>, tonic::Status> {
        let mut inbound = req.into_inner();
        let captured = self.captured_trace_events.clone();
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<TraceEventAck, tonic::Status>>(4);
        tokio::spawn(async move {
            while let Some(event_result) = inbound.next().await {
                match event_result {
                    Ok(event) => {
                        // Stash for test assertions.
                        {
                            let mut log = captured.lock().await;
                            log.push(event.clone());
                        }
                        // Emit an ACCEPTED ack.
                        let ack = TraceEventAck {
                            event_id: format!("evt-mock-{}", uuid::Uuid::new_v4().simple()),
                            status: 1, // ACCEPTED
                            error: None,
                        };
                        if tx.send(Ok(ack)).await.is_err() {
                            break;
                        }
                    }
                    Err(status) => {
                        let _ = tx.send(Err(status)).await;
                        break;
                    }
                }
            }
        });
        Ok(tonic::Response::new(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        ))
    }

    async fn issue_budget_grant(
        &self,
        _req: tonic::Request<IssueBudgetGrantRequest>,
    ) -> Result<tonic::Response<IssueBudgetGrantResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("MockSidecar: not needed"))
    }

    async fn revoke_budget_grant(
        &self,
        _req: tonic::Request<RevokeBudgetGrantRequest>,
    ) -> Result<tonic::Response<RevokeBudgetGrantResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("MockSidecar: not needed"))
    }

    async fn consume_budget_grant(
        &self,
        _req: tonic::Request<ConsumeBudgetGrantRequest>,
    ) -> Result<tonic::Response<ConsumeBudgetGrantResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("MockSidecar: not needed"))
    }

    type StreamDrainSignalStream =
        tokio_stream::wrappers::ReceiverStream<Result<DrainSignal, tonic::Status>>;

    async fn stream_drain_signal(
        &self,
        _req: tonic::Request<DrainSubscribeRequest>,
    ) -> Result<tonic::Response<Self::StreamDrainSignalStream>, tonic::Status> {
        Err(tonic::Status::unimplemented("MockSidecar: not needed"))
    }

    async fn resume_after_approval(
        &self,
        _req: tonic::Request<ResumeAfterApprovalRequest>,
    ) -> Result<tonic::Response<ResumeAfterApprovalResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("MockSidecar: not needed"))
    }

    async fn release_reservation(
        &self,
        _req: tonic::Request<ReleaseReservationRequest>,
    ) -> Result<tonic::Response<ReleaseReservationResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("MockSidecar: not needed"))
    }
}

/// Mint a tempdir UDS path. Caller is responsible for unlinking. Path
/// MUST stay under the platform `sockaddr_un.sun_path` cap (104 bytes
/// on macOS, 108 on Linux) — we use a 12-char UUID suffix and the
/// system tmp dir to stay comfortably under the limit.
fn mint_uds_path() -> std::path::PathBuf {
    let id = uuid::Uuid::new_v4().simple().to_string();
    let dir = std::env::temp_dir().join(format!("sg-ep-{}", &id[..8]));
    std::fs::create_dir_all(&dir).expect("create tempdir");
    // 4-char socket suffix keeps the path well under SUN_LEN.
    dir.join("a.sock")
}

/// Bind a UDS listener at `uds_path`, register `mock`, and serve until
/// the shutdown channel fires. Returns the server task handle.
async fn spawn_mock_sidecar(
    uds_path: std::path::PathBuf,
    mock: MockSidecar,
) -> (
    tokio::task::JoinHandle<()>,
    tokio::sync::oneshot::Sender<()>,
) {
    use spendguard_envoy_extproc::proto::spendguard::sidecar_adapter::v1::sidecar_adapter_server::SidecarAdapterServer;
    // Listen on the UDS.
    let listener = UnixListener::bind(&uds_path).expect("bind UDS");
    let incoming = UnixListenerStream::new(listener);
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let _ = Server::builder()
            .add_service(SidecarAdapterServer::new(mock))
            .serve_with_incoming_shutdown(incoming, async {
                let _ = rx.await;
            })
            .await;
    });
    (handle, tx)
}

/// Wire up an `ExtProcService` already connected to the mock sidecar.
/// Returns the service + the shared state_map (so tests can introspect
/// reservation_id / decision_outcome stashes).
async fn boot_extproc_with_sidecar(uds_path: &std::path::Path) -> ExtProcService {
    let tokenizer = Arc::new(Tokenizer::new_with_embedded_assets().expect("tokenizer loads"));
    let sidecar = SidecarClient::connect(uds_path, DEFAULT_REQUEST_TIMEOUT)
        .await
        .expect("connect to mock sidecar");
    ExtProcService::with_tokenizer("00000000-0000-4000-8000-000000000099", tokenizer)
        .with_sidecar(sidecar)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slice3_request_body_calls_sidecar_and_continues_on_allow() {
    install_test_extractors_once();

    // Stand up a tempdir UDS mock sidecar that returns CONTINUE.
    let uds = mint_uds_path();
    let mock = MockSidecar::allow_with_reservation("res-allow-123");
    let last_req_handle = mock.last_request_handle();
    let (mock_handle, mock_shutdown) = spawn_mock_sidecar(uds.clone(), mock).await;
    // Give the mock a moment to bind.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Wire the ExtProc service to the mock sidecar.
    let svc = boot_extproc_with_sidecar(&uds).await;
    assert!(svc.sidecar_wired(), "SLICE 3 must wire sidecar");
    let state_map = svc.state_map().clone();

    // Boot the ExtProc gRPC server on an ephemeral port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ExtProc port");
    let addr = listener.local_addr().expect("local_addr");
    drop(listener);

    let (extproc_shutdown_tx, extproc_shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let extproc_handle = tokio::spawn(async move {
        Server::builder()
            .add_service(ExternalProcessorServer::new(svc))
            .serve_with_shutdown(addr, async {
                let _ = extproc_shutdown_rx.await;
            })
            .await
    });

    // Connect a client.
    let endpoint = format!("http://{addr}");
    let mut client = None;
    for _ in 0..20 {
        if let Ok(c) = ExternalProcessorClient::connect(endpoint.clone()).await {
            client = Some(c);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let mut client = client.expect("ExtProc client connects within 500ms");

    let stream_id = "test-slice3-allow-stream";
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
            {"role": "user", "content": "What is 2+2?"}
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
    drop(tx);

    let response = client
        .process(tokio_stream::wrappers::ReceiverStream::new(rx))
        .await
        .expect("server accepts stream");
    let mut response_stream = response.into_inner();

    // First reply: RequestHeaders CONTINUE.
    let first = response_stream
        .next()
        .await
        .expect("first reply")
        .expect("frame parses");
    match first.response.expect("response set") {
        PResp::RequestHeaders(hr) => {
            assert_eq!(
                hr.response.expect("common").status,
                ResponseStatus::Continue as i32,
                "RequestHeaders ACKed CONTINUE"
            );
        }
        other => panic!("expected RequestHeaders, got {other:?}"),
    }

    // Second reply: RequestBody CONTINUE (from the ALLOW path).
    let second = response_stream
        .next()
        .await
        .expect("second reply")
        .expect("frame parses");
    match second.response.expect("response set") {
        PResp::RequestBody(br) => {
            assert_eq!(
                br.response.expect("common").status,
                ResponseStatus::Continue as i32,
                "RequestBody ACKed CONTINUE on sidecar ALLOW"
            );
        }
        other => panic!("expected RequestBody (ALLOW path), got {other:?}"),
    }

    // Assert SLICE 4 prerequisites: reservation_id + decision_id +
    // outcome are stashed on the per-stream state.
    let state = state_map.get(stream_id).await.expect("state present");
    assert_eq!(
        state.reservation_id.as_deref(),
        Some("res-allow-123"),
        "SLICE 3 must stash reservation_id from sidecar response"
    );
    assert_eq!(
        state.decision_id.as_deref(),
        Some("dec-mock-1"),
        "SLICE 3 must stash decision_id from sidecar response"
    );
    assert_eq!(
        state.decision_outcome,
        Some(DecisionOutcome::Allow),
        "SLICE 3 must stash DecisionOutcome::Allow"
    );

    // Assert the wire shape the mock sidecar received: review-standards
    // §4.1.1 demands a non-empty idempotency.key + a populated
    // claim_estimate.
    let captured = last_req_handle
        .lock()
        .await
        .clone()
        .expect("mock sidecar must have received a RequestDecision");
    assert!(!captured.session_id.is_empty(), "session_id non-empty");
    assert!(captured.session_id.contains(stream_id));
    let idem = captured.idempotency.expect("idempotency set");
    assert!(
        !idem.key.is_empty(),
        "review-standards §4.1.1: idempotency.key MUST be non-empty"
    );
    let inputs = captured.inputs.expect("inputs set");
    let ce = inputs.claim_estimate.expect("claim_estimate set");
    assert!(
        ce.input_tokens > 0,
        "Tier 2 must yield > 0 tokens; got {}",
        ce.input_tokens
    );
    assert_eq!(ce.tokenizer_tier, "T2");
    assert_eq!(ce.reserved_strategy, "A");
    assert_eq!(
        ce.predicted_b_tokens, 0,
        "review-standards §3.1.4: B MUST be 0"
    );
    assert_eq!(
        ce.predicted_c_tokens, 0,
        "review-standards §3.1.4: C MUST be 0"
    );
    assert_eq!(ce.model, "gpt-4o-mini");

    // Clean up — drop client, signal shutdown, drain task handles.
    let _ = extproc_shutdown_tx.send(());
    let _ = mock_shutdown.send(());
    let _ = tokio::time::timeout(Duration::from_secs(5), extproc_handle).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), mock_handle).await;
    let _ = std::fs::remove_file(&uds);
    let _ = std::fs::remove_dir_all(uds.parent().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slice3_request_body_returns_429_on_deny() {
    install_test_extractors_once();

    let uds = mint_uds_path();
    let mock = MockSidecar::deny_with(
        vec!["BUDGET_EXHAUSTED".to_string()],
        "RUN_BUDGET_PROJECTION_EXCEEDED",
    );
    let (mock_handle, mock_shutdown) = spawn_mock_sidecar(uds.clone(), mock).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let svc = boot_extproc_with_sidecar(&uds).await;
    let state_map = svc.state_map().clone();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ExtProc port");
    let addr = listener.local_addr().expect("local_addr");
    drop(listener);

    let (extproc_shutdown_tx, extproc_shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let extproc_handle = tokio::spawn(async move {
        Server::builder()
            .add_service(ExternalProcessorServer::new(svc))
            .serve_with_shutdown(addr, async {
                let _ = extproc_shutdown_rx.await;
            })
            .await
    });

    let endpoint = format!("http://{addr}");
    let mut client = None;
    for _ in 0..20 {
        if let Ok(c) = ExternalProcessorClient::connect(endpoint.clone()).await {
            client = Some(c);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let mut client = client.expect("client connects");

    let stream_id = "test-slice3-deny-stream";
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
    let body_bytes = serde_json::to_vec(&serde_json::json!({
        "model": "gpt-4o-mini",
        "messages": [{"role": "user", "content": "Hello"}]
    }))
    .unwrap();
    let body_frame = ProcessingRequest {
        request: Some(PReq::RequestBody(HttpBody {
            body: body_bytes.into(),
            end_of_stream: true,
            ..Default::default()
        })),
        ..Default::default()
    };

    let (tx, rx) = tokio::sync::mpsc::channel(2);
    tx.send(headers_frame).await.unwrap();
    tx.send(body_frame).await.unwrap();
    drop(tx);

    let response = client
        .process(tokio_stream::wrappers::ReceiverStream::new(rx))
        .await
        .expect("server accepts stream");
    let mut response_stream = response.into_inner();

    // First reply: RequestHeaders CONTINUE.
    let first = response_stream.next().await.unwrap().unwrap();
    match first.response.unwrap() {
        PResp::RequestHeaders(_) => {}
        other => panic!("expected RequestHeaders, got {other:?}"),
    }

    // Second reply: ImmediateResponse 429.
    let second = response_stream.next().await.unwrap().unwrap();
    match second.response.expect("response set") {
        PResp::ImmediateResponse(ir) => {
            let status = ir.status.expect("status set");
            assert_eq!(
                status.code,
                StatusCode::TooManyRequests as i32,
                "STOP must map to 429"
            );
            // Headers must include x-spendguard-decision: deny.
            let headers = ir.headers.expect("headers set");
            let dec = headers
                .set_headers
                .iter()
                .find(|h| {
                    h.header.as_ref().map(|hv| hv.key.as_str()) == Some("x-spendguard-decision")
                })
                .expect("x-spendguard-decision header set");
            assert_eq!(dec.header.as_ref().unwrap().value, "deny");
            // Reason codes propagated.
            let reasons = headers
                .set_headers
                .iter()
                .find(|h| {
                    h.header.as_ref().map(|hv| hv.key.as_str()) == Some("x-spendguard-reason-codes")
                })
                .expect("x-spendguard-reason-codes header set");
            assert_eq!(reasons.header.as_ref().unwrap().value, "BUDGET_EXHAUSTED");
            // run_code_triggered propagated.
            let run_code = headers
                .set_headers
                .iter()
                .find(|h| {
                    h.header.as_ref().map(|hv| hv.key.as_str()) == Some("x-spendguard-run-code")
                })
                .expect("x-spendguard-run-code header set");
            assert_eq!(
                run_code.header.as_ref().unwrap().value,
                "RUN_BUDGET_PROJECTION_EXCEEDED"
            );
            // Body MUST be empty (review-standards §4.1.3).
            assert!(
                ir.body.is_empty(),
                "body must be empty (no info disclosure)"
            );
        }
        other => panic!("expected ImmediateResponse 429, got {other:?}"),
    }

    // State stash assertions for SLICE 4.
    let state = state_map.get(stream_id).await.expect("state present");
    assert!(
        state.reservation_id.is_none(),
        "Deny path must NOT stash reservation_id"
    );
    assert_eq!(state.decision_id.as_deref(), Some("dec-mock-1"));
    assert_eq!(state.decision_outcome, Some(DecisionOutcome::Deny));

    let _ = extproc_shutdown_tx.send(());
    let _ = mock_shutdown.send(());
    let _ = tokio::time::timeout(Duration::from_secs(5), extproc_handle).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), mock_handle).await;
    let _ = std::fs::remove_file(&uds);
    let _ = std::fs::remove_dir_all(uds.parent().unwrap());
}

// =============================================================================
// SLICE 4 — Response-Body audit emit. Full pipeline:
//   Request-Headers → Request-Body (ALLOW) → Response-Headers (status) →
//   Response-Body → EmitTraceEvents(LLM_CALL_POST) → StreamState removed.
// =============================================================================

/// Helper — assemble the four ProcessingRequest frames for a full
/// Request/Response cycle through ExtProc. `response_status` is the
/// upstream status code we simulate on Response-Headers; `response_body`
/// is the JSON body forwarded from the upstream provider.
fn build_full_cycle_frames(
    stream_id: &str,
    request_body_json: &serde_json::Value,
    response_status: u16,
    response_body: &[u8],
) -> [ProcessingRequest; 4] {
    let req_headers = ProcessingRequest {
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
    let req_body = ProcessingRequest {
        request: Some(PReq::RequestBody(HttpBody {
            body: serde_json::to_vec(request_body_json).unwrap().into(),
            end_of_stream: true,
            ..Default::default()
        })),
        ..Default::default()
    };
    let resp_headers = ProcessingRequest {
        request: Some(PReq::ResponseHeaders(HttpHeaders {
            headers: Some(HeaderMap {
                headers: vec![HeaderValue {
                    key: ":status".into(),
                    value: response_status.to_string(),
                    raw_value: Default::default(),
                }],
            }),
            attributes: Default::default(),
            end_of_stream: false,
        })),
        ..Default::default()
    };
    let resp_body = ProcessingRequest {
        request: Some(PReq::ResponseBody(HttpBody {
            body: response_body.to_vec().into(),
            end_of_stream: true,
            ..Default::default()
        })),
        ..Default::default()
    };
    [req_headers, req_body, resp_headers, resp_body]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slice4_full_cycle_2xx_emits_llm_call_post_success() {
    install_test_extractors_once();

    let uds = mint_uds_path();
    let mock = MockSidecar::allow_with_reservation("res-slice4-allow");
    let trace_log = mock.captured_trace_events_handle();
    let (mock_handle, mock_shutdown) = spawn_mock_sidecar(uds.clone(), mock).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let svc = boot_extproc_with_sidecar(&uds).await;
    let state_map = svc.state_map().clone();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ExtProc port");
    let addr = listener.local_addr().expect("local_addr");
    drop(listener);

    let (extproc_shutdown_tx, extproc_shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let extproc_handle = tokio::spawn(async move {
        Server::builder()
            .add_service(ExternalProcessorServer::new(svc))
            .serve_with_shutdown(addr, async {
                let _ = extproc_shutdown_rx.await;
            })
            .await
    });

    let endpoint = format!("http://{addr}");
    let mut client = None;
    for _ in 0..20 {
        if let Ok(c) = ExternalProcessorClient::connect(endpoint.clone()).await {
            client = Some(c);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let mut client = client.expect("client connects");

    let stream_id = "test-slice4-success-stream";
    let request_body = serde_json::json!({
        "model": "gpt-4o-mini",
        "messages": [{"role": "user", "content": "What is 2+2?"}]
    });
    // OpenAI-shaped response body with usage block — SLICE 4 extracts
    // prompt_tokens=17 / completion_tokens=42.
    let response_body = serde_json::to_vec(&serde_json::json!({
        "id": "chatcmpl-mock",
        "object": "chat.completion",
        "model": "gpt-4o-mini",
        "usage": {"prompt_tokens": 17, "completion_tokens": 42, "total_tokens": 59},
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "4"}}]
    }))
    .unwrap();
    let frames = build_full_cycle_frames(stream_id, &request_body, 200, &response_body);

    let (tx, rx) = tokio::sync::mpsc::channel(4);
    for frame in frames {
        tx.send(frame).await.unwrap();
    }
    drop(tx);

    let response = client
        .process(tokio_stream::wrappers::ReceiverStream::new(rx))
        .await
        .expect("server accepts stream");
    let mut response_stream = response.into_inner();

    // Drain all 4 expected CONTINUE-ish replies.
    for i in 0..4 {
        let resp = match response_stream.next().await {
            Some(Ok(r)) => r,
            Some(Err(status)) => panic!("frame parse error #{i}: {status:?}"),
            None => panic!("missing reply #{i}"),
        };
        match resp.response.expect("response set") {
            PResp::RequestHeaders(hr) => {
                assert_eq!(
                    hr.response.expect("common").status,
                    ResponseStatus::Continue as i32
                );
            }
            PResp::RequestBody(br) => {
                assert_eq!(
                    br.response.expect("common").status,
                    ResponseStatus::Continue as i32
                );
            }
            PResp::ResponseHeaders(hr) => {
                assert_eq!(
                    hr.response.expect("common").status,
                    ResponseStatus::Continue as i32
                );
            }
            PResp::ResponseBody(br) => {
                assert_eq!(
                    br.response.expect("common").status,
                    ResponseStatus::Continue as i32
                );
            }
            other => panic!("unexpected reply variant: {other:?}"),
        }
    }

    // Give the audit-emit spawn a moment to drain (best-effort: the
    // Response-Body reply is sent before the emit completes in some
    // tonic versions; poll for at least one captured event).
    let mut events: Vec<TraceEvent> = Vec::new();
    for _ in 0..20 {
        events = trace_log.lock().await.clone();
        if !events.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        events.len(),
        1,
        "exactly one LLM_CALL_POST must be emitted per review-standards §5.1; got {} events",
        events.len()
    );
    let event = &events[0];
    assert_eq!(event.kind, trace_event::EventKind::LlmCallPost as i32);
    assert!(event.session_id.contains(stream_id));
    let payload = match event.payload.as_ref().expect("payload set") {
        trace_event::Payload::LlmCallPost(p) => p,
        _ => panic!("expected LlmCallPost payload"),
    };
    assert_eq!(payload.reservation_id, "res-slice4-allow");
    assert_eq!(payload.outcome, PostOutcome::Success as i32);
    assert_eq!(payload.actual_input_tokens, Some(17));
    assert_eq!(payload.actual_output_tokens, Some(42));
    // Strategy A reservation was input × 2; the provider reported 42 —
    // commit path sends the actual output count.
    assert_eq!(payload.estimated_amount_atomic, "42");
    // audit_code stays empty on the happy path.
    assert!(event.provider_response_metadata.is_empty());

    // StreamState must be reclaimed after the audit emit.
    assert!(
        state_map.get(stream_id).await.is_none(),
        "StreamState must be removed after successful audit emit"
    );

    let _ = extproc_shutdown_tx.send(());
    let _ = mock_shutdown.send(());
    let _ = tokio::time::timeout(Duration::from_secs(5), extproc_handle).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), mock_handle).await;
    let _ = std::fs::remove_file(&uds);
    let _ = std::fs::remove_dir_all(uds.parent().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slice4_full_cycle_5xx_emits_llm_call_post_run_aborted() {
    install_test_extractors_once();

    let uds = mint_uds_path();
    let mock = MockSidecar::allow_with_reservation("res-slice4-5xx");
    let trace_log = mock.captured_trace_events_handle();
    let (mock_handle, mock_shutdown) = spawn_mock_sidecar(uds.clone(), mock).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let svc = boot_extproc_with_sidecar(&uds).await;
    let state_map = svc.state_map().clone();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ExtProc port");
    let addr = listener.local_addr().expect("local_addr");
    drop(listener);

    let (extproc_shutdown_tx, extproc_shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let extproc_handle = tokio::spawn(async move {
        Server::builder()
            .add_service(ExternalProcessorServer::new(svc))
            .serve_with_shutdown(addr, async {
                let _ = extproc_shutdown_rx.await;
            })
            .await
    });

    let endpoint = format!("http://{addr}");
    let mut client = None;
    for _ in 0..20 {
        if let Ok(c) = ExternalProcessorClient::connect(endpoint.clone()).await {
            client = Some(c);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let mut client = client.expect("client connects");

    let stream_id = "test-slice4-5xx-stream";
    let request_body = serde_json::json!({
        "model": "gpt-4o-mini",
        "messages": [{"role": "user", "content": "Hello"}]
    });
    // 502-ish error body (no usage block). The audit emit treats this
    // as RUN_ABORTED + UPSTREAM_5XX.
    let response_body = serde_json::to_vec(&serde_json::json!({
        "error": {"message": "Upstream provider unavailable", "type": "server_error"}
    }))
    .unwrap();
    let frames = build_full_cycle_frames(stream_id, &request_body, 502, &response_body);

    let (tx, rx) = tokio::sync::mpsc::channel(4);
    for frame in frames {
        tx.send(frame).await.unwrap();
    }
    drop(tx);

    let response = client
        .process(tokio_stream::wrappers::ReceiverStream::new(rx))
        .await
        .expect("server accepts stream");
    let mut response_stream = response.into_inner();

    // Drain all 4 replies — all CONTINUE (audit emit is a side effect).
    for _ in 0..4 {
        let resp = response_stream
            .next()
            .await
            .expect("reply")
            .expect("frame parses");
        let _ = resp.response.expect("response set");
    }

    // Poll for the captured LLM_CALL_POST event.
    let mut events: Vec<TraceEvent> = Vec::new();
    for _ in 0..20 {
        events = trace_log.lock().await.clone();
        if !events.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(events.len(), 1, "exactly one LLM_CALL_POST on 5xx path");
    let payload = match events[0].payload.as_ref().expect("payload set") {
        trace_event::Payload::LlmCallPost(p) => p,
        _ => panic!("expected LlmCallPost payload"),
    };
    assert_eq!(payload.reservation_id, "res-slice4-5xx");
    assert_eq!(
        payload.outcome,
        PostOutcome::RunAborted as i32,
        "5xx upstream must map to LLM_CALL_POST.RUN_ABORTED"
    );
    assert_eq!(events[0].provider_response_metadata, "UPSTREAM_5XX");
    // No estimated commit amount on the RUN_ABORTED path — sidecar
    // takes the release path.
    assert!(payload.estimated_amount_atomic.is_empty());

    // State still removed after audit emit on the 5xx path too.
    assert!(state_map.get(stream_id).await.is_none());

    let _ = extproc_shutdown_tx.send(());
    let _ = mock_shutdown.send(());
    let _ = tokio::time::timeout(Duration::from_secs(5), extproc_handle).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), mock_handle).await;
    let _ = std::fs::remove_file(&uds);
    let _ = std::fs::remove_dir_all(uds.parent().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slice4_deny_path_does_not_emit_response_body_audit() {
    // STOP at Request-Body returns ImmediateResponse 429; Envoy never
    // invokes Response-Body, so SLICE 4 emits NOTHING from this side
    // (the sidecar audited at decision time). Verify by feeding the
    // full cycle including Response-Headers + Response-Body frames
    // (which a buggy client might still send) and asserting the audit
    // log stays empty.
    install_test_extractors_once();

    let uds = mint_uds_path();
    let mock = MockSidecar::deny_with(
        vec!["BUDGET_EXHAUSTED".to_string()],
        "RUN_BUDGET_PROJECTION_EXCEEDED",
    );
    let trace_log = mock.captured_trace_events_handle();
    let (mock_handle, mock_shutdown) = spawn_mock_sidecar(uds.clone(), mock).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let svc = boot_extproc_with_sidecar(&uds).await;
    let _state_map = svc.state_map().clone();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ExtProc port");
    let addr = listener.local_addr().expect("local_addr");
    drop(listener);

    let (extproc_shutdown_tx, extproc_shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let extproc_handle = tokio::spawn(async move {
        Server::builder()
            .add_service(ExternalProcessorServer::new(svc))
            .serve_with_shutdown(addr, async {
                let _ = extproc_shutdown_rx.await;
            })
            .await
    });

    let endpoint = format!("http://{addr}");
    let mut client = None;
    for _ in 0..20 {
        if let Ok(c) = ExternalProcessorClient::connect(endpoint.clone()).await {
            client = Some(c);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let mut client = client.expect("client connects");

    let stream_id = "test-slice4-deny-stream";
    let request_body = serde_json::json!({
        "model": "gpt-4o-mini",
        "messages": [{"role": "user", "content": "Hello"}]
    });
    let response_body = b"{\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}";
    let frames = build_full_cycle_frames(stream_id, &request_body, 200, response_body);

    let (tx, rx) = tokio::sync::mpsc::channel(4);
    for frame in frames {
        tx.send(frame).await.unwrap();
    }
    drop(tx);

    let response = client
        .process(tokio_stream::wrappers::ReceiverStream::new(rx))
        .await
        .expect("server accepts stream");
    let mut response_stream = response.into_inner();

    // Drain the replies. Note: production Envoy would stop after the
    // 429 ImmediateResponse, but our mock client doesn't — it keeps
    // feeding frames. The server still ACKs each with CONTINUE-ish
    // (the deny short-circuit only short-circuits the Request-Body
    // ACK; subsequent Response-* frames return CONTINUE).
    while let Some(Ok(_resp)) = response_stream.next().await {}

    // Audit log MUST stay empty — Deny outcome skips emit per
    // review-standards §5.1 ("no double commit"; sidecar already
    // audited at Request-Body STOP).
    tokio::time::sleep(Duration::from_millis(100)).await;
    let events = trace_log.lock().await.clone();
    assert!(
        events.is_empty(),
        "Deny path must NOT emit LLM_CALL_POST from ExtProc; got {} events",
        events.len()
    );

    let _ = extproc_shutdown_tx.send(());
    let _ = mock_shutdown.send(());
    let _ = tokio::time::timeout(Duration::from_secs(5), extproc_handle).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), mock_handle).await;
    let _ = std::fs::remove_file(&uds);
    let _ = std::fs::remove_dir_all(uds.parent().unwrap());
}
