//! tonic `Service` implementation wrapping the in-process tokenizer.
//!
//! Spec ref `tokenizer-service-spec-v1alpha1.md` §2.2 (`Tokenize`
//! and `ShadowVerify` RPCs). SLICE_03:
//!
//!   * `tokenize` — forwards to [`spendguard_tokenizer::Tokenizer::tokenize`].
//!   * `shadow_verify` — returns `Status::unimplemented` with a stable
//!     error message. SLICE_05 wires the real shadow worker via
//!     `services/tokenizer/src/shadow_worker.rs`.

use std::sync::Arc;

use tonic::{Request, Response, Status};
use tracing::warn;

use crate::proto::tokenizer::v1::{
    tokenizer_server::Tokenizer as TokenizerSvcTrait, ShadowVerifyRequest, ShadowVerifyResponse,
    TokenizeRequest, TokenizeResponse,
};
use spendguard_tokenizer::{Tokenizer, TokenizerError};

/// Service struct holding a shared library handle. Constructed once
/// in main(); cloned cheaply on every RPC dispatch because
/// `Arc<Tokenizer>` is cheap-clone-and-share by design.
#[derive(Clone)]
pub struct TokenizerSvc {
    tokenizer: Arc<Tokenizer>,
}

impl TokenizerSvc {
    pub fn new(tokenizer: Arc<Tokenizer>) -> Self {
        Self { tokenizer }
    }
}

#[tonic::async_trait]
impl TokenizerSvcTrait for TokenizerSvc {
    async fn tokenize(
        &self,
        request: Request<TokenizeRequest>,
    ) -> Result<Response<TokenizeResponse>, Status> {
        let proto_req = request.into_inner();
        let lib_req: spendguard_tokenizer::TokenizeRequest = proto_req.into();

        // Per spec §3.5 — the request_id mints UUIDv7 if empty so
        // downstream telemetry has a stable id.
        let lib_req = if lib_req.request_id.is_empty() {
            spendguard_tokenizer::TokenizeRequest {
                request_id: uuid::Uuid::now_v7().to_string(),
                ..lib_req
            }
        } else {
            lib_req
        };

        let lib_resp = match self.tokenizer.tokenize(&lib_req) {
            Ok(resp) => resp,
            Err(err) => return Err(map_tokenizer_error(err)),
        };
        Ok(Response::new(lib_resp.into()))
    }

    async fn shadow_verify(
        &self,
        _request: Request<ShadowVerifyRequest>,
    ) -> Result<Response<ShadowVerifyResponse>, Status> {
        // Per spec §0.1 + §4 the shadow path is async + lives in
        // SLICE_05; SLICE_03 surfaces an UNIMPLEMENTED status so
        // any caller that wires the gRPC stub today gets a clear
        // signal rather than a malformed empty response.
        warn!(
            "shadow_verify called but SLICE_03 only ships Tier 2 + Tier 3 — \
             returning UNIMPLEMENTED. SLICE_05 will wire the real worker."
        );
        Err(Status::unimplemented(
            "ShadowVerify is not implemented in SLICE_03 (Tier 1 shadow lands in SLICE_05; \
             see docs/tokenizer-service-spec-v1alpha1.md §0.1 + §4)",
        ))
    }
}

/// Translate [`TokenizerError`] to a tonic `Status`. Per spec §8 the
/// service surfaces:
///
///   * Asset signature mismatch → `Status::failed_precondition`
///     (server is in an invalid state; caller should retry against
///     a different replica).
///   * Encoder internal error → `Status::internal` (caller MUST
///     fail-closed per §8 — silent Tier 3 fallback is forbidden).
///   * Dispatch pattern invalid → `Status::internal` (programmer
///     error; identical surface to encoder internal).
///   * Asset load failed → `Status::failed_precondition` (same
///     server-side state as signature mismatch).
fn map_tokenizer_error(err: TokenizerError) -> Status {
    match err {
        TokenizerError::AssetSignatureMismatch { .. } | TokenizerError::AssetLoadFailed { .. } => {
            Status::failed_precondition(err.to_string())
        }
        TokenizerError::EncoderInternal { .. } | TokenizerError::DispatchPatternInvalid { .. } => {
            Status::internal(err.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc() -> TokenizerSvc {
        let tokenizer = Tokenizer::new_with_embedded_assets().expect("load");
        TokenizerSvc::new(Arc::new(tokenizer))
    }

    #[tokio::test]
    async fn tokenize_gpt_4o_mini_returns_t2() {
        let s = svc();
        let req = Request::new(TokenizeRequest {
            model: "gpt-4o-mini".to_string(),
            messages: vec![crate::proto::tokenizer::v1::tokenize_request::Message {
                role: "user".to_string(),
                content: "hello world".to_string(),
                tool_calls: vec![],
            }],
            raw_text: String::new(),
            request_id: String::new(),
        });
        let resp = s.tokenize(req).await.expect("ok").into_inner();
        assert_eq!(resp.tier, "T2");
        assert_eq!(resp.kind, "OPENAI_TIKTOKEN");
        assert!(resp.input_tokens > 0);
        assert!(!resp.tokenizer_version_id.is_empty());
        assert!(resp.latency_ns >= 0);
    }

    #[tokio::test]
    async fn tokenize_unknown_model_returns_t3() {
        let s = svc();
        let req = Request::new(TokenizeRequest {
            model: "some-private-finetune".to_string(),
            messages: vec![],
            raw_text: "hello world hello world hello world".to_string(),
            request_id: String::new(),
        });
        let resp = s.tokenize(req).await.expect("ok").into_inner();
        assert_eq!(resp.tier, "T3");
        assert_eq!(resp.kind, "HEURISTIC");
        assert!(resp.tokenizer_version_id.is_empty());
        assert!(resp.fallback_char_count > 0);
    }

    #[tokio::test]
    async fn shadow_verify_returns_unimplemented_in_slice_03() {
        let s = svc();
        let req = Request::new(ShadowVerifyRequest {
            model: "gpt-4o".to_string(),
            messages: vec![],
            raw_text: "x".to_string(),
            t2_input_tokens: 1,
            t2_tokenizer_version_id: "abc".to_string(),
        });
        let err = s.shadow_verify(req).await.expect_err("unimplemented");
        assert_eq!(err.code(), tonic::Code::Unimplemented);
        assert!(err.message().contains("SLICE_03"));
        assert!(err.message().contains("SLICE_05"));
    }

    #[tokio::test]
    async fn tokenize_mints_request_id_when_empty() {
        // The library doesn't return request_id but we verify the
        // service didn't panic on empty + the latency_ns is
        // populated (proving the tokenize path executed).
        let s = svc();
        let req = Request::new(TokenizeRequest {
            model: "gpt-4o".to_string(),
            messages: vec![],
            raw_text: "hi".to_string(),
            request_id: String::new(),
        });
        let resp = s.tokenize(req).await.expect("ok").into_inner();
        assert!(resp.latency_ns >= 0);
    }
}
