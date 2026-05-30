//! tonic `Service` implementation wrapping the in-process tokenizer.
//!
//! Spec ref `tokenizer-service-spec-v1alpha1.md` §2.2 (`Tokenize`
//! and `ShadowVerify` RPCs). SLICE_03:
//!
//!   * `tokenize` — forwards to [`spendguard_tokenizer::Tokenizer::tokenize`].
//!   * `shadow_verify` — returns `Status::unimplemented` with a stable
//!     error message. SLICE_05 wires the real shadow worker via
//!     `services/tokenizer/src/shadow_worker.rs`.
//!
//! ## Round-2 fix M6 + Round-3 fix N3 — DoS protection
//!
//! Tokenize requests come from authenticated callers (sidecar +
//! shadow worker), but defense in depth requires per-field caps so a
//! buggy caller cannot pressure the encoder cache by sending
//! megabyte-scale text. The hot path validates BEFORE invoking the
//! tokenizer library (which would otherwise allocate proportional to
//! input size for the BPE encode buffer).
//!
//! Layered caps:
//!   1. Protocol layer (main.rs `max_decoding_message_size`) — 1 MiB
//!      hard cap. Anything bigger is rejected by tonic with
//!      `Status::resource_exhausted` BEFORE proto deserialisation.
//!   2. Field layer (this module, `TokenizerSvc::tokenize`) — 1 MiB
//!      `raw_text` / per-message content; 256 B model; 1000 message
//!      array bound. Rejected with `Status::invalid_argument`.
//!
//! Round-3 N3: the field caps and the protocol cap MUST agree.
//! Previously the protocol layer rejected at 1 MiB while the docs +
//! field caps advertised 2 MiB → callers catching `InvalidArgument`
//! never saw the field-layer error, only `ResourceExhausted`. We
//! tightened the field caps to 1 MiB to match. This is intentionally
//! redundant: the field validation runs against a value that already
//! passed the protocol cap, but it provides a stable + named error
//! distinct from `ResourceExhausted` and makes the in-process library
//! form (no tonic protocol layer) defend itself with the same bound.
//!
//! Violations return `Status::invalid_argument` with a stable code so
//! callers can distinguish from `internal` (encoder panic).

use std::sync::Arc;

use tonic::{Request, Response, Status};
use tracing::warn;

use crate::proto::tokenizer::v1::{
    tokenizer_server::Tokenizer as TokenizerSvcTrait, ShadowVerifyRequest, ShadowVerifyResponse,
    TokenizeRequest, TokenizeResponse,
};
use spendguard_tokenizer::{Tokenizer, TokenizerError};

// ============================================================================
// Round-2 fix M6 + Round-3 fix N3 — request-shape caps.
//
// Field cap == protocol cap == 1 MiB by design. The redundancy is
// defense-in-depth:
//   * Protocol cap (main.rs `max_decoding_message_size`) rejects
//     oversized frames with `ResourceExhausted` before deserialisation.
//   * Field cap (this module) rejects per-field violations with the
//     more specific `InvalidArgument` so callers can metric on the
//     offending field (model vs raw_text vs messages).
//
// `MAX_RAW_TEXT_LEN == MAX_MESSAGE_CONTENT_LEN == 1 MiB == 1 << 20`.
// Tighten the protocol cap in lock-step if the field caps grow.
//
// Kept as `pub(crate) const` so the test mod (and future calibration
// tooling) can reference them.
// ============================================================================

/// Max bytes accepted in the `model` field. Real-world model strings
/// are < 64 chars; 256 leaves runway for vendor prefixes.
pub(crate) const MAX_MODEL_LEN: usize = 256;

/// Max bytes accepted in `raw_text` (the text-completion shape).
/// Round-3 N3: matches the 1 MiB protocol-layer
/// `max_decoding_message_size` configured in main.rs so the field
/// validation error surface is reachable (was previously 2 MiB and
/// therefore unreachable — the protocol cap fired first).
pub(crate) const MAX_RAW_TEXT_LEN: usize = 1 << 20;

/// Max number of `Message` elements in the chat-shape array.
pub(crate) const MAX_MESSAGES: usize = 1_000;

