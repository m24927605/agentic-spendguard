//! ExternalProcessor gRPC service impl.
//!
//! ## Slice history
//!
//! * **SLICE 1**: Handshake-only — the first inbound `ProcessingRequest`
//!   is echoed with a CONTINUE-status response so a mock Envoy client
//!   gets a 200-equivalent and closes cleanly.
//! * **SLICE 2**: Wires the Request-Body phase.
//!   `RequestHeaders` captures `:path` + `x-request-id` and stashes a
//!   fresh [`StreamState`] in the shared [`StreamStateMap`].
//!   `RequestBody` parses the body, dispatches to the in-process
//!   tokenizer library, and stashes a [`ClaimEstimate`] on the state.
//!   All phases still return CONTINUE.
//! * **SLICE 3 (this slice)**: Wires the budget-decision RPC.
//!   `RequestBody` translates `StreamState` → sidecar `RequestDecision`,
//!   maps `DecisionResponse` → ExtProc `ProcessingResponse`
//!   (CONTINUE / 429 / 403 / 503), and stashes `reservation_id` +
//!   `decision_id` + `decision_outcome` on the per-stream state for
//!   SLICE 4's audit-emit consumer. `RequestHeaders` /
//!   `ResponseHeaders` / `ResponseBody` still return CONTINUE — SLICE
//!   4 will wire `EmitTraceEvents` on `ResponseBody`.
//!
//! Spec refs:
//!   - docs/specs/coverage/D01_envoy_extproc/design.md §3.2, §3.4, §3.5
//!   - docs/specs/coverage/D01_envoy_extproc/implementation.md §5, §6
//!   - docs/specs/coverage/D01_envoy_extproc/review-standards.md §4
//!     (SLICE 3 blocker checklist — non-empty idempotency key, timeout
//!      enforcement, fail-closed on Unspecified, no info disclosure,
//!      reservation_id stash for SLICE 4)

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};
use tracing::{debug, info, warn};

use crate::decision::{build_request_decision, DecisionBuildCtx};
use crate::proto::envoy::service::ext_proc::v3::{
    common_response::ResponseStatus, external_processor_server::ExternalProcessor,
    processing_request::Request as PReq, processing_response::Response as PResp, BodyResponse,
    CommonResponse, HeadersResponse, ProcessingRequest, ProcessingResponse,
};
use crate::response::{
    build_extproc_response, build_sidecar_error_response, missing_estimate_immediate,
    MappedResponse,
};
use crate::sidecar_client::SidecarClient;
use crate::state::{
    derive_path_from_headers, derive_stream_id_from_headers, StreamState, StreamStateMap,
};
use crate::tokenize::estimate_tokens_or_warn;
use spendguard_tokenizer::Tokenizer;

/// gRPC service impl. SLICE 3 holds:
///   * `tenant_id` — structured-logging + sidecar tenant assertion (unchanged from SLICE 1).
///   * `tokenizer` — boot-time-loaded Tier 2 tokenizer; cloned via Arc
///     into each stream handler.
///   * `state_map` — process-shared per-stream state. SLICE 4 reads the
///     same map at Response-Body time for audit emit.
///   * `sidecar` — optional sidecar adapter client. `None` for SLICE 1
///     skeleton / SLICE 2 tokenizer tests; `Some` for SLICE 3 budget-
///     query wired tests + production. When `None`, Request-Body returns
///     a SLICE 2-compatible CONTINUE so the older test fixtures pass.
#[derive(Clone)]
pub struct ExtProcService {
    pub tenant_id: String,
    tokenizer: Arc<Tokenizer>,
    state_map: StreamStateMap,
    sidecar: Option<SidecarClient>,
}

