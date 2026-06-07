//! Minimal OpenAI ChatCompletion request / streaming-chunk shape.
//!
//! D17 SLICE 5 ([`design.md`](../../docs/specs/coverage/D17_cursor_mitm/design.md)
//! §7 translator + [`implementation.md`](../../docs/specs/coverage/D17_cursor_mitm/implementation.md)
//! §6): the codec needs a canonical neutral shape it can translate
//! Cursor's wire envelope into so the sidecar's
//! `RequestDecision` / `LlmCallPost` lane sees the same kind of input
//! it gets from `services/egress_proxy`. The simplest neutral shape is
//! the OpenAI Chat Completions request, because Cursor's own envelope
//! is already a near-clone of it (per the SLICE 3 RECON.md observations).
//!
//! ## Why hand-rolled
//!
//! We deliberately do NOT pull in `async-openai` or `openai-api-rs`:
//!
//! * They have a wide and rapidly moving surface (assistants, vector
//!   stores, batches, etc.) that has nothing to do with the codec's
//!   shrink-wrapped reserve / commit / release lane.
//! * They impose a `reqwest` dep transitively, which we don't want at
//!   crate-default level (`mitm` feature handles its own client).
//! * They evolve their internal types per release; keeping the shape
//!   pinned in this crate prevents a Cursor codec regression every
//!   time async-openai churns.
//!
//! We treat additive vendor fields the same way the translator treats
//! Cursor's unknown proto fields: stash them in a [`serde_json::Value`]
//! field so the round-trip preserves them byte-for-byte where possible.
//!
//! ## Wire-shape coverage
//!
//! The SLICE 5 translator only binds to the fields Cursor's envelope
//! actually carries today (per
//! [`design.md`](../../docs/specs/coverage/D17_cursor_mitm/design.md)
//! §5 capture observations + the proto in `src/proto/cursor.proto`):
//!
//! * `model: String`
//! * `messages: Vec<Message>` where `Message = {role, content}`
//! * `system: Option<String>` (Cursor sometimes splits this out — when
//!   present, the translator prepends it as the first `role=system`
//!   message in the OpenAI shape, per OpenAI's canonical convention)
//! * `max_tokens: Option<u32>`
//! * `temperature: Option<f32>`
//! * `stream: bool` (we always set this to `true` on the translated
//!   request because Cursor's wire is server-streaming Connect-RPC —
//!   the translator never produces a unary OpenAI request)
//!
//! All other OpenAI fields are passed through via [`OpenAiChatRequest::extra`]
//! and survive the translation round-trip transparently.

use serde::{Deserialize, Serialize};

/// OpenAI Chat Completions request shape (subset).
///
/// Only the fields the SLICE 5 translator binds to are first-class;
/// everything else lives in [`Self::extra`] as a `serde_json::Value` so
/// unknown-vendor extensions survive the round-trip. The translator
/// emits this when it converts a [`crate::cursor_proto::CursorChatRequest`]
/// to canonical OpenAI shape (and back).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OpenAiChatRequest {
    /// Model identifier. Translator copies from
    /// [`crate::cursor_proto::CursorChatRequest::model`] verbatim — model
    /// re-mapping (e.g. Cursor-internal `cursor-small` → `gpt-4o-mini`)
    /// is the egress proxy's job, not the codec's, because SLICE 6
    /// reserve/commit needs to keep the original Cursor model id for
    /// the audit chain.
    pub model: String,

    /// Chat history in OpenAI canonical order. When the Cursor envelope
    /// has a top-level `system` field, the translator prepends a
    /// `role=system` message here so the OpenAI shape matches the
    /// canonical `chat.completions` convention.
    pub messages: Vec<OpenAiMessage>,

    /// Cursor-supplied max_tokens hint. None when Cursor did not send one.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_tokens: Option<u32>,

    /// Cursor-supplied temperature. None when Cursor did not send one.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub temperature: Option<f32>,

    /// Always `Some(true)` in the SLICE 5 translation. Cursor's wire is
    /// server-streaming Connect-RPC, so the OpenAI request the codec
    /// derives is also a streaming request. We keep the field optional
    /// (Some/None) to preserve byte-identity on round-trip — a
    /// downstream test that flips streaming off (re-encode path) sees
    /// `None`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub stream: Option<bool>,

    /// Pass-through bucket for any OpenAI field the translator does not
    /// have a typed binding for (tools, response_format, seed, etc.).
    /// Serialised inline at the OpenAI request root by serde's `flatten`.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// A single OpenAI chat message (`role`, `content`).