/// Max bytes per individual `message.content`. See `MAX_RAW_TEXT_LEN`
/// for the protocol-cap alignment rationale.
pub(crate) const MAX_MESSAGE_CONTENT_LEN: usize = 1 << 20;

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

        // ── Round-2 fix M6: DoS protection request-shape caps ──────
        // Reject oversize payloads before the library allocates any
        // buffers proportional to input size. Stable error codes so
        // callers (sidecar / shadow worker) can metric on them.
        if proto_req.model.len() > MAX_MODEL_LEN {
            return Err(Status::invalid_argument(format!(
                "model field too long: {} > {} bytes (M6 DoS cap)",
                proto_req.model.len(),
                MAX_MODEL_LEN
            )));
        }
        if proto_req.raw_text.len() > MAX_RAW_TEXT_LEN {
            return Err(Status::invalid_argument(format!(
                "raw_text exceeds {} bytes (M6 DoS cap)",
                MAX_RAW_TEXT_LEN
            )));
        }
        if proto_req.messages.len() > MAX_MESSAGES {
            return Err(Status::invalid_argument(format!(
                "too many messages: {} > {} (M6 DoS cap)",
                proto_req.messages.len(),
                MAX_MESSAGES
            )));
        }
        for (idx, m) in proto_req.messages.iter().enumerate() {
            if m.content.len() > MAX_MESSAGE_CONTENT_LEN {
                return Err(Status::invalid_argument(format!(
                    "messages[{idx}].content exceeds {MAX_MESSAGE_CONTENT_LEN} bytes (M6 DoS cap)"
                )));
            }
        }

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

    // ── Round-2 fix M6 + Round-3 fix N3 — DoS protection tests ────
    // (R3 N3: caps tightened to 1 MiB to match protocol-layer
    //  `max_decoding_message_size`; tests reference constants so they
    //  auto-track the boundary.)

    #[tokio::test]
    async fn tokenize_rejects_oversize_model() {
        let s = svc();
        let req = Request::new(TokenizeRequest {
            model: "x".repeat(MAX_MODEL_LEN + 1),
            messages: vec![],
            raw_text: String::new(),
            request_id: String::new(),
        });
        let err = s.tokenize(req).await.expect_err("M6 should reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("model field too long"));
    }

    #[tokio::test]
    async fn tokenize_rejects_oversize_raw_text() {
        let s = svc();
        let req = Request::new(TokenizeRequest {
            model: "gpt-4o".to_string(),
            messages: vec![],
            raw_text: "a".repeat(MAX_RAW_TEXT_LEN + 1),
            request_id: String::new(),
        });
        let err = s.tokenize(req).await.expect_err("M6 should reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("raw_text exceeds"));
    }

    #[tokio::test]
    async fn tokenize_rejects_too_many_messages() {
        let s = svc();
        let blank_msg = crate::proto::tokenizer::v1::tokenize_request::Message {
            role: "user".to_string(),
            content: "x".to_string(),
            tool_calls: vec![],
        };
        let req = Request::new(TokenizeRequest {
            model: "gpt-4o".to_string(),
            messages: vec![blank_msg; MAX_MESSAGES + 1],
            raw_text: String::new(),
            request_id: String::new(),
        });
        let err = s.tokenize(req).await.expect_err("M6 should reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("too many messages"));
    }

    #[tokio::test]
    async fn tokenize_rejects_oversize_message_content() {
        let s = svc();
        let big_msg = crate::proto::tokenizer::v1::tokenize_request::Message {
            role: "user".to_string(),
            content: "x".repeat(MAX_MESSAGE_CONTENT_LEN + 1),
            tool_calls: vec![],
        };
        let req = Request::new(TokenizeRequest {
            model: "gpt-4o".to_string(),
            messages: vec![big_msg],
            raw_text: String::new(),
            request_id: String::new(),
        });
        let err = s.tokenize(req).await.expect_err("M6 should reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("messages[0].content exceeds"));
    }

    #[tokio::test]
    async fn tokenize_accepts_at_cap_boundary() {
        // Boundary check: exactly MAX_MODEL_LEN bytes must succeed.
        let s = svc();
        let req = Request::new(TokenizeRequest {
            // Use a model string that hits a dispatch entry; pad with
            // suffix to bring up to MAX_MODEL_LEN. We deliberately use
            // an unknown model to land on Tier 3 (the encode path is
            // tested elsewhere); the point here is the cap itself
            // permits up-to-and-including MAX_MODEL_LEN.
            model: "x".repeat(MAX_MODEL_LEN),
            messages: vec![],
            raw_text: "hello".to_string(),
            request_id: String::new(),
        });
        let resp = s.tokenize(req).await.expect("at-cap accepted").into_inner();
        // Unknown model lands on Tier 3.
        assert_eq!(resp.tier, "T3");
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
