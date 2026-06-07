//! Minimal OpenAI ChatCompletion request / streaming-chunk shape.
//!
//! D18 SLICE 78 — same neutral pivot the D17 cursor_codec uses. The
//! codec needs a canonical neutral shape it can translate Cascade's
//! wire envelope into so the sidecar's `RequestDecision` /
//! `LlmCallPost` lane sees the same kind of input it gets from
//! `services/egress_proxy`.

use serde::{Deserialize, Serialize};

/// OpenAI Chat Completions request shape (subset).
///
/// Only the fields the SLICE 78 translator binds to are first-class;
/// everything else lives in [`Self::extra`] as a `serde_json::Value`
/// so unknown-vendor extensions survive the round-trip.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OpenAiChatRequest {
    /// Model identifier. Translator copies from
    /// [`crate::windsurf_proto::CascadeRequest::model_name`] verbatim.
    pub model: String,

    /// Chat history in OpenAI canonical order.
    pub messages: Vec<OpenAiMessage>,

    /// Cascade-supplied max_tokens hint. None when Cascade did not
    /// send one.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_tokens: Option<u32>,

    /// Always `Some(true)` in the SLICE 78 translation. Cascade's
    /// wire is server-streaming gRPC-Web, so the OpenAI request the
    /// codec derives is also a streaming request.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub stream: Option<bool>,

    /// Pass-through bucket for any OpenAI field the translator does
    /// not have a typed binding for (tools, response_format, seed,
    /// etc.).
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// A single OpenAI chat message (`role`, `content`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OpenAiMessage {
    /// Role string. Cascade uses the same vocabulary as OpenAI.
    pub role: String,

    /// Content payload. Cascade's wire envelope is a plain UTF-8
    /// string today.
    pub content: String,

    /// Pass-through for OpenAI-specific message fields.
    #[serde(flatten, default)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// One streaming chunk of an OpenAI Chat Completions response.
///
/// Cascade's wire emits one
/// [`crate::windsurf_proto::CascadeResponseDelta`] per gRPC-Web data
/// frame; the SLICE 78 translator maps each delta 1:1 to an
/// [`OpenAiChatResponseChunk`].
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OpenAiChatResponseChunk {
    /// Model that produced this chunk.
    pub model: String,

    /// Streaming-chunk choices array.
    pub choices: Vec<OpenAiChunkChoice>,

    /// Per-chunk usage block.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub usage: Option<OpenAiUsage>,

    /// Pass-through for OpenAI-specific chunk fields.
    #[serde(flatten, default)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// One delta entry in an [`OpenAiChatResponseChunk`].
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OpenAiChunkChoice {
    /// Choice index. SLICE 78 always emits 0.
    #[serde(default)]
    pub index: u32,

    /// Delta payload — the OpenAI Chat Completions streaming shape.
    pub delta: OpenAiChunkDelta,

    /// finish_reason — None on follow-up chunks, populated on the
    /// terminal chunk only.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub finish_reason: Option<String>,
}

/// A streaming delta — the per-chunk diff of the assistant reply.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OpenAiChunkDelta {
    /// Role marker on the first chunk; None on follow-ups.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub role: Option<String>,

    /// The string fragment appended in this chunk. Cascade's wire
    /// delta maps here directly.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub content: Option<String>,
}

/// Usage block. SLICE 78 fills `prompt_tokens` from Cascade's
/// `usage.input_tokens` and `completion_tokens` from `usage.output_tokens`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OpenAiUsage {
    /// Input tokens; 0 when not reported.
    #[serde(default)]
    pub prompt_tokens: u32,
    /// Output tokens; this is the field SLICE 78 commit lane reads.
    #[serde(default)]
    pub completion_tokens: u32,
    /// `prompt_tokens + completion_tokens`.
    #[serde(default)]
    pub total_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_minimal_request_json() {
        let req = OpenAiChatRequest {
            model: "gpt-4o".to_string(),
            messages: vec![OpenAiMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
                extra: Default::default(),
            }],
            max_tokens: Some(64),
            stream: Some(true),
            extra: Default::default(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let decoded: OpenAiChatRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, req);
    }

    #[test]
    fn extra_fields_survive_round_trip() {
        let json = serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"type": "function", "function": {"name": "lookup"}}],
            "stream": true
        });
        let decoded: OpenAiChatRequest = serde_json::from_value(json.clone()).unwrap();
        assert!(decoded.extra.contains_key("tools"));
        let encoded = serde_json::to_value(&decoded).unwrap();
        assert_eq!(encoded, json);
    }
}
