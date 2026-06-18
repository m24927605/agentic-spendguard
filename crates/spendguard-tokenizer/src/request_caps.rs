//! Shared request-shape caps for the tokenizer (DoS defense-in-depth).
//!
//! ## Why this module exists
//!
//! Per `tokenizer-service-spec-v1alpha1.md` §2.1 the tokenizer ships in
//! two co-existing forms — the gRPC service (`services/tokenizer/`) and
//! this in-process library (linked directly by `egress_proxy` /
//! `envoy_extproc` on the hot path). Round-2 fix M6 + Round-3 fix N3
//! added per-field size caps so a buggy / hostile caller cannot pressure
//! the encoder cache by sending megabyte-scale text that allocates a BPE
//! encode buffer proportional to input size.
//!
//! Historically those caps + their enforcement lived **only** in
//! `TokenizerSvc::tokenize` (the tonic handler). The library entry point
//! [`crate::Tokenizer::tokenize`] went straight to dispatch + encode with
//! zero length validation, so the documented invariant — "the in-process
//! library form defends itself with the same bound" — was false. This
//! module makes the bound real: it owns the canonical cap constants and a
//! single [`validate`] helper that BOTH forms call, keeping the gRPC and
//! library bounds in lockstep so a cap change cannot drift between them.
//!
//! ## Fail-closed contract (LOAD-BEARING)
//!
//! [`validate`] returns [`TokenizerError::RequestTooLarge`] — a **distinct**
//! variant, not [`TokenizerError::EncoderInternal`] — precisely so that
//! callers can recognise an oversized-request rejection and map it to a
//! fail-closed decision (DENY / large-request guard) rather than silently
//! treating it like any other tokenizer error.
//!
//! This matters because at least one in-process caller
//! (`egress_proxy/src/decision.rs`) historically mapped *any* tokenizer
//! `Err` to a permissive `TokenizeResponse { tier: "T3", .. }` whose
//! `input_tokens` defaults to **0** — a fail-OPEN, budget-under-counting
//! path. If an oversized attacker prompt rejected here collapsed into a
//! ~0-token estimate, the cap would *weaken* the budget guard instead of
//! strengthening it. The distinct variant + [`TokenizerError::is_request_too_large`]
//! accessor let that caller fail closed on this case while keeping its
//! existing (rare, bug-only) fallback for genuine internal errors.

use crate::{TokenizeRequest, TokenizerError};

/// Shared decoded-message byte cap for the gRPC protocol layer and the
/// field-validation layer. 4 MiB so realistic multi-turn prompts traverse
/// the sidecar/tokenizer path while oversized frames are rejected before
/// any encoder work begins (POST_GA_03 / #114).
///
/// This is the single source of truth; the gRPC service form
/// (`services/tokenizer`) and its `max_decoding_message_size` protocol cap
/// reference this value so all three bounds stay in lockstep.
pub const TOKENIZER_REQUEST_CAP_BYTES: usize = 4 << 20;

/// Max bytes accepted in the `model` field. Real-world model strings are
/// < 64 chars; 256 leaves runway for vendor prefixes.
pub const MAX_MODEL_LEN: usize = 256;

/// Max bytes accepted in `raw_text` (the text-completion shape). Aligned
/// with the protocol-layer `max_decoding_message_size` so the field
/// validation error surface is reachable.
pub const MAX_RAW_TEXT_LEN: usize = TOKENIZER_REQUEST_CAP_BYTES;

/// Max bytes per individual `message.content`. See [`MAX_RAW_TEXT_LEN`].
pub const MAX_MESSAGE_CONTENT_LEN: usize = TOKENIZER_REQUEST_CAP_BYTES;

/// Max number of `Message` elements in the chat-shape array.
pub const MAX_MESSAGES: usize = 1_000;

/// Validate a request against the shared per-field size caps.
///
/// Returns [`TokenizerError::RequestTooLarge`] on the first violated cap.
/// All comparisons are on **byte length** (`.len()`), matching the gRPC
/// handler so the two forms reject byte-for-byte identical inputs. This is
/// pure (no allocation) and runs before any encoder buffer is allocated.
///
/// Order matches the gRPC handler: model, raw_text, message count, then
/// per-message content. The first failing field wins so error messages
/// are deterministic for callers metricking on the offending field.
pub fn validate(req: &TokenizeRequest) -> Result<(), TokenizerError> {
    if req.model.len() > MAX_MODEL_LEN {
        return Err(TokenizerError::RequestTooLarge {
            field: "model",
            actual_bytes: req.model.len(),
            limit_bytes: MAX_MODEL_LEN,
        });
    }
    if req.raw_text.len() > MAX_RAW_TEXT_LEN {
        return Err(TokenizerError::RequestTooLarge {
            field: "raw_text",
            actual_bytes: req.raw_text.len(),
            limit_bytes: MAX_RAW_TEXT_LEN,
        });
    }
    if req.messages.len() > MAX_MESSAGES {
        return Err(TokenizerError::RequestTooLarge {
            field: "messages",
            actual_bytes: req.messages.len(),
            limit_bytes: MAX_MESSAGES,
        });
    }
    for m in &req.messages {
        if m.content.len() > MAX_MESSAGE_CONTENT_LEN {
            return Err(TokenizerError::RequestTooLarge {
                field: "messages.content",
                actual_bytes: m.content.len(),
                limit_bytes: MAX_MESSAGE_CONTENT_LEN,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Message;

    #[test]
    fn at_cap_request_is_accepted() {
        let req = TokenizeRequest {
            model: "x".repeat(MAX_MODEL_LEN),
            raw_text: "y".repeat(MAX_RAW_TEXT_LEN),
            messages: vec![Message {
                role: "user".to_string(),
                content: "z".repeat(MAX_MESSAGE_CONTENT_LEN),
                tool_calls: vec![],
            }],
            request_id: String::new(),
        };
        assert!(validate(&req).is_ok());
    }

    #[test]
    fn oversized_model_is_rejected_as_request_too_large() {
        let req = TokenizeRequest {
            model: "x".repeat(MAX_MODEL_LEN + 1),
            ..Default::default()
        };
        let err = validate(&req).expect_err("oversized model must reject");
        assert!(err.is_request_too_large());
        match err {
            TokenizerError::RequestTooLarge { field, .. } => assert_eq!(field, "model"),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn oversized_raw_text_is_rejected() {
        let req = TokenizeRequest {
            raw_text: "y".repeat(MAX_RAW_TEXT_LEN + 1),
            ..Default::default()
        };
        let err = validate(&req).expect_err("oversized raw_text must reject");
        assert!(err.is_request_too_large());
    }

    #[test]
    fn too_many_messages_is_rejected() {
        let req = TokenizeRequest {
            messages: vec![Message::default(); MAX_MESSAGES + 1],
            ..Default::default()
        };
        let err = validate(&req).expect_err("too many messages must reject");
        assert!(err.is_request_too_large());
        match err {
            TokenizerError::RequestTooLarge { field, .. } => assert_eq!(field, "messages"),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn oversized_message_content_is_rejected() {
        let req = TokenizeRequest {
            messages: vec![Message {
                role: "user".to_string(),
                content: "z".repeat(MAX_MESSAGE_CONTENT_LEN + 1),
                tool_calls: vec![],
            }],
            ..Default::default()
        };
        let err = validate(&req).expect_err("oversized content must reject");
        assert!(err.is_request_too_large());
        match err {
            TokenizerError::RequestTooLarge { field, .. } => {
                assert_eq!(field, "messages.content")
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn empty_request_is_accepted() {
        assert!(validate(&TokenizeRequest::default()).is_ok());
    }
}
