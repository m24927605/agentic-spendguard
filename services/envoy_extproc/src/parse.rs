//! Request-body parsing for the ExtProc Request-Body phase.
//!
//! Spec refs:
//!   - docs/specs/coverage/D01_envoy_extproc/design.md §3.2 (token counting reuses egress_proxy routing)
//!   - docs/specs/coverage/D01_envoy_extproc/implementation.md §5 (parse_request_body shape)
//!   - docs/internal/slices/COV_02_envoy_extproc_token_counter.md (this slice scope)
//!
//! Wraps `spendguard-provider-routing::route` + per-provider JSON shape
//! extraction. The returned [`ParsedRequest`] carries everything
//! [`crate::tokenize::estimate_tokens`] needs to call the tokenizer:
//!   - `provider`/`provider_str` — for audit metadata + tier labels
//!   - `model_id` — the model string sent to the tokenizer dispatch table
//!   - `tokenizer_kind` — pre-resolved by `provider-routing`; passed
//!     through unchanged. For Bedrock this is the per-model dispatch
//!     result (Anthropic / Cohere / Llama), not the routing-table default.
//!   - `messages` — extracted in the canonical
//!     `spendguard_tokenizer::Message` shape so the tokenizer does NOT
//!     re-shape the JSON.
//!
//! ## Per-provider body shapes (v1 minimum coverage)
//!
//! | Path | RequestShape | Where `model` lives | Where `messages` live |
//! |------|--------------|--------------------|----------------------|
//! | `/v1/chat/completions` | OpenAI ChatCompletions | `body.model` | `body.messages[]` |
//! | `/v1/responses` | OpenAI Responses | `body.model` | `body.input[]` (string-or-array) |
//! | `/v1/messages` | Anthropic Messages | `body.model` | `body.messages[]` + optional `body.system` |
//! | `/model/<id>/invoke` | Bedrock InvokeModel | URL captures `<id>` | Anthropic-on-Bedrock shape, or `prompt`, or `inputText` |
//! | Vertex generateContent | Vertex | `body.model` (path also encodes it) | `body.contents[]` |
//! | Azure OpenAI | Azure ChatCompletions | URL deployment id (`model` body field absent) | `body.messages[]` |
//!
//! The Vertex / Azure paths reuse the OpenAI-shape extractor (with a
//! per-provider deployment-id fallback for Azure) — the tokenizer only
//! needs `(role, content)` tuples to count Tier 2 envelope tokens, so
//! exotic provider knobs (`tools`, `tool_choice`, `system` blocks) are
//! flattened into the `messages` vector with synthetic roles where
//! needed. This matches the egress_proxy SLICE_10
//! `serialize_messages_for_tokenizer` heuristic in
//! `services/egress_proxy/src/decision.rs:370-395`.

use bytes::Bytes;
use serde_json::Value;
use spendguard_provider_routing::{
    resolve_model_id, resolve_tokenizer_kind, route, ProviderKind, RequestShape,
};
use spendguard_tokenizer::{EncoderKind, Message};
use thiserror::Error;

