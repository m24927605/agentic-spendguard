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
//!   1. Protocol layer (main.rs `max_decoding_message_size`) — 4 MiB
//!      hard cap. Anything bigger is rejected by tonic with
//!      `Status::resource_exhausted` BEFORE proto deserialisation.
//!   2. Field layer (this module, `TokenizerSvc::tokenize`) — 4 MiB
//!      `raw_text` / per-message content; 256 B model; 1000 message
//!      array bound. Rejected with `Status::invalid_argument`.
//!
//! Round-3 N3 / POST_GA_03 #114: the field caps and the protocol cap
//! MUST agree. The shared cap is now 4 MiB so realistic multi-turn
//! prompts can traverse the sidecar/tokenizer path while oversized
//! frames remain rejected before encoder work begins. This is
//! intentionally redundant: the field validation runs against a value
//! that already passed the protocol cap, but it provides a stable +
//! named error distinct from `ResourceExhausted` and makes the
//! in-process library form (no tonic protocol layer) defend itself
//! with the same bound.
//!
//! Violations return `Status::invalid_argument` with a stable code so
//! callers can distinguish from `internal` (encoder panic).

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;

use tonic::{Request, Response, Status};
use tracing::warn;

use crate::proto::tokenizer::v1::{
    tokenizer_server::Tokenizer as TokenizerSvcTrait, ShadowVerifyRequest, ShadowVerifyResponse,
    TokenizeRequest, TokenizeResponse,
};
use crate::shadow::worker::{ShadowEvent, ShadowWorkerHandle};
use spendguard_tokenizer::encoders::EncoderKind;
use spendguard_tokenizer::{Tokenizer, TokenizerError};

// ============================================================================
// Round-2 fix M6 + Round-3 fix N3 — request-shape caps.
//
// Field cap == protocol cap == 4 MiB by design. The redundancy is
// defense-in-depth:
//   * Protocol cap (main.rs `max_decoding_message_size`) rejects
//     oversized frames with `ResourceExhausted` before deserialisation.
//   * Field cap (this module) rejects per-field violations with the
//     more specific `InvalidArgument` so callers can metric on the
//     offending field (model vs raw_text vs messages).
//
// `MAX_RAW_TEXT_LEN == MAX_MESSAGE_CONTENT_LEN == 4 MiB == 4 << 20`.
// Tighten the protocol cap in lock-step if the field caps grow.
//
// Kept as `pub(crate) const` so the test mod (and future calibration
// tooling) can reference them.
// ============================================================================

/// Max bytes accepted in the `model` field. Real-world model strings
/// are < 64 chars; 256 leaves runway for vendor prefixes.
pub(crate) const MAX_MODEL_LEN: usize = 256;

/// POST_GA_03 / #114: shared decoded-message cap for the gRPC protocol
/// layer and this field-validation layer. Raised from 1 MiB to 4 MiB so
/// SLICE_10 sidecar integrations can carry realistic multi-turn prompts
/// without weakening the layered cap invariant.
pub const TOKENIZER_REQUEST_CAP_BYTES: usize = 4 << 20;

/// POST_GA_03 / #127: bound synchronous BPE work from the gRPC service
/// form. The in-process library form keeps its direct synchronous API;
/// callers that need a timeout should wrap it at their trust boundary.
const ENCODE_TIMEOUT: Duration = Duration::from_millis(100);

pub static INVALID_REQUEST_ID_TOTAL: AtomicU64 = AtomicU64::new(0);
pub static REQUEST_ID_V4_ACCEPTED_TOTAL: AtomicU64 = AtomicU64::new(0);
pub static ENCODE_TIMEOUT_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Max bytes accepted in `raw_text` (the text-completion shape).
/// POST_GA_03 / #114: matches the 4 MiB protocol-layer
/// `max_decoding_message_size` configured in main.rs so the field
/// validation error surface is reachable.
pub(crate) const MAX_RAW_TEXT_LEN: usize = TOKENIZER_REQUEST_CAP_BYTES;

/// Max number of `Message` elements in the chat-shape array.
pub(crate) const MAX_MESSAGES: usize = 1_000;

/// Max bytes per individual `message.content`. See `MAX_RAW_TEXT_LEN`
/// for the protocol-cap alignment rationale.
pub(crate) const MAX_MESSAGE_CONTENT_LEN: usize = TOKENIZER_REQUEST_CAP_BYTES;

/// Service struct holding a shared library handle. Constructed once
/// in main(); cloned cheaply on every RPC dispatch because
/// `Arc<Tokenizer>` is cheap-clone-and-share by design.
///
/// SLICE_05: also holds an optional [`ShadowWorkerHandle`] for
/// fire-and-forget Tier 1 shadow event submission. The handle is
/// `Option<…>` because SLICE_03 / SLICE_04 callers (existing tests +
/// the build-up bootstrap path) construct the service without a worker.
/// When `Some` the service handler tries-sends an event AFTER returning
/// the Tier 2 response to the caller — Tier 2 hot path latency is
/// structurally unaffected because the channel is bounded + try_send
/// is non-blocking + the send result is intentionally ignored.
#[derive(Clone)]
pub struct TokenizerSvc {
    tokenizer: Arc<Tokenizer>,
    shadow_worker: Option<ShadowWorkerHandle>,
    /// Per-request tenant id pulled from a gRPC metadata header.
    /// Empty when the caller is anonymous (test / library form).
    /// Phase F's control plane API references this for the per-(tenant,
    /// model) override surface.
    tenant_header_name: String,
}