///
/// SLICE 5 ships a string `content`; OpenAI's vision content arrays
/// (multimodal) ride in `extra` and survive the round-trip via the
/// flatten pass-through.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OpenAiMessage {
    /// Role string. Cursor uses the same vocabulary as OpenAI
    /// (`system` | `user` | `assistant` | `tool`).
    pub role: String,

    /// Content payload. Cursor's wire envelope is a plain UTF-8 string
    /// today; SLICE 8 capture will refine if multimodal arrays appear.
    pub content: String,

    /// Pass-through for OpenAI-specific message fields the translator
    /// doesn't bind (`name`, `tool_calls`, `tool_call_id`, etc.).
    #[serde(flatten, default)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// One streaming chunk of an OpenAI Chat Completions response.
///
/// Cursor's wire emits one [`crate::cursor_proto::CursorChatResponseChunk`]
/// per Connect-RPC data frame; the SLICE 5 translator maps each chunk
/// 1:1 to an [`OpenAiChatResponseChunk`]. The translator is reversible —
/// the SLICE 6 re-encode path takes an `OpenAiChatResponseChunk` (from
/// the upstream provider, in the demo path) and re-emits a Cursor chunk.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OpenAiChatResponseChunk {
    /// Model that produced this chunk (mirrored from the request; may
    /// be empty on follow-up chunks because Cursor only stamps it on
    /// the first chunk in capture observation).
    pub model: String,

    /// Streaming-chunk choices array. OpenAI canonical shape; SLICE 5
    /// always emits exactly one choice (matching the Cursor wire
    /// observation that has exactly one delta per chunk).
    pub choices: Vec<OpenAiChunkChoice>,

    /// Per-chunk usage block. OpenAI's Chat Completions API only emits
    /// this on the terminal chunk when `stream_options.include_usage =
    /// true`; the SLICE 5 translator carries Cursor's
    /// `cumulative_output_tokens` here so the SLICE 6 commit lane can
    /// extract the actuals without parsing the rest of the stream.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub usage: Option<OpenAiUsage>,

    /// Pass-through for OpenAI-specific chunk fields (`object`, `created`,
    /// `system_fingerprint`, etc.).
    #[serde(flatten, default)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// One delta entry in an [`OpenAiChatResponseChunk`].
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OpenAiChunkChoice {
    /// Choice index. SLICE 5 always emits 0 (single-choice).
    #[serde(default)]
    pub index: u32,

    /// Delta payload — the OpenAI Chat Completions streaming shape.
    pub delta: OpenAiChunkDelta,

    /// finish_reason — None on follow-up chunks, populated on the
    /// terminal chunk only (matches Cursor's wire observation).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub finish_reason: Option<String>,
}

/// A streaming delta — the per-chunk diff of the assistant reply.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OpenAiChunkDelta {
    /// Role marker on the first chunk; None on follow-ups.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub role: Option<String>,

    /// The string fragment appended in this chunk. Cursor's wire delta
    /// maps here directly.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub content: Option<String>,
}

/// Usage block. SLICE 5 fills `output_tokens` from
/// `CursorChatResponseChunk::cumulative_output_tokens`; the other
/// fields default to 0 because Cursor's terminal chunk doesn't carry
/// input-side token counts (egress_proxy keeps its own input tally).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OpenAiUsage {
    /// Input tokens; 0 when not reported.
    #[serde(default)]
    pub prompt_tokens: u32,
    /// Output tokens; this is the field SLICE 6 commit lane reads.
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
            model: "gpt-4o-mini".to_string(),
            messages: vec![OpenAiMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
                extra: Default::default(),
            }],
            max_tokens: Some(64),
            temperature: Some(0.2),
            stream: Some(true),
            extra: Default::default(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let decoded: OpenAiChatRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, req);
    }

    #[test]
    fn extra_fields_survive_round_trip() {
        // Tool calls etc. ride in `extra`; verify they survive serde.
        let json = serde_json::json!({
            "model": "gpt-4o",
            "messages": [
                {"role": "user", "content": "hi", "name": "alice"}
            ],
            "tools": [{"type": "function", "function": {"name": "lookup"}}],
            "stream": true
        });
        let decoded: OpenAiChatRequest = serde_json::from_value(json.clone()).unwrap();
        assert!(decoded.extra.contains_key("tools"));
        let encoded = serde_json::to_value(&decoded).unwrap();
        assert_eq!(encoded, json);
    }

    #[test]
    fn chunk_with_usage_round_trips() {
        let chunk = OpenAiChatResponseChunk {
            model: "gpt-4o-mini".to_string(),
            choices: vec![OpenAiChunkChoice {
                index: 0,
                delta: OpenAiChunkDelta {
                    role: None,
                    content: Some("hello".to_string()),
                },
                finish_reason: Some("stop".to_string()),
            }],
            usage: Some(OpenAiUsage {
                prompt_tokens: 0,
                completion_tokens: 12,
                total_tokens: 12,
            }),
            extra: Default::default(),
        };
        let json = serde_json::to_string(&chunk).unwrap();
        let decoded: OpenAiChatResponseChunk = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, chunk);
    }
}