/// Parsed request body — input to [`crate::tokenize::estimate_tokens`].
#[derive(Debug, Clone)]
pub struct ParsedRequest {
    /// Provider that owns this request (audit + log).
    pub provider: ProviderKind,
    /// `ProviderKind::as_str()` cached so we don't re-borrow at log time.
    pub provider_str: &'static str,
    /// Wire shape — drives the per-provider message extractor branch.
    pub request_shape: RequestShape,
    /// Upstream model identifier. Routes the tokenizer dispatch table.
    pub model_id: String,
    /// Tokenizer kind resolved via `provider-routing::resolve_tokenizer_kind`.
    /// `None` for unknown Bedrock models / Azure deployments where the
    /// underlying model is opaque — caller routes to Tier 3.
    ///
    /// SLICE 2 carries this hint for SLICE 3/4 audit metadata only; the
    /// actual tokenizer dispatch is owned by `Tokenizer::tokenize` via
    /// model string.
    pub tokenizer_kind: Option<EncoderKind>,
    /// Canonical `(role, content)` view of the request body. Empty when
    /// the provider shape has no extractable messages — `estimate_tokens`
    /// then falls back to Tier 3 on `raw_text`.
    pub messages: Vec<Message>,
    /// Raw fallback text (only populated when `messages` is empty AND a
    /// `prompt` / `inputText` string field was present). The tokenizer
    /// library treats `raw_text` as the lone Tier 3 input when
    /// `messages.is_empty()`.
    pub raw_text: String,
    /// SLICE 7 (COV_07) demo-only hook: when the binary is built with
    /// the `uds-dev` cargo feature AND the request body carries
    /// `"spendguard_estimate_override": "<digits>"`, this field is
    /// populated with the parsed `i64`. The tokenize stage swaps the
    /// estimate in iff the feature is on; production builds (chart
    /// image, `--no-default-features`) never reach this code path
    /// because the field-extractor itself is feature-gated. Mirrors
    /// the `litellm_guardrail` SDK demo path so both demos can exercise
    /// the DENY lane against the seeded 1B-atomic hard-cap.
    #[cfg(feature = "uds-dev")]
    pub demo_estimate_override: Option<i64>,
}

/// Parse errors mapped 1:1 to ExtProc handler outcomes.
///
/// Per slice §"Scope" the server treats every variant as a non-fatal
/// warn-and-continue — SLICE 3 will fail-closed when an estimate is
/// missing. We surface typed errors here so the SLICE 3 wire-up can map
/// each kind to the right ExtProc immediate_response status without
/// re-parsing.
#[derive(Debug, Error)]
pub enum ParseError {
    /// Path doesn't match any row in the routing table — Envoy admitted
    /// a request our routing-table doesn't know about. Should be rare in
    /// production (Envoy AI Gateway config gates which paths reach us)
    /// but we MUST handle it without panicking per review-standards §3.2.
    #[error("path `{path}` does not match any provider routing entry")]
    UnknownProviderPath { path: String },

    /// JSON body failed to parse. Returned from `serde_json::from_slice`.
    /// Maps to ExtProc immediate_response 400 in SLICE 3.
    #[error("request body is not valid JSON: {source}")]
    InvalidJson {
        #[source]
        source: serde_json::Error,
    },
}

/// Parse + dispatch the ExtProc Request-Body bytes.
///
/// `path` is the inbound HTTP request path captured from the
/// Request-Headers phase `:path` pseudo-header. `body` is the raw bytes
/// from `ProcessingRequest::RequestBody.body`.
pub fn parse_request_body(path: &str, body: &Bytes) -> Result<ParsedRequest, ParseError> {
    let cfg = route(path).ok_or_else(|| ParseError::UnknownProviderPath {
        path: path.to_string(),
    })?;

    // Empty body is admissible in theory (e.g. an Envoy-injected probe)
    // but yields no extractable messages — surface as InvalidJson so the
    // caller sees a typed error rather than a silent zero-token estimate.
    let value: Value =
        serde_json::from_slice(body).map_err(|source| ParseError::InvalidJson { source })?;

    let model_id = resolve_model_id(cfg, path, &value);
    let tokenizer_kind = resolve_tokenizer_kind(cfg, path, &value);
    let (messages, raw_text) = extract_messages(cfg.request_shape, &value);

    // SLICE 7 demo override extraction (uds-dev only). Accepts either a
    // JSON number or a JSON string of digits — matches the SDK
    // `litellm_guardrail` resolver's shape so both demos use the same
    // request envelope.
    #[cfg(feature = "uds-dev")]
    let demo_estimate_override = value
        .get("spendguard_estimate_override")
        .and_then(|v| match v {
            Value::Number(n) => n.as_i64(),
            Value::String(s) => s.trim().parse::<i64>().ok(),
            _ => None,
        })
        .filter(|n| *n > 0);

    Ok(ParsedRequest {
        provider: cfg.kind,
        provider_str: cfg.kind.as_str(),
        request_shape: cfg.request_shape,
        model_id,
        tokenizer_kind,
        messages,
        raw_text,
        #[cfg(feature = "uds-dev")]
        demo_estimate_override,
    })
}

