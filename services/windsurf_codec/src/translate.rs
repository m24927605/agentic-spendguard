//! Cascade ↔ OpenAI canonical translation.
//!
//! D18 SLICE 78 translator. Pure functions, no I/O. Pivots between
//! the Cascade wire envelope (proto3 messages from
//! [`crate::envelope`]) and the canonical OpenAI shape the sidecar
//! and the upstream provider both understand.

use crate::openai_models::{OpenAiChatRequest, OpenAiChatResponseChunk, OpenAiMessage};
use crate::windsurf_proto::{CascadeRequest, CascadeResponseDelta};

/// Translate a [`CascadeRequest`] into the canonical OpenAI Chat
/// Completions request shape.
///
/// Pure function — no I/O, no side effects. The output is what the
/// SLICE 78 MITM session feeds into the sidecar `RequestDecision`
/// pipeline.
pub fn cascade_request_to_openai(req: &CascadeRequest) -> OpenAiChatRequest {
    let mut messages: Vec<OpenAiMessage> = Vec::with_capacity(req.messages.len());

    for m in &req.messages {
        messages.push(OpenAiMessage {
            role: m.role.clone(),
            content: m.content.clone(),
            extra: Default::default(),
        });
    }

    OpenAiChatRequest {
        model: req.model_name.clone(),
        messages,
        max_tokens: req.max_tokens,
        stream: Some(true),
        extra: Default::default(),
    }
}

/// Translate an upstream OpenAI streaming chunk back into a Cascade
/// wire delta so the IDE client can decode it.
pub fn openai_chunk_to_cascade(chunk: &OpenAiChatResponseChunk) -> CascadeResponseDelta {
    let first = chunk.choices.first();

    let text_chunk = first.and_then(|c| c.delta.content.clone());
    let finish_reason = first.and_then(|c| c.finish_reason.clone());

    let usage = chunk
        .usage
        .as_ref()
        .map(|u| crate::windsurf_proto::CascadeUsage {
            input_tokens: u.prompt_tokens,
            output_tokens: u.completion_tokens,
        });

    CascadeResponseDelta {
        model_name: chunk.model.clone(),
        text_chunk,
        finish_reason,
        usage,
        cascade_wire_version: Some("cascade.v2.0".to_string()),
    }
}

/// Helper: extract the cumulative output-token count from a terminal
/// Cascade delta so the SLICE 78 commit lane can build the
/// `estimated_amount_atomic` body of `LlmCallPostPayload`. Returns 0
/// when Cascade didn't stamp usage.
pub fn extract_cascade_output_tokens(delta: &CascadeResponseDelta) -> u32 {
    delta.usage.as_ref().map(|u| u.output_tokens).unwrap_or(0)
}

/// Helper: mirror of [`extract_cascade_output_tokens`] for the OpenAI
/// shape.
pub fn extract_openai_output_tokens(chunk: &OpenAiChatResponseChunk) -> u32 {
    chunk
        .usage
        .as_ref()
        .map(|u| u.completion_tokens)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai_models::{OpenAiChunkChoice, OpenAiChunkDelta, OpenAiUsage};
    use crate::windsurf_proto::{CascadeMessage, CascadeUsage};

    fn user(content: &str) -> CascadeMessage {
        CascadeMessage {
            role: "user".to_string(),
            content: content.to_string(),
        }
    }
    fn assistant(content: &str) -> CascadeMessage {
        CascadeMessage {
            role: "assistant".to_string(),
            content: content.to_string(),
        }
    }

    /// (1) Minimal request: one user turn.
    #[test]
    fn minimal_request_translates() {
        let cascade_req = CascadeRequest {
            messages: vec![user("hello")],
            model_name: "gpt-4o".to_string(),
            max_tokens: Some(64),
            tool_declarations: vec![],
            workspace_id: None,
            cascade_wire_version: Some("cascade.v2.0".into()),
        };
        let openai = cascade_request_to_openai(&cascade_req);
        assert_eq!(openai.model, "gpt-4o");
        assert_eq!(openai.messages.len(), 1);
        assert_eq!(openai.messages[0].role, "user");
        assert_eq!(openai.messages[0].content, "hello");
        assert_eq!(openai.max_tokens, Some(64));
        assert_eq!(openai.stream, Some(true));
    }

    /// (2) Multi-turn translates verbatim.
    #[test]
    fn multi_turn_request_preserves_order() {
        let cascade_req = CascadeRequest {
            messages: vec![user("a"), assistant("b"), user("c")],
            model_name: "claude-3.5-sonnet".to_string(),
            max_tokens: None,
            tool_declarations: vec![],
            workspace_id: None,
            cascade_wire_version: Some("cascade.v2.0".into()),
        };
        let openai = cascade_request_to_openai(&cascade_req);
        assert_eq!(openai.messages.len(), 3);
        assert_eq!(openai.messages[0].role, "user");
        assert_eq!(openai.messages[1].role, "assistant");
        assert_eq!(openai.messages[2].role, "user");
    }

    /// (3) Streaming delta → Cascade with usage.
    #[test]
    fn openai_chunk_translates_to_cascade_with_usage() {
        let chunk = OpenAiChatResponseChunk {
            model: "gpt-4o".to_string(),
            choices: vec![OpenAiChunkChoice {
                index: 0,
                delta: OpenAiChunkDelta {
                    role: None,
                    content: Some("hello".to_string()),
                },
                finish_reason: Some("stop".to_string()),
            }],
            usage: Some(OpenAiUsage {
                prompt_tokens: 5,
                completion_tokens: 17,
                total_tokens: 22,
            }),
            extra: Default::default(),
        };
        let cascade = openai_chunk_to_cascade(&chunk);
        assert_eq!(cascade.text_chunk.as_deref(), Some("hello"));
        assert_eq!(cascade.finish_reason.as_deref(), Some("stop"));
        let usage = cascade.usage.unwrap();
        assert_eq!(usage.input_tokens, 5);
        assert_eq!(usage.output_tokens, 17);
    }

    /// (4) Output token extraction sentinel zero.
    #[test]
    fn extract_cascade_tokens_sentinel_zero_when_unset() {
        let delta = CascadeResponseDelta {
            model_name: String::new(),
            text_chunk: None,
            finish_reason: Some("stop".to_string()),
            usage: None,
            cascade_wire_version: Some("cascade.v2.0".into()),
        };
        assert_eq!(extract_cascade_output_tokens(&delta), 0);
    }

    /// (5) Output token extraction populates.
    #[test]
    fn extract_cascade_tokens_populated() {
        let delta = CascadeResponseDelta {
            model_name: String::new(),
            text_chunk: None,
            finish_reason: Some("stop".to_string()),
            usage: Some(CascadeUsage {
                input_tokens: 7,
                output_tokens: 42,
            }),
            cascade_wire_version: Some("cascade.v2.0".into()),
        };
        assert_eq!(extract_cascade_output_tokens(&delta), 42);
    }

    /// (6) Empty OpenAI choices → empty cascade text_chunk.
    #[test]
    fn empty_openai_choices_yields_no_text_chunk() {
        let chunk = OpenAiChatResponseChunk {
            model: "gpt-4o".to_string(),
            choices: vec![],
            usage: None,
            extra: Default::default(),
        };
        let cascade = openai_chunk_to_cascade(&chunk);
        assert_eq!(cascade.text_chunk, None);
        assert_eq!(cascade.finish_reason, None);
    }
}
