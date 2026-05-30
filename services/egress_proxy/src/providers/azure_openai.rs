//! SLICE_11 — Azure OpenAI provider implementation.
//!
//! Inbound: `POST /openai/deployments/{deployment_id}/chat/completions`
//!
//! Azure OpenAI's wire shape is identical to OpenAI's; the difference
//! is the URL (deployment id alias) + the authentication header
//! (`api-key` instead of `Authorization: Bearer`). The proxy
//! forwards Authorization byte-identical per spec §3.4 so the client
//! is responsible for using whichever header Azure expects.
//!
//! Usage shape:
//!
//!   {
//!     "usage": {
//!       "prompt_tokens": N,
//!       "completion_tokens": M,
//!       "total_tokens": N+M
//!     }
//!   }
//!
//! This is identical to OpenAI's, so we delegate to the OpenAI
//! extractor.

use serde_json::Value;

use crate::providers::openai;
use crate::routing::UsageMetrics;

/// Pull usage from an Azure OpenAI JSON response. Delegates to
/// [`crate::providers::openai::extract_usage`] since the wire shape
/// is identical.
pub fn extract_usage(body: &Value) -> UsageMetrics {
    openai::extract_usage(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_azure_chat_completions_usage() {
        // Same wire shape as OpenAI; the deployment id segment of the
        // URL is what differs.
        let body = json!({
            "id": "chatcmpl-azure-abc",
            "object": "chat.completion",
            "usage": {
                "prompt_tokens": 13,
                "completion_tokens": 42,
                "total_tokens": 55,
            }
        });
        let u = extract_usage(&body);
        assert_eq!(u.input_tokens, 13);
        assert_eq!(u.output_tokens, 42);
        assert_eq!(u.total_tokens, 55);
    }

    #[test]
    fn missing_usage_returns_zeros() {
        let body = json!({"id": "chatcmpl-abc"});
        let u = extract_usage(&body);
        assert_eq!(u.input_tokens, 0);
    }

    #[test]
    fn azure_specific_content_filter_response() {
        // Azure adds content_filter_results — extract_usage ignores it.
        let body = json!({
            "id": "chatcmpl-azure-xyz",
            "content_filter_results": {"hate": {"filtered": false}},
            "usage": {
                "prompt_tokens": 5,
                "completion_tokens": 0,  // filtered → no completion
                "total_tokens": 5,
            }
        });
        let u = extract_usage(&body);
        assert_eq!(u.input_tokens, 5);
        assert_eq!(u.output_tokens, 0);
        assert_eq!(u.total_tokens, 5);
    }
}
