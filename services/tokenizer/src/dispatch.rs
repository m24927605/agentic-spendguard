//! Proto ↔ library-struct conversions.
//!
//! Spec ref `tokenizer-service-spec-v1alpha1.md` §2.2 / §2.3 —
//! the gRPC and library forms share semantics; the conversions
//! here are lossless mechanical mappings.

use crate::proto::tokenizer::v1 as proto;
use spendguard_tokenizer::{Message, TokenizeRequest, TokenizeResponse, ToolCall};

impl From<proto::TokenizeRequest> for TokenizeRequest {
    fn from(p: proto::TokenizeRequest) -> Self {
        TokenizeRequest {
            model: p.model,
            messages: p.messages.into_iter().map(Into::into).collect(),
            raw_text: p.raw_text,
            request_id: p.request_id,
        }
    }
}

impl From<proto::tokenize_request::Message> for Message {
    fn from(p: proto::tokenize_request::Message) -> Self {
        Message {
            role: p.role,
            content: p.content,
            tool_calls: p.tool_calls.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<proto::tokenize_request::message::ToolCall> for ToolCall {
    fn from(p: proto::tokenize_request::message::ToolCall) -> Self {
        ToolCall {
            name: p.name,
            arguments_json: p.arguments_json,
        }
    }
}

impl From<TokenizeResponse> for proto::TokenizeResponse {
    fn from(r: TokenizeResponse) -> Self {
        proto::TokenizeResponse {
            input_tokens: r.input_tokens,
            tier: r.tier,
            tokenizer_version_id: r.tokenizer_version_id,
            kind: r.kind,
            fallback_char_count: r.fallback_char_count,
            fallback_margin_ratio: r.fallback_margin_ratio,
            latency_ns: r.latency_ns,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proto_request_to_library_round_trip() {
        let p = proto::TokenizeRequest {
            model: "gpt-4o-mini".to_string(),
            messages: vec![proto::tokenize_request::Message {
                role: "user".to_string(),
                content: "hello".to_string(),
                tool_calls: vec![proto::tokenize_request::message::ToolCall {
                    name: "get_weather".to_string(),
                    arguments_json: r#"{"city":"sf"}"#.to_string(),
                }],
            }],
            raw_text: String::new(),
            request_id: "abc".to_string(),
        };
        let lib: TokenizeRequest = p.into();
        assert_eq!(lib.model, "gpt-4o-mini");
        assert_eq!(lib.messages.len(), 1);
        assert_eq!(lib.messages[0].tool_calls.len(), 1);
        assert_eq!(lib.messages[0].tool_calls[0].name, "get_weather");
        assert_eq!(lib.request_id, "abc");
    }

    #[test]
    fn library_response_to_proto_round_trip() {
        let lib = TokenizeResponse {
            input_tokens: 42,
            tier: "T2".to_string(),
            tokenizer_version_id: "abc-def".to_string(),
            kind: "OPENAI_TIKTOKEN".to_string(),
            fallback_char_count: 0,
            fallback_margin_ratio: 0.0,
            latency_ns: 123,
        };
        let p: proto::TokenizeResponse = lib.into();
        assert_eq!(p.input_tokens, 42);
        assert_eq!(p.tier, "T2");
        assert_eq!(p.kind, "OPENAI_TIKTOKEN");
        assert_eq!(p.latency_ns, 123);
    }
}
