//! Cursor ↔ OpenAI canonical translation.
//!
//! D17 SLICE 5 ([`design.md`](../../docs/specs/coverage/D17_cursor_mitm/design.md)
//! §7 translator). Pure functions, no I/O. The translator is the
//! pivot point between the wire envelope (proto3 messages decoded by
//! [`crate::envelope`]) and the canonical OpenAI shape the sidecar
//! and the upstream provider both understand.
//!
//! ## Directionality
//!
//! * [`cursor_request_to_openai`] — runs when SpendGuard observes a
//!   Cursor IDE Agent request on `api.cursor.sh`. We translate to the
//!   OpenAI Chat Completions request shape so the SLICE 6 MITM session
//!   state machine can feed it to the sidecar's `RequestDecision`
//!   path with the same body-signature derivation egress_proxy uses.
//! * [`openai_chunk_to_cursor`] — runs in the response path when the
//!   demo forwards to an upstream OpenAI-compatible endpoint instead
//!   of `api.cursor.sh`. We rebuild a Cursor wire chunk so the IDE
//!   client decodes a familiar envelope and stays happy with the
//!   stream framing.
//!
//! ## Lossiness contract
//!
//! Translation is *intentionally lossy in the typed surface and
//! lossless in the raw surface*:
//!
//! * Typed fields the translator binds to (`model`, `messages`,
//!   `system`, `max_tokens`, `temperature`) carry semantic meaning
//!   into the OpenAI shape — they're what the sidecar and upstream
//!   provider read.
//! * Unknown fields on either side survive via the
//!   [`OpenAiChatRequest::extra`] / [`OpenAiMessage::extra`] +
//!   [`OpenAiChatResponseChunk::extra`] pass-through buckets and via
//!   prost's unknown-field tolerance on decode (proto3 default —
//!   round-tripping unmodified fields preserves their bytes). This is
//!   the SLICE 7 byte-for-byte preservation contract restated at the
//!   translator boundary.
//!
//! ## System prompt handling
//!
//! Cursor's wire envelope has both a top-level
//! [`crate::cursor_proto::CursorChatRequest::system`] AND a possible
//! `role=system` entry in `messages`. OpenAI canonical convention is
//! ONE leading system message in `messages`. The translator collapses
//! these per the rules in [`cursor_request_to_openai`]:
//!
//! 1. If `system` is present and the first messages-array entry is NOT
//!    already `role=system`, the translator prepends a synthetic
//!    `role=system` message with the `system` field's value.
//! 2. If `system` is present and the first messages-array entry IS
//!    `role=system`, the wire `system` is dropped (Cursor's envelope
//!    is ambiguous when both fields populated — we trust the
//!    messages-array path because OpenAI canonical wins).
//! 3. If `system` is absent, the messages array is passed through
//!    verbatim.

use crate::cursor_proto::{CursorChatRequest, CursorChatResponseChunk, Message as CursorMessage};
use crate::openai_models::{OpenAiChatRequest, OpenAiChatResponseChunk, OpenAiMessage};

/// Translate a [`CursorChatRequest`] (decoded wire envelope) into the
/// canonical OpenAI Chat Completions request shape.
///
/// Pure function — no I/O, no side effects. The output is what the
/// SLICE 6 MITM session feeds into the sidecar `RequestDecision`
/// pipeline and (in the demo) the upstream OpenAI-compatible
/// endpoint.
///
/// ## System prompt rules
///
/// See module-level docs for the exact precedence between
/// [`CursorChatRequest::system`] and a leading `role=system` entry in
/// [`CursorChatRequest::messages`].
pub fn cursor_request_to_openai(req: &CursorChatRequest) -> OpenAiChatRequest {
    let mut messages: Vec<OpenAiMessage> = Vec::with_capacity(req.messages.len() + 1);

    let already_has_leading_system = req
        .messages
        .first()
        .map(|m| m.role == "system")
        .unwrap_or(false);

    if let Some(system) = req.system.as_deref() {
        if !already_has_leading_system {
            messages.push(OpenAiMessage {
                role: "system".to_string(),
                content: system.to_string(),
                extra: Default::default(),
            });
        }
    }

    for m in &req.messages {
        messages.push(OpenAiMessage {
            role: m.role.clone(),
            content: m.content.clone(),
            extra: Default::default(),
        });
    }

    OpenAiChatRequest {
        model: req.model.clone(),
        messages,
        max_tokens: req.max_tokens,
        temperature: req.temperature,
        stream: Some(true),
        extra: Default::default(),
    }
}

