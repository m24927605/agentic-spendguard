//! ExternalProcessor gRPC service impl.
//!
//! ## Slice history
//!
//! * **SLICE 1**: Handshake-only — the first inbound `ProcessingRequest`
//!   is echoed with a CONTINUE-status response so a mock Envoy client
//!   gets a 200-equivalent and closes cleanly.
//! * **SLICE 2 (this slice)**: Wires the Request-Body phase.
//!   `RequestHeaders` captures `:path` + `x-request-id` and stashes a
//!   fresh [`StreamState`] in the shared [`StreamStateMap`].
//!   `RequestBody` parses the body, dispatches to the in-process
//!   tokenizer library, and stashes a [`ClaimEstimate`] on the state.
//!   All phases still return CONTINUE — SLICE 3 will replace the
//!   Request-Body CONTINUE with the real budget-decision translation.
//!
//! Spec refs:
//!   - docs/specs/coverage/D01_envoy_extproc/design.md §3.2, §3.4
//!   - docs/specs/coverage/D01_envoy_extproc/implementation.md §5, §6
//!   - docs/specs/coverage/D01_envoy_extproc/review-standards.md §3
//!     (SLICE 2 blocker checklist — Tier 2 hot path, unknown model T3,
//!      no fake B/C, parse-error returns typed error)

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};
use tracing::{debug, info, warn};

use crate::proto::envoy::service::ext_proc::v3::{
    common_response::ResponseStatus, external_processor_server::ExternalProcessor,
    processing_request::Request as PReq, processing_response::Response as PResp, BodyResponse,
    CommonResponse, HeadersResponse, ProcessingRequest, ProcessingResponse,
};
use crate::state::{
    derive_path_from_headers, derive_stream_id_from_headers, StreamState, StreamStateMap,
};
use crate::tokenize::estimate_tokens_or_warn;
use spendguard_tokenizer::Tokenizer;

/// gRPC service impl. SLICE 2 holds:
///   * `tenant_id` — structured-logging field (unchanged from SLICE 1).
///   * `tokenizer` — boot-time-loaded Tier 2 tokenizer; cloned via Arc
///     into each stream handler.
///   * `state_map` — process-shared per-stream state. SLICE 3 + SLICE 4
///     extend the same map.
#[derive(Clone)]
pub struct ExtProcService {
    pub tenant_id: String,
    tokenizer: Arc<Tokenizer>,
    state_map: StreamStateMap,
}

impl std::fmt::Debug for ExtProcService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtProcService")
            .field("tenant_id", &self.tenant_id)
            // Tokenizer + state_map don't surface useful debug info; keep
            // the implementation noise out of trace logs.
            .field("tokenizer", &"<Tokenizer>")
            .field("state_map", &"<StreamStateMap>")
            .finish()
    }
}

impl ExtProcService {
    /// SLICE 1 entry point — boots the Tier 2 tokenizer with the
    /// embedded assets. This is the production path: `main.rs` calls
    /// this once at startup so the binary fails fast (Tier 2 panic
    /// invariant per spec §3.6) if the bundled BPE assets fail their
    /// sha256 manifest check.
    ///
    /// Panics if `Tokenizer::new_with_embedded_assets` errors — matches
    /// the spec §7.4 fail-fast posture. The `?` form is exposed via
    /// [`Self::try_new`] for callers who want to surface the error.
    pub fn new(tenant_id: impl Into<String>) -> Self {
        let tokenizer =
            Tokenizer::new_with_embedded_assets().expect("Tier 2 tokenizer assets must load");
        Self::with_tokenizer(tenant_id, Arc::new(tokenizer))
    }

    /// Fallible variant of [`Self::new`] — useful for unit tests that
    /// want to control error handling explicitly.
    pub fn try_new(
        tenant_id: impl Into<String>,
    ) -> Result<Self, spendguard_tokenizer::TokenizerError> {
        let tokenizer = Tokenizer::new_with_embedded_assets()?;
        Ok(Self::with_tokenizer(tenant_id, Arc::new(tokenizer)))
    }

    /// Inject a pre-constructed tokenizer. Used by:
    ///   - integration tests that share one tokenizer across many
    ///     `ExtProcService` instances (faster than booting per test);
    ///   - SLICE 3 wiring that may want to dependency-inject an
    ///     instrumented `Tokenizer` variant.
    pub fn with_tokenizer(tenant_id: impl Into<String>, tokenizer: Arc<Tokenizer>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            tokenizer,
            state_map: StreamStateMap::default(),
        }
    }

    /// Read-only handle to the per-stream state map. Tests use this to
    /// assert that a `ClaimEstimate` was stashed after Request-Body.
    pub fn state_map(&self) -> &StreamStateMap {
        &self.state_map
    }
}