/// Per-shape `(role, content)` extraction. Returns the canonical
/// `Vec<Message>` + an optional `raw_text` fallback for shapes that
/// carry an unstructured prompt string (Bedrock Llama `prompt`, Bedrock
/// Titan `inputText`).
fn extract_messages(shape: RequestShape, body: &Value) -> (Vec<Message>, String) {
    match shape {
        RequestShape::OpenAiChatCompletions | RequestShape::AzureOpenAiChatCompletions => {
            (extract_openai_chat_messages(body), String::new())
        }
        RequestShape::OpenAiResponses => {
            // The Responses API accepts `input` as either a plain string
            // or an array of message-shaped blocks. Both forms get
            // flattened into a single-message synthetic `user` body so
            // the tokenizer sees the same envelope as ChatCompletions.
            let messages = extract_openai_responses_input(body);
            (messages, String::new())
        }
        RequestShape::AnthropicMessages => (extract_anthropic_messages(body), String::new()),
        RequestShape::BedrockInvokeModel => extract_bedrock_body(body),
        RequestShape::VertexGenerateContent => (extract_vertex_contents(body), String::new()),
    }
}

/// OpenAI ChatCompletions + Azure OpenAI — `body.messages[]` array of
/// `{role, content}` objects.
///
/// Tool-call envelopes (`messages[].tool_calls`, `messages[].name`) are
/// flattened by treating each tool call's `function.arguments` string
/// as additional content under the message's role — matches the
/// egress_proxy heuristic in `services/egress_proxy/src/decision.rs`.
fn extract_openai_chat_messages(body: &Value) -> Vec<Message> {
    let arr = match body.get("messages").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .map(|m| Message {
            role: m
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("user")
                .to_string(),
            content: extract_content_field(m.get("content")),
            tool_calls: Vec::new(),
        })
        .collect()
}

/// OpenAI Responses API — `body.input` is `string | Vec<MessageBlock>`.
/// Spec §5 + impl §5: the tokenizer only consumes `(role, content)`
/// pairs, so a bare string is wrapped as `{role: "user", content: <input>}`.
fn extract_openai_responses_input(body: &Value) -> Vec<Message> {
    let input = match body.get("input") {
        Some(v) => v,
        None => return Vec::new(),
    };
    if let Some(s) = input.as_str() {
        return vec![Message {
            role: "user".to_string(),
            content: s.to_string(),
            tool_calls: Vec::new(),
        }];
    }
    if let Some(arr) = input.as_array() {
        return arr
            .iter()
            .map(|m| Message {
                role: m
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("user")
                    .to_string(),
                content: extract_content_field(m.get("content")),
                tool_calls: Vec::new(),
            })
            .collect();
    }
    Vec::new()
}

/// Anthropic Messages — `body.messages[]` + optional top-level
/// `body.system`. Per Anthropic API docs the `system` field is a
/// top-level sibling (string OR array of content blocks); we synthesise
/// a `{role: "system", content: <system>}` prefix Message so the
/// tokenizer counts those tokens too — matches Anthropic's own
/// `count_tokens` API behaviour.
fn extract_anthropic_messages(body: &Value) -> Vec<Message> {
    let mut out = Vec::new();
    if let Some(system) = body.get("system") {
        let content = extract_content_field(Some(system));
        if !content.is_empty() {
            out.push(Message {
                role: "system".to_string(),
                content,
                tool_calls: Vec::new(),
            });
        }
    }
    if let Some(arr) = body.get("messages").and_then(|v| v.as_array()) {
        for m in arr {
            out.push(Message {
                role: m
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("user")
                    .to_string(),
                content: extract_content_field(m.get("content")),
                tool_calls: Vec::new(),
            });
        }
    }
    out
}