/// Translate an upstream OpenAI streaming chunk back into a Cursor
/// wire chunk so the IDE client can decode it.
///
/// The reverse of [`cursor_request_to_openai`]'s direction. SLICE 6
/// uses this on the response path when the demo bridges to an upstream
/// OpenAI-compatible endpoint instead of a real Cursor backend.
///
/// ## Lossy fields
///
/// * Only the first `choices[0].delta.content` survives — Cursor's wire
///   has exactly one delta per chunk per the SLICE 3 RECON.md.
/// * `usage.completion_tokens` → `cumulative_output_tokens` if the
///   chunk carries a usage block (terminal chunk on OpenAI's stream
///   with `stream_options.include_usage = true`); otherwise the field
///   is left as `None`.
/// * Per-choice `finish_reason` is mapped to the chunk-level
///   `finish_reason` (Cursor's terminal-chunk marker).
pub fn openai_chunk_to_cursor(chunk: &OpenAiChatResponseChunk) -> CursorChatResponseChunk {
    let first = chunk.choices.first();

    let delta = first
        .and_then(|c| c.delta.content.clone())
        .unwrap_or_default();

    let finish_reason = first.and_then(|c| c.finish_reason.clone());

    let cumulative_output_tokens = chunk.usage.as_ref().map(|u| u.completion_tokens);

    CursorChatResponseChunk {
        model: chunk.model.clone(),
        delta,
        finish_reason,
        cumulative_output_tokens,
    }
}

/// Helper: extract the cumulative output-token count from a terminal
/// Cursor chunk so the SLICE 6 commit lane can build the
/// `estimated_amount_atomic` body of `LlmCallPostPayload`. Returns 0
/// when Cursor didn't stamp the count.
pub fn extract_cursor_output_tokens(chunk: &CursorChatResponseChunk) -> u32 {
    chunk.cumulative_output_tokens.unwrap_or(0)
}

/// Helper: mirror of [`extract_cursor_output_tokens`] for the OpenAI
/// shape. Reads `usage.completion_tokens` if present, falling back to
/// counting the chunk-deltas (callers track that themselves).
pub fn extract_openai_output_tokens(chunk: &OpenAiChatResponseChunk) -> u32 {
    chunk
        .usage
        .as_ref()
        .map(|u| u.completion_tokens)
        .unwrap_or(0)
}