impl std::fmt::Debug for ExtProcService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtProcService")
            .field("tenant_id", &self.tenant_id)
            // Tokenizer + state_map don't surface useful debug info; keep
            // the implementation noise out of trace logs.
            .field("tokenizer", &"<Tokenizer>")
            .field("state_map", &"<StreamStateMap>")
            .field("sidecar_wired", &self.sidecar.is_some())
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
    ///   - SLICE 3 wiring that needs an `ExtProcService` with no
    ///     sidecar (regression coverage for the SLICE 2 path).
    pub fn with_tokenizer(tenant_id: impl Into<String>, tokenizer: Arc<Tokenizer>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            tokenizer,
            state_map: StreamStateMap::default(),
            sidecar: None,
        }
    }

    /// SLICE 3 entry point — wires an already-constructed
    /// [`SidecarClient`] into the service. Once present, the
    /// Request-Body phase translates each frame into a
    /// `RequestDecision` RPC and maps the response to the right
    /// ExtProc reply. `main.rs` calls this once at startup after the
    /// SLICE 1 fail-fast handshake dial proves the UDS is reachable.
    pub fn with_sidecar(mut self, sidecar: SidecarClient) -> Self {
        self.sidecar = Some(sidecar);
        self
    }

    /// Read-only handle to the per-stream state map. Tests use this to
    /// assert that a `ClaimEstimate` was stashed after Request-Body and
    /// that SLICE 3 wired the `reservation_id` / `decision_id` /
    /// `decision_outcome` after the sidecar RPC.
    pub fn state_map(&self) -> &StreamStateMap {
        &self.state_map
    }

    /// Returns true if SLICE 3 sidecar wiring is active. Used by tests
    /// to assert the regression flag is set.
    pub fn sidecar_wired(&self) -> bool {
        self.sidecar.is_some()
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
        let sidecar = self.sidecar.clone();

        tokio::spawn(async move {
            // Per-stream id derived from the first Request-Headers frame.
            // SLICE 1's "first frame = handshake" semantics remain: the
            // very first inbound frame is ACKed with a CONTINUE. SLICE 2
            // adds the side effect of stashing :path + minting a stream
            // id so the subsequent Request-Body phase can look it up.
            // SLICE 3 (this slice) replaces the Request-Body CONTINUE
            // with the real budget-decision translation when `sidecar`
            // is wired.
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

                // Side-effect hook (SLICE 2): stashes path / parsed body
                // BEFORE building the response so the state map is
                // populated by the time we issue the sidecar RPC.
                handle_side_effects(&msg, &mut stream_id, &tenant_id, &tokenizer, &state_map).await;

                // SLICE 3 hot path: when the inbound frame is a
                // Request-Body AND the sidecar is wired, build a
                // RequestDecision, dispatch, and map the response.
                // Anything else falls back to the SLICE 2 CONTINUE.
                let resp = match (&msg.request, &sidecar) {
                    (Some(PReq::RequestBody(_)), Some(client)) => {
                        let mapped = handle_request_body_budget_query(
                            stream_id.as_deref(),
                            &tenant_id,
                            &state_map,
                            client,
                        )
                        .await;
                        // Stash SLICE 3 outcome on the per-stream state
                        // so SLICE 4's audit-emit (Response-Body) can
                        // reference reservation_id / decision_id.
                        stash_outcome_on_state(&state_map, stream_id.as_deref(), &mapped).await;
                        mapped.processing
                    }
                    _ => build_continue_for(&msg),
                };

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

/// SLICE 3 Request-Body hot path.
///
/// Reads SLICE 2's stashed [`StreamState`], builds a `DecisionRequest`,
/// invokes the sidecar adapter, and maps the response. Three failure
/// paths fold into a fail-closed 503/distinct-503:
///   * No stream id (RequestBody before RequestHeaders) → 503 missing-estimate
///   * State map miss / no ClaimEstimate → 503 missing-estimate
///   * Sidecar RPC error / timeout → 503 sidecar-unavailable
///
/// On success the response is whatever `build_extproc_response` decides
/// based on the sidecar `Decision` enum (CONTINUE / 429 / 403 / 503).
async fn handle_request_body_budget_query(
    stream_id: Option<&str>,
    tenant_id: &str,
    state_map: &StreamStateMap,
    sidecar: &SidecarClient,
) -> MappedResponse {
    let Some(stream_id) = stream_id else {
        warn!(
            tenant_id = %tenant_id,
            "RequestBody arrived without stream id; failing closed (503 missing-estimate)"
        );
        return missing_estimate_immediate(1);
    };

    // Pull the SLICE 2 state. Cloning the StreamState is cheap (~few
    // hundred bytes) and lets us drop the lock before issuing the RPC.
    let state_snapshot = match state_map.get(stream_id).await {
        Some(s) => s,
        None => {
            warn!(
                tenant_id = %tenant_id,
                stream_id = %stream_id,
                "RequestBody: StreamState missing from map (evicted under pressure?); failing closed"
            );
            return missing_estimate_immediate(1);
        }
    };

    // Build the decision request.
    let ctx = DecisionBuildCtx {
        tenant_id,
        stream_id,
        unit_id: None,
    };
    let req = match build_request_decision(&state_snapshot, &ctx) {
        Ok(r) => r,
        Err(e) => {
            warn!(
                tenant_id = %tenant_id,
                stream_id = %stream_id,
                err = %e,
                "RequestBody: SLICE 2 ClaimEstimate missing; failing closed (review-standards §4.1.1)"
            );
            return missing_estimate_immediate(1);
        }
    };

    // Hot path RPC. `request_decision` enforces the configured
    // timeout (review-standards §4.1.2); see sidecar_client.rs.
    match sidecar.request_decision(req).await {
        Ok(resp) => {
            info!(
                tenant_id = %tenant_id,
                stream_id = %stream_id,
                decision = resp.decision,
                decision_id = %resp.decision_id,
                "RequestBody: sidecar RequestDecision returned"
            );
            build_extproc_response(resp)
        }
        Err(e) => {
            warn!(
                tenant_id = %tenant_id,
                stream_id = %stream_id,
                err = %e,
                "RequestBody: sidecar RPC failed; failing closed (503)"
            );
            build_sidecar_error_response(&e)
        }
    }
}

/// Stash SLICE 3 outcome on the per-stream state. SLICE 4 reads
/// these fields during Response-Body audit emit. When the state row
/// has been evicted under capacity pressure (mutate returns `false`),
/// we surface a warn! so the SLICE 4 audit-emit consumer's
/// `decision_outcome: None` is correlatable to the eviction event —
/// otherwise a missing outcome silently degrades the audit trail.
async fn stash_outcome_on_state(
    state_map: &StreamStateMap,
    stream_id: Option<&str>,
    mapped: &MappedResponse,
) {
    let Some(id) = stream_id else {
        return;
    };
    let reservation_id = mapped.reservation_id.clone();
    let decision_id = mapped.decision_id.clone();
    let outcome = mapped.outcome;
    let mutated = state_map
        .mutate(id, move |s| {
            s.reservation_id = reservation_id;
            s.decision_id = decision_id;
            s.decision_outcome = Some(outcome);
        })
        .await;
    if !mutated {
        warn!(
            stream_id = %id,
            ?outcome,
            "stream state evicted before SLICE 3 outcome could be stashed; SLICE 4 audit emit will see decision_outcome: None"
        );
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