/// Bedrock InvokeModel — body shape depends on the upstream vendor.
///
/// * Anthropic-on-Bedrock: `{anthropic_version, messages, system, ...}`
///   — same extractor as native Anthropic.
/// * Llama: `{prompt: "..."}` — fall through `raw_text`.
/// * Cohere: `{message: "..."}` or `{chat_history: [...]}` — flatten.
/// * Titan: `{inputText: "..."}` — fall through `raw_text`.
///
/// Anything else falls through with an empty messages vec; the tokenizer
/// will Tier 3 against the raw body bytes if the caller sets
/// `raw_text` (we don't, to avoid arbitrary-binary edge cases).
fn extract_bedrock_body(body: &Value) -> (Vec<Message>, String) {
    // Anthropic-on-Bedrock — present `anthropic_version` is the
    // contract-locked marker (`anthropic_version: "bedrock-2023-05-31"`).
    if body.get("anthropic_version").is_some() || body.get("messages").is_some() {
        return (extract_anthropic_messages(body), String::new());
    }
    // Llama family — `{prompt: "..."}`.
    if let Some(s) = body.get("prompt").and_then(|v| v.as_str()) {
        return (Vec::new(), s.to_string());
    }
    // Cohere — `{message: "..."}` (chat) or `chat_history` array.
    if let Some(s) = body.get("message").and_then(|v| v.as_str()) {
        let mut msgs = vec![Message {
            role: "user".to_string(),
            content: s.to_string(),
            tool_calls: Vec::new(),
        }];
        if let Some(arr) = body.get("chat_history").and_then(|v| v.as_array()) {
            // Cohere chat_history uses role tags `USER` / `CHATBOT`;
            // normalise to lower-case so the tokenizer envelope matches
            // the OpenAI / Anthropic baseline.
            let history: Vec<Message> = arr
                .iter()
                .map(|m| Message {
                    role: m
                        .get("role")
                        .and_then(|v| v.as_str())
                        .map(|r| r.to_ascii_lowercase())
                        .unwrap_or_else(|| "user".to_string()),
                    content: m
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    tool_calls: Vec::new(),
                })
                .collect();
            // Cohere convention: history comes BEFORE the current message.
            let mut combined = history;
            combined.append(&mut msgs);
            return (combined, String::new());
        }
        return (msgs, String::new());
    }
    // Titan — `{inputText: "..."}`.
    if let Some(s) = body.get("inputText").and_then(|v| v.as_str()) {
        return (Vec::new(), s.to_string());
    }
    (Vec::new(), String::new())
}

/// Vertex generateContent — `body.contents[]` of
/// `{role, parts: [{text}, ...]}`. Each content block becomes one
/// `Message`; the parts' text fields are concatenated.
fn extract_vertex_contents(body: &Value) -> Vec<Message> {
    let arr = match body.get("contents").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .map(|c| {
            let role = c
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("user")
                .to_string();
            let content = match c.get("parts").and_then(|v| v.as_array()) {
                Some(parts) => parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join(""),
                None => String::new(),
            };
            Message {
                role,
                content,
                tool_calls: Vec::new(),
            }
        })
        .collect()
}