#[tonic::async_trait]
impl ExternalProcessor for ExtProcService {
    type ProcessStream = ReceiverStream<Result<ProcessingResponse, Status>>;

    async fn process(
        &self,
        req: Request<Streaming<ProcessingRequest>>,
    ) -> Result<Response<Self::ProcessStream>, Status> {
        let (tx, rx) = mpsc::channel(4);
        let mut input = req.into_inner();
        let tenant_id = self.tenant_id.clone();
        let tokenizer = self.tokenizer.clone();
        let state_map = self.state_map.clone();

        tokio::spawn(async move {
            // Per-stream id derived from the first Request-Headers frame.
            // SLICE 1's "first frame = handshake" semantics remain: the
            // very first inbound frame is ACKed with a CONTINUE. SLICE 2
            // adds the side effect of stashing :path + minting a stream
            // id so the subsequent Request-Body phase can look it up.
            let mut stream_id: Option<String> = None;
            let mut frame_index: u64 = 0;
            loop {
                let msg = match input.message().await {
                    Ok(Some(m)) => m,
                    Ok(None) => {
                        debug!(
                            tenant_id = %tenant_id,
                            frames = frame_index,
                            stream_id = ?stream_id,
                            "ExtProc client closed stream cleanly"
                        );
                        return;
                    }
                    Err(e) => {
                        warn!(
                            tenant_id = %tenant_id,
                            err = %e,
                            stream_id = ?stream_id,
                            "ExtProc inbound stream error; closing"
                        );
                        let _ = tx.send(Err(e)).await;
                        return;
                    }
                };

                frame_index += 1;
                if frame_index == 1 {
                    info!(
                        tenant_id = %tenant_id,
                        "ExtProc handshake frame accepted (SLICE 1 skeleton ACK)"
                    );
                }

                // Side-effect hook: SLICE 2 stashes path / parsed body
                // BEFORE building the CONTINUE response so the state map
                // is populated by the time tests assert on it. Errors
                // from the side-effect path warn-and-continue per slice
                // doc § "Scope" — SLICE 3 will fail-closed.
                handle_side_effects(&msg, &mut stream_id, &tenant_id, &tokenizer, &state_map).await;

                let resp = build_continue_for(&msg);
                if tx.send(Ok(resp)).await.is_err() {
                    debug!(
                        tenant_id = %tenant_id,
                        "ExtProc downstream receiver dropped; closing"
                    );
                    return;
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

/// Apply SLICE 2 per-phase side effects: stash :path on Request-Headers,
/// parse + tokenize on Request-Body. Side effects are bounded — on
/// failure we log and the caller still returns CONTINUE.
///
/// Splitting this out of the spawn loop keeps `process()` readable and
/// makes the side-effect surface easy to unit-test in isolation.
async fn handle_side_effects(
    msg: &ProcessingRequest,
    stream_id_slot: &mut Option<String>,
    tenant_id: &str,
    tokenizer: &Arc<Tokenizer>,
    state_map: &StreamStateMap,
) {
    match &msg.request {
        Some(PReq::RequestHeaders(h)) => {
            let headers = match &h.headers {
                Some(hm) => hm,
                None => {
                    debug!(
                        tenant_id = %tenant_id,
                        "RequestHeaders missing HeaderMap; skipping stash"
                    );
                    return;
                }
            };
            // Pre-existing id (e.g. from a SLICE 1 handshake test that
            // sent multiple RequestHeaders) wins so we don't re-key the
            // state map mid-stream.
            let id = stream_id_slot
                .clone()
                .unwrap_or_else(|| derive_stream_id_from_headers(headers));
            let path = derive_path_from_headers(headers);
            let mut state = StreamState::new();
            state.path = path.clone();
            state_map.upsert(id.clone(), state).await;
            *stream_id_slot = Some(id.clone());
            debug!(
                tenant_id = %tenant_id,
                stream_id = %id,
                path = %path,
                "RequestHeaders side effect: stashed initial StreamState"
            );
        }
        Some(PReq::RequestBody(b)) => {
            let id = match stream_id_slot.clone() {
                Some(id) => id,
                None => {
                    // Envoy AI Gateway always emits RequestHeaders before
                    // RequestBody per the v0.6 reference contract, but a
                    // mis-configured client could send body first. Mint a
                    // fallback id so we still stash + log; SLICE 3 will
                    // surface this as a typed error.
                    let id = uuid::Uuid::new_v4().to_string();
                    warn!(
                        tenant_id = %tenant_id,
                        fallback_id = %id,
                        "RequestBody arrived before RequestHeaders; minting fallback stream id"
                    );
                    state_map.upsert(id.clone(), StreamState::new()).await;
                    *stream_id_slot = Some(id.clone());
                    id
                }
            };
            // Look up the path stashed during RequestHeaders.
            let path = match state_map.get(&id).await {
                Some(s) => s.path,
                None => {
                    // Defensive: state may have been evicted under
                    // capacity pressure. Treat as missing path.
                    String::new()
                }
            };
            if path.is_empty() {
                warn!(
                    tenant_id = %tenant_id,
                    stream_id = %id,
                    "RequestBody side effect: no :path stashed; skipping parse"
                );
                return;
            }
            match crate::parse::parse_request_body(&path, &b.body) {
                Ok(parsed) => {
                    let estimate = estimate_tokens_or_warn(tokenizer, &parsed);
                    debug!(
                        tenant_id = %tenant_id,
                        stream_id = %id,
                        path = %path,
                        provider = parsed.provider_str,
                        model = %parsed.model_id,
                        input_tokens = estimate
                            .as_ref()
                            .map(|c| c.input_tokens)
                            .unwrap_or(0),
                        tokenizer_tier = %estimate
                            .as_ref()
                            .map(|c| c.tokenizer_tier.as_str())
                            .unwrap_or("none"),
                        "RequestBody side effect: parsed + estimated"
                    );
                    state_map
                        .mutate(&id, |s| {
                            s.parsed = Some(parsed);
                            s.estimate = estimate;
                        })
                        .await;
                }
                Err(e) => {
                    warn!(
                        tenant_id = %tenant_id,
                        stream_id = %id,
                        path = %path,
                        err = %e,
                        "RequestBody parse failed; SLICE 3 will fail-closed"
                    );
                }
            }
        }
        _ => {}
    }
}

/// Build a `CONTINUE`-status `ProcessingResponse` matching the inbound
/// oneof variant. ExtProc protocol invariant: the server must respond
/// to a `request_headers` with a `HeadersResponse`, to a `request_body`
/// with a `BodyResponse`, etc. (see upstream
/// `external_processor.proto` doc comments). SLICE 2 emits CONTINUE
/// across the board; SLICE 3 will replace the Request-Body arm.
fn build_continue_for(req: &ProcessingRequest) -> ProcessingResponse {
    let common = CommonResponse {
        status: ResponseStatus::Continue as i32,
        ..Default::default()
    };
    let resp = match &req.request {
        Some(PReq::RequestHeaders(_)) => PResp::RequestHeaders(HeadersResponse {
            response: Some(common),
        }),
        Some(PReq::ResponseHeaders(_)) => PResp::ResponseHeaders(HeadersResponse {
            response: Some(common),
        }),
        Some(PReq::RequestBody(_)) => PResp::RequestBody(BodyResponse {
            response: Some(common),
        }),
        Some(PReq::ResponseBody(_)) => PResp::ResponseBody(BodyResponse {
            response: Some(common),
        }),
        // Trailers / unknown — out of scope for SLICE 1-4 (design §3.5
        // anti-scope). We still emit a CONTINUE so the stream doesn't
        // wedge.
        _ => PResp::RequestHeaders(HeadersResponse {
            response: Some(common),
        }),
    };

    ProcessingResponse {
        response: Some(resp),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::envoy::config::core::v3::HeaderMap;
    use crate::proto::envoy::service::ext_proc::v3::{HttpBody, HttpHeaders};

    #[test]
    fn build_continue_for_request_headers_returns_request_headers() {
        let req = ProcessingRequest {
            request: Some(PReq::RequestHeaders(HttpHeaders {
                headers: Some(HeaderMap::default()),
                attributes: Default::default(),
                end_of_stream: false,
            })),
            ..Default::default()
        };
        let resp = build_continue_for(&req);
        match resp.response.expect("response set") {
            PResp::RequestHeaders(hr) => {
                let common = hr.response.expect("common set");
                assert_eq!(common.status, ResponseStatus::Continue as i32);
            }
            other => panic!("expected RequestHeaders, got {other:?}"),
        }
    }

    #[test]
    fn build_continue_for_request_body_returns_request_body() {
        let req = ProcessingRequest {
            request: Some(PReq::RequestBody(HttpBody {
                body: bytes::Bytes::new(),
                end_of_stream: true,
                ..Default::default()
            })),
            ..Default::default()
        };
        let resp = build_continue_for(&req);
        match resp.response.expect("response set") {
            PResp::RequestBody(br) => {
                let common = br.response.expect("common set");
                assert_eq!(common.status, ResponseStatus::Continue as i32);
            }
            other => panic!("expected RequestBody, got {other:?}"),
        }
    }
}