impl TokenizerSvc {
    pub fn new(tokenizer: Arc<Tokenizer>) -> Self {
        Self {
            tokenizer,
            shadow_worker: None,
            tenant_header_name: DEFAULT_TENANT_METADATA_HEADER.to_string(),
        }
    }

    /// Set the shadow worker handle. Returns Self for fluent chaining
    /// in main.rs's boot sequence.
    pub fn with_shadow_worker(mut self, h: ShadowWorkerHandle) -> Self {
        self.shadow_worker = Some(h);
        self
    }

    /// Override the tenant metadata header (default
    /// `x-spendguard-tenant-id`). Surfaced for tests + future
    /// multi-tenant routing.
    pub fn with_tenant_header(mut self, name: impl Into<String>) -> Self {
        self.tenant_header_name = name.into();
        self
    }
}

/// gRPC metadata header carrying the caller's tenant id. The default
/// matches the sidecar convention.
pub const DEFAULT_TENANT_METADATA_HEADER: &str = "x-spendguard-tenant-id";

#[tonic::async_trait]
impl TokenizerSvcTrait for TokenizerSvc {
    async fn tokenize(
        &self,
        request: Request<TokenizeRequest>,
    ) -> Result<Response<TokenizeResponse>, Status> {
        // SLICE_05: pull the tenant id from the gRPC metadata header
        // BEFORE consuming the Request. R2 B5: parse as UUID; invalid
        // UUIDs are rejected with InvalidArgument so misconfigured
        // callers fail closed. Absent / empty header yields None and
        // the shadow path is silently skipped for the request (test +
        // anonymous library form).
        let tenant_id: Option<uuid::Uuid> = match request
            .metadata()
            .get(&self.tenant_header_name)
            .and_then(|v| v.to_str().ok())
        {
            None => None,
            Some(s) if s.is_empty() => None,
            Some(s) => Some(uuid::Uuid::parse_str(s).map_err(|e| {
                Status::invalid_argument(format!(
                    "{}: invalid UUID `{s}`: {e}",
                    self.tenant_header_name
                ))
            })?),
        };

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

        let lib_req = validate_or_mint_request_id(lib_req)?;

        let encode_req = lib_req.clone();
        let tokenizer = Arc::clone(&self.tokenizer);
        let encode_task = tokio::task::spawn_blocking(move || tokenizer.tokenize(&encode_req));
        let lib_resp = match tokio::time::timeout(ENCODE_TIMEOUT, encode_task).await {
            Ok(Ok(Ok(resp))) => resp,
            Ok(Ok(Err(err))) => return Err(map_tokenizer_error(err)),
            Ok(Err(join_err)) => {
                return Err(Status::internal(format!(
                    "tokenizer encode task failed: {join_err}"
                )));
            }
            Err(_) => {
                ENCODE_TIMEOUT_TOTAL.fetch_add(1, Ordering::Relaxed);
                return Err(Status::deadline_exceeded("encode timeout exceeded"));
            }
        };

        // SLICE_05: fire-and-forget shadow event AFTER computing the
        // Tier 2 response. The send is non-blocking; the result is
        // intentionally ignored for hot-path latency, but R2 M5 wires
        // Prometheus counters so silent shadow drops surface in /metrics.
        // Only T2 results with a known kind go to the worker; T3 fallback
        // (Tier 3 heuristic) is excluded because there is no encoder to
        // compare against.
        //
        // R2 M2: chat-shape (messages array) is SKIPPED in SLICE_05.
        // Per-vendor message shape (Anthropic `Human:` / `Assistant:`
        // role markers, Gemini `contents[role]`) needs honest
        // round-tripping into the provider count_tokens schema; the
        // SLICE_05 flatten was an approximation that would generate
        // false drift alerts. Deferred to SLICE-extra; tracked as
        // GH issue.
        //
        // R2 B5: tenant_id must be Some(uuid) for the worker to dispatch
        // — anonymous callers (no header) yield None and the shadow
        // path is silently skipped.
        if let (Some(ref worker), Some(tenant_uuid)) = (&self.shadow_worker, tenant_id) {
            if let Some(kind) = encoder_kind_from_str(&lib_resp.kind) {
                // R2 M2: skip chat-shape requests; raw_text only.
                if lib_req.raw_text.is_empty() && !lib_req.messages.is_empty() {
                    crate::shadow::worker::SHADOW_SKIPPED_CHAT_SHAPE
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                } else {
                    let event = ShadowEvent {
                        tenant_id: tenant_uuid,
                        model: lib_req.model.clone(),
                        encoder_kind: kind,
                        t2_input_tokens: lib_resp.input_tokens,
                        t2_tokenizer_version_id: lib_resp.tokenizer_version_id.clone(),
                        raw_text: lib_req.raw_text.clone(),
                    };
                    match worker.try_send(event) {
                        Ok(()) => {}
                        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                            crate::shadow::worker::SHADOW_DROPPED_FULL
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                            crate::shadow::worker::SHADOW_WORKER_DEAD
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            warn!("shadow worker dead — drift detection offline");
                        }
                    }
                }
            }
        }

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

/// SLICE_05 — map the string `kind` the library returns to the
/// strongly-typed `EncoderKind` the shadow worker dispatches on. Tier 3
/// (HEURISTIC) returns None because there is no encoder to compare
/// against.
fn encoder_kind_from_str(kind: &str) -> Option<EncoderKind> {
    match kind {
        "OPENAI_TIKTOKEN" => Some(EncoderKind::OpenAi),
        "ANTHROPIC_BPE" => Some(EncoderKind::Anthropic),
        "GEMINI_BPE" => Some(EncoderKind::Gemini),
        "COHERE_BPE" => Some(EncoderKind::Cohere),
        "SENTENCEPIECE_LLAMA" => Some(EncoderKind::Llama),
        _ => None,
    }
}

fn validate_or_mint_request_id(
    req: spendguard_tokenizer::TokenizeRequest,
) -> Result<spendguard_tokenizer::TokenizeRequest, Status> {
    if req.request_id.is_empty() {
        return Ok(spendguard_tokenizer::TokenizeRequest {
            request_id: uuid::Uuid::now_v7().to_string(),
            ..req
        });
    }

    let parsed = uuid::Uuid::parse_str(&req.request_id).map_err(|e| {
        INVALID_REQUEST_ID_TOTAL.fetch_add(1, Ordering::Relaxed);
        Status::invalid_argument(format!("request_id must be a UUIDv7 or UUIDv4: {e}"))
    })?;

    match parsed.get_version_num() {
        7 => Ok(req),
        4 => {
            REQUEST_ID_V4_ACCEPTED_TOTAL.fetch_add(1, Ordering::Relaxed);
            warn!(
                request_id = %parsed,
                "tokenizer request_id UUIDv4 accepted for backward compatibility; prefer UUIDv7"
            );
            Ok(req)
        }
        other => {
            INVALID_REQUEST_ID_TOTAL.fetch_add(1, Ordering::Relaxed);
            Err(Status::invalid_argument(format!(
                "request_id must be UUIDv7 or UUIDv4, got UUIDv{other}"
            )))
        }
    }
}

// R2 M2: text_for_shadow flatten REMOVED. Chat-shape requests are
// skipped from shadow sampling in SLICE_05 because the naive flatten
// (role: content concatenation) does not match the per-vendor message
// shape Anthropic / Gemini count_tokens expects — producing false drift
// alerts. raw_text shadow paths continue. Honest per-vendor chat
// shadowing deferred to SLICE-extra (tracked as GH issue).

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