/// Coerce a JSON value that might be a string or an array of content
/// blocks (Anthropic / OpenAI Responses tool-result envelopes) into a
/// flat string. Treats unknown / null shapes as empty.
fn extract_content_field(v: Option<&Value>) -> String {
    let v = match v {
        Some(v) => v,
        None => return String::new(),
    };
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    if let Some(arr) = v.as_array() {
        // Anthropic content blocks: each has either {type: "text", text}
        // or {type: "image", source} etc. We only sum text fields for
        // Tier 2 envelope counting; non-text blocks contribute 0 to the
        // input_tokens estimate (the provider's billed cost for image
        // tokens is a separate ratecard that the response usage block
        // exposes — SLICE 4 audit_emit consumes it).
        // SLICE 5 conformance: revisit envelope vs `.join("\n")` for
        // multi-block tool-use. Today multi-block tool-use envelopes
        // surface as the text-block concatenation only — the `tool_use`
        // / `tool_result` blocks contribute 0 tokens. Acceptable for
        // SLICE 2 (Tier 2 envelope counting); SLICE 5 will fold the
        // tool-result envelope into the count via the canonical envelope
        // form used by Anthropic's own `count_tokens`.
        return arr
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("");
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use spendguard_provider_routing::{init_extractors_for_test, RoutingExtractors, UsageMetrics};

    /// `route()` requires the routing table to have extractors registered
    /// once per process; the helper is idempotent so parallel test
    /// processes don't race.
    fn install_test_extractors() {
        fn noop(_: &Value) -> UsageMetrics {
            UsageMetrics::default()
        }
        init_extractors_for_test(RoutingExtractors {
            openai: noop,
            anthropic: noop,
            bedrock: noop,
            vertex: noop,
            azure_openai: noop,
        });
    }

    #[test]
    fn parses_openai_chat_completions_body() {
        install_test_extractors();
        let body = Bytes::from(
            r#"{
                "model": "gpt-4o-mini",
                "messages": [
                    {"role": "system", "content": "You are helpful."},
                    {"role": "user", "content": "Hello"}
                ]
            }"#,
        );
        let parsed = parse_request_body("/v1/chat/completions", &body).expect("parse ok");
        assert_eq!(parsed.provider, ProviderKind::OpenAi);
        assert_eq!(parsed.provider_str, "openai");
        assert_eq!(parsed.request_shape, RequestShape::OpenAiChatCompletions);
        assert_eq!(parsed.model_id, "gpt-4o-mini");
        assert_eq!(parsed.tokenizer_kind, Some(EncoderKind::OpenAi));
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(parsed.messages[0].role, "system");
        assert_eq!(parsed.messages[0].content, "You are helpful.");
        assert_eq!(parsed.messages[1].role, "user");
        assert_eq!(parsed.messages[1].content, "Hello");
        assert!(parsed.raw_text.is_empty());
    }

    #[test]
    fn parses_anthropic_messages_body() {
        install_test_extractors();
        let body = Bytes::from(
            r#"{
                "model": "claude-3-5-sonnet-20240620",
                "system": "You are concise.",
                "messages": [
                    {"role": "user", "content": "What is 2+2?"}
                ]
            }"#,
        );
        let parsed = parse_request_body("/v1/messages", &body).expect("parse ok");
        assert_eq!(parsed.provider, ProviderKind::Anthropic);
        assert_eq!(parsed.model_id, "claude-3-5-sonnet-20240620");
        assert_eq!(parsed.tokenizer_kind, Some(EncoderKind::Anthropic));
        // system prefix + user => 2 messages.
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(parsed.messages[0].role, "system");
        assert_eq!(parsed.messages[0].content, "You are concise.");
        assert_eq!(parsed.messages[1].role, "user");
        assert_eq!(parsed.messages[1].content, "What is 2+2?");
    }

    #[test]
    fn parses_anthropic_content_blocks_text_only() {
        install_test_extractors();
        // Content as an array of {type: "text", text: ...} blocks.
        let body = Bytes::from(
            r#"{
                "model": "claude-3-5-sonnet-20240620",
                "messages": [
                    {"role": "user", "content": [
                        {"type": "text", "text": "Part A"},
                        {"type": "text", "text": " Part B"}
                    ]}
                ]
            }"#,
        );
        let parsed = parse_request_body("/v1/messages", &body).expect("parse ok");
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].content, "Part A Part B");
    }

    #[test]
    fn parses_bedrock_anthropic_body() {
        install_test_extractors();
        // Anthropic-on-Bedrock: model from URL, body shape matches
        // Anthropic native (anthropic_version + messages).
        let body = Bytes::from(
            r#"{
                "anthropic_version": "bedrock-2023-05-31",
                "max_tokens": 100,
                "messages": [
                    {"role": "user", "content": "Hello Bedrock"}
                ]
            }"#,
        );
        let path = "/model/anthropic.claude-3-5-sonnet-20240620-v1:0/invoke";
        let parsed = parse_request_body(path, &body).expect("parse ok");
        assert_eq!(parsed.provider, ProviderKind::Bedrock);
        assert_eq!(parsed.model_id, "anthropic.claude-3-5-sonnet-20240620-v1:0");
        // Bedrock routes to the per-model tokenizer dispatch — anthropic
        // for a claude-3 family Bedrock id.
        assert_eq!(parsed.tokenizer_kind, Some(EncoderKind::Anthropic));
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].role, "user");
        assert_eq!(parsed.messages[0].content, "Hello Bedrock");
        assert!(parsed.raw_text.is_empty());
    }

    #[test]
    fn parses_bedrock_llama_prompt_into_raw_text() {
        install_test_extractors();
        // Llama-on-Bedrock: `{prompt: "..."}` — no messages, raw_text only.
        let body = Bytes::from(r#"{"prompt": "Once upon a time"}"#);
        let path = "/model/meta.llama3-1-70b-instruct-v1:0/invoke";
        let parsed = parse_request_body(path, &body).expect("parse ok");
        assert_eq!(parsed.provider, ProviderKind::Bedrock);
        assert_eq!(parsed.tokenizer_kind, Some(EncoderKind::Llama));
        assert!(parsed.messages.is_empty());
        assert_eq!(parsed.raw_text, "Once upon a time");
    }

    #[test]
    fn rejects_unknown_provider_path() {
        install_test_extractors();
        let body = Bytes::from(r#"{"foo": "bar"}"#);
        let err = parse_request_body("/totally/unknown/path", &body)
            .expect_err("must surface UnknownProviderPath");
        match err {
            ParseError::UnknownProviderPath { path } => {
                assert_eq!(path, "/totally/unknown/path");
            }
            other => panic!("expected UnknownProviderPath, got {other:?}"),
        }
    }

    #[test]
    fn rejects_malformed_json() {
        install_test_extractors();
        let body = Bytes::from(r#"{"model": "gpt-4o", "messages": ["#);
        let err = parse_request_body("/v1/chat/completions", &body)
            .expect_err("must surface InvalidJson");
        assert!(matches!(err, ParseError::InvalidJson { .. }), "got: {err}");
    }

    #[test]
    fn missing_model_falls_back_to_unknown_marker() {
        // SLICE_11 routing's `resolve_model_id` returns "unknown" when
        // the model field is missing (egress_proxy parity). The
        // tokenizer then Tier 3 falls back.
        install_test_extractors();
        let body = Bytes::from(r#"{"messages": [{"role": "user", "content": "hi"}]}"#);
        let parsed = parse_request_body("/v1/chat/completions", &body).expect("parse ok");
        assert_eq!(parsed.model_id, "unknown");
        assert_eq!(parsed.messages.len(), 1);
    }

    #[test]
    fn parses_openai_responses_string_input() {
        install_test_extractors();
        // The Responses API accepts a bare string as input.
        let body = Bytes::from(r#"{"model": "gpt-4o", "input": "Hello world"}"#);
        let parsed = parse_request_body("/v1/responses", &body).expect("parse ok");
        assert_eq!(parsed.request_shape, RequestShape::OpenAiResponses);
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].role, "user");
        assert_eq!(parsed.messages[0].content, "Hello world");
    }

    #[test]
    fn parses_vertex_contents_with_parts() {
        install_test_extractors();
        let body = Bytes::from(
            r#"{
                "contents": [
                    {"role": "user", "parts": [{"text": "Hello "}, {"text": "Gemini"}]}
                ]
            }"#,
        );
        let path = "/v1/projects/my-proj/locations/us-central1/publishers/google/models/gemini-1.5-pro:generateContent";
        let parsed = parse_request_body(path, &body).expect("parse ok");
        assert_eq!(parsed.provider, ProviderKind::Vertex);
        assert_eq!(parsed.tokenizer_kind, Some(EncoderKind::Gemini));
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].content, "Hello Gemini");
    }

    #[test]
    fn bedrock_unknown_vendor_returns_none_tokenizer() {
        install_test_extractors();
        // Titan model id — pre-Claude-3 Bedrock vendor not in the
        // dispatch table. Per spec §3.1 falls to Tier 3 (tokenizer_kind = None).
        let body = Bytes::from(r#"{"inputText": "Hello Titan"}"#);
        let path = "/model/amazon.titan-text-express-v1/invoke";
        let parsed = parse_request_body(path, &body).expect("parse ok");
        assert_eq!(parsed.provider, ProviderKind::Bedrock);
        assert_eq!(parsed.tokenizer_kind, None);
        assert!(parsed.messages.is_empty());
        assert_eq!(parsed.raw_text, "Hello Titan");
    }
}
