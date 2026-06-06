//! ExternalProcessor gRPC service impl — SLICE 1 skeleton.
//!
//! Handles the Handshake phase only: when an Envoy client opens a
//! `Process` stream and sends its first frame, we echo a
//! `ProcessingResponse` with a CONTINUE-status `HeadersResponse` (or
//! `BodyResponse`, matching the inbound oneof) so the mock client gets
//! a 200-equivalent and closes cleanly.
//!
//! Slices 2-4 will replace each `_` arm with real per-phase
//! translation.
//!
//! Spec refs:
//!   - docs/specs/coverage/D01_envoy_extproc/design.md §3
//!   - docs/specs/coverage/D01_envoy_extproc/implementation.md §6
//!   - Envoy ExternalProcessor proto: proto/envoy/service/ext_proc/v3/

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};
use tracing::{debug, info, warn};

use crate::proto::envoy::service::ext_proc::v3::{
    common_response::ResponseStatus, external_processor_server::ExternalProcessor,
    processing_request::Request as PReq, processing_response::Response as PResp, BodyResponse,
    CommonResponse, HeadersResponse, ProcessingRequest, ProcessingResponse,
};

/// gRPC service impl. SLICE 1 carries no per-tenant state; SLICE 3
/// will wire a `SidecarClient` field.
#[derive(Debug, Default, Clone)]
pub struct ExtProcService {
    /// Process-level tenant id (mirrored from [`crate::config::Config`])
    /// used for structured logging only in SLICE 1. SLICE 2 will route
    /// it into the sidecar Handshake assertion.
    pub tenant_id: String,
}

impl ExtProcService {
    pub fn new(tenant_id: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
        }
    }
}

#[tonic::async_trait]
impl ExternalProcessor for ExtProcService {
    type ProcessStream = ReceiverStream<Result<ProcessingResponse, Status>>;

    async fn process(
        &self,
        req: Request<Streaming<ProcessingRequest>>,
    ) -> Result<Response<Self::ProcessStream>, Status> {
        // tonic 0.12 uses a bounded mpsc; channel(4) matches the
        // implementation §6 excerpt and gives each in-flight handler
        // some slack without unbounded queueing.
        let (tx, rx) = mpsc::channel(4);
        let mut input = req.into_inner();
        let tenant_id = self.tenant_id.clone();

        tokio::spawn(async move {
            let mut frame_index: u64 = 0;
            loop {
                let msg = match input.message().await {
                    Ok(Some(m)) => m,
                    Ok(None) => {
                        debug!(
                            tenant_id = %tenant_id,
                            frames = frame_index,
                            "ExtProc client closed stream cleanly"
                        );
                        return;
                    }
                    Err(e) => {
                        warn!(
                            tenant_id = %tenant_id,
                            err = %e,
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

/// Build a `CONTINUE`-status `ProcessingResponse` matching the inbound
/// oneof variant. ExtProc protocol invariant: the server must respond
/// to a `request_headers` with a `HeadersResponse`, to a `request_body`
/// with a `BodyResponse`, etc. (see upstream
/// `external_processor.proto` doc comments). SLICE 1 emits CONTINUE
/// across the board.
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
        // Trailers / unknown — out of scope for SLICE 1 (design §3.5
        // anti-scope). We still emit a CONTINUE so the stream doesn't
        // wedge; SLICE 2 will refine.
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
    use crate::proto::envoy::service::ext_proc::v3::HttpHeaders;

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
            request: Some(PReq::RequestBody(
                crate::proto::envoy::service::ext_proc::v3::HttpBody {
                    body: bytes::Bytes::new(),
                    end_of_stream: true,
                    ..Default::default()
                },
            )),
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