    // ── Round-2 fix M6 + Round-3 fix N3 / POST_GA_03 #114 — DoS tests ────
    // The field cap and protocol-layer `max_decoding_message_size`
    // both reference TOKENIZER_REQUEST_CAP_BYTES, so tests auto-track
    // the 4 MiB boundary.

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

    #[tokio::test]
    async fn tokenize_accepts_caller_supplied_uuidv7_request_id() {
        let s = svc();
        let req = Request::new(TokenizeRequest {
            model: "gpt-4o".to_string(),
            messages: vec![],
            raw_text: "hi".to_string(),
            request_id: uuid::Uuid::now_v7().to_string(),
        });
        let resp = s.tokenize(req).await.expect("uuidv7 ok").into_inner();
        assert_eq!(resp.tier, "T2");
    }

    #[tokio::test]
    async fn tokenize_accepts_uuidv4_request_id_for_backward_compat() {
        let s = svc();
        let req = Request::new(TokenizeRequest {
            model: "gpt-4o".to_string(),
            messages: vec![],
            raw_text: "hi".to_string(),
            request_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        });
        let resp = s.tokenize(req).await.expect("uuidv4 ok").into_inner();
        assert_eq!(resp.tier, "T2");
    }

    #[tokio::test]
    async fn tokenize_rejects_invalid_request_id() {
        let s = svc();
        let req = Request::new(TokenizeRequest {
            model: "gpt-4o".to_string(),
            messages: vec![],
            raw_text: "hi".to_string(),
            request_id: "tenant-a:copied-request-id".to_string(),
        });
        let err = s.tokenize(req).await.expect_err("invalid request_id");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err
            .message()
            .contains("request_id must be a UUIDv7 or UUIDv4"));
    }
}