/// Round-trip the Cursor → OpenAI → Cursor (model + messages only —
/// stream / max_tokens / temperature are lossy by construction).
///
/// Used by the SLICE 5 round-trip tests and by the SLICE 7 re-encode
/// tests as a coarse sanity check. The byte-level preservation
/// contract lives in [`crate::reencode`].
#[allow(dead_code)]
pub(crate) fn round_trip_messages(req: &CursorChatRequest) -> Vec<CursorMessage> {
    let openai = cursor_request_to_openai(req);
    openai
        .messages
        .into_iter()
        .map(|m| CursorMessage {
            role: m.role,
            content: m.content,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai_models::{OpenAiChunkChoice, OpenAiChunkDelta, OpenAiUsage};

    fn user(content: &str) -> CursorMessage {
        CursorMessage {
            role: "user".to_string(),
            content: content.to_string(),
        }
    }
    fn assistant(content: &str) -> CursorMessage {
        CursorMessage {
            role: "assistant".to_string(),
            content: content.to_string(),
        }
    }
    fn system_msg(content: &str) -> CursorMessage {
        CursorMessage {
            role: "system".to_string(),
            content: content.to_string(),
        }
    }

    /// (1) Minimal request: no system, one user turn.
    #[test]
    fn minimal_request_no_system() {
        let cursor_req = CursorChatRequest {
            messages: vec![user("hello")],
            model: "gpt-4o-mini".to_string(),
            system: None,
            max_tokens: Some(64),
            temperature: Some(0.5),
        };
        let openai = cursor_request_to_openai(&cursor_req);
        assert_eq!(openai.model, "gpt-4o-mini");
        assert_eq!(openai.messages.len(), 1);
        assert_eq!(openai.messages[0].role, "user");
        assert_eq!(openai.messages[0].content, "hello");
        assert_eq!(openai.max_tokens, Some(64));
        assert_eq!(openai.temperature, Some(0.5));
        assert_eq!(openai.stream, Some(true));
    }

    /// (2) System field present, no leading system in messages → prepend.
    #[test]
    fn system_field_prepends_when_messages_lacks_leading_system() {
        let cursor_req = CursorChatRequest {
            messages: vec![user("hello"), assistant("hi"), user("again")],
            model: "claude-3.5-sonnet".to_string(),
            system: Some("you are terse".to_string()),
            max_tokens: None,
            temperature: None,
        };
        let openai = cursor_request_to_openai(&cursor_req);
        assert_eq!(openai.messages.len(), 4);
        assert_eq!(openai.messages[0].role, "system");
        assert_eq!(openai.messages[0].content, "you are terse");
        assert_eq!(openai.messages[1].role, "user");
        assert_eq!(openai.messages[1].content, "hello");
    }

    /// (3) System field present AND leading system in messages → drop the field.
    #[test]
    fn leading_system_in_messages_wins_over_wire_system_field() {
        let cursor_req = CursorChatRequest {
            messages: vec![system_msg("from messages"), user("hi")],
            model: "gpt-4o".to_string(),
            system: Some("from field — should be dropped".to_string()),
            max_tokens: None,
            temperature: None,
        };
        let openai = cursor_request_to_openai(&cursor_req);
        assert_eq!(openai.messages.len(), 2);
        assert_eq!(openai.messages[0].role, "system");
        assert_eq!(openai.messages[0].content, "from messages");
        assert_eq!(openai.messages[1].role, "user");
    }

    /// (4) Empty messages list with system field still prepends.
    #[test]
    fn empty_messages_with_system_yields_single_system_message() {
        let cursor_req = CursorChatRequest {
            messages: vec![],
            model: "gpt-4o".to_string(),
            system: Some("alone".to_string()),
            max_tokens: None,
            temperature: None,
        };
        let openai = cursor_request_to_openai(&cursor_req);
        assert_eq!(openai.messages.len(), 1);
        assert_eq!(openai.messages[0].role, "system");
        assert_eq!(openai.messages[0].content, "alone");
    }

    /// (5) Streaming chunk → Cursor: usage + delta + finish_reason.
    #[test]
    fn openai_chunk_translates_to_cursor() {
        let chunk = OpenAiChatResponseChunk {
            model: "gpt-4o-mini".to_string(),
            choices: vec![OpenAiChunkChoice {
                index: 0,
                delta: OpenAiChunkDelta {
                    role: Some("assistant".to_string()),
                    content: Some("hello ".to_string()),
                },
                finish_reason: None,
            }],
            usage: None,
            extra: Default::default(),
        };
        let cursor = openai_chunk_to_cursor(&chunk);
        assert_eq!(cursor.model, "gpt-4o-mini");
        assert_eq!(cursor.delta, "hello ");
        assert_eq!(cursor.finish_reason, None);
        assert_eq!(cursor.cumulative_output_tokens, None);
    }

    /// (6) Terminal OpenAI chunk → Cursor terminal chunk.
    #[test]
    fn terminal_openai_chunk_carries_finish_reason_and_usage() {
        let chunk = OpenAiChatResponseChunk {
            model: "gpt-4o-mini".to_string(),
            choices: vec![OpenAiChunkChoice {
                index: 0,
                delta: OpenAiChunkDelta {
                    role: None,
                    content: None,
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
        let cursor = openai_chunk_to_cursor(&chunk);
        assert_eq!(cursor.delta, "");
        assert_eq!(cursor.finish_reason.as_deref(), Some("stop"));
        assert_eq!(cursor.cumulative_output_tokens, Some(17));
    }

    /// (7) Round-trip: Cursor → OpenAI → semantic-equivalent Cursor.
    #[test]
    fn round_trip_messages_preserves_user_turns() {
        let cursor_req = CursorChatRequest {
            messages: vec![user("a"), assistant("b"), user("c")],
            model: "gpt-4o".to_string(),
            system: None,
            max_tokens: None,
            temperature: None,
        };
        let round_tripped = round_trip_messages(&cursor_req);
        assert_eq!(round_tripped.len(), 3);
        assert_eq!(round_tripped[0].role, "user");
        assert_eq!(round_tripped[0].content, "a");
        assert_eq!(round_tripped[1].role, "assistant");
        assert_eq!(round_tripped[2].role, "user");
    }

    /// (8) Cursor token extraction sentinels.
    #[test]
    fn extract_cursor_tokens_sentinel_zero_when_unset() {
        let chunk = CursorChatResponseChunk {
            model: String::new(),
            delta: String::new(),
            finish_reason: Some("stop".to_string()),
            cumulative_output_tokens: None,
        };
        assert_eq!(extract_cursor_output_tokens(&chunk), 0);
        let chunk2 = CursorChatResponseChunk {
            model: String::new(),
            delta: String::new(),
            finish_reason: Some("stop".to_string()),
            cumulative_output_tokens: Some(42),
        };
        assert_eq!(extract_cursor_output_tokens(&chunk2), 42);
    }

    /// (9) Empty OpenAI choices → empty Cursor delta.
    #[test]
    fn empty_openai_choices_yields_empty_cursor_delta() {
        let chunk = OpenAiChatResponseChunk {
            model: "gpt-4o".to_string(),
            choices: vec![],
            usage: None,
            extra: Default::default(),
        };
        let cursor = openai_chunk_to_cursor(&chunk);
        assert_eq!(cursor.delta, "");
        assert_eq!(cursor.finish_reason, None);
    }
}
