//! Gemini `POST /v1/models/{model}:countTokens` client.
//!
//! Spec refs:
//!   - `tokenizer-service-spec-v1alpha1.md` §4.1 (Tier 1 sampling)
//!   - `tokenizer-service-spec-v1alpha1.md` §7.1 R2 M5 (Gemma
//!     approximation honest disclosure — this client measures the
//!     residual drift)
//!
//! ## Endpoint contract
//!
//! Documented at <https://ai.google.dev/api/tokens>:
//!
//! ```text
//! POST https://generativelanguage.googleapis.com/v1/models/{model}:countTokens?key=$GEMINI_API_KEY
//! Headers:
//!   content-type: application/json
//! Body:
//!   { "contents": [{ "parts": [{ "text": "<text>" }] }] }
//! Response 200:
//!   { "totalTokens": <int>, "totalBillableCharacters": <int> }
//! Response 4xx/5xx:
//!   { "error": { "code": <int>, "message": "...", "status": "..." } }
//! ```
//!
//! ## Resilience
//!
//! Same 5-second timeout as the Anthropic client. Auth uses query-string
//! `?key=...` (Google convention); we never log the URL with the key.

use std::time::{Duration, Instant};

use reqwest::{Client, StatusCode};
use serde_json::json;

use super::{ProviderCount, ProviderError};

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct GeminiClient {
    http: Client,
    base_url: String,
    api_key: String,
}

impl GeminiClient {
    pub fn new(api_key: impl Into<String>) -> Result<Self, ProviderError> {
        Self::with_base_url(api_key, DEFAULT_BASE_URL)
    }

    pub fn with_base_url(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Result<Self, ProviderError> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(ProviderError::Auth("gemini api key empty".into()));
        }
        let http = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!(
                "spendguard-tokenizer/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/spendguard)"
            ))
            .build()
            .map_err(|e| ProviderError::Other(format!("build client: {e}")))?;
        Ok(Self {
            http,
            base_url: base_url.into(),
            api_key,
        })
    }

    pub async fn count_tokens(
        &self,
        model: &str,
        text: &str,
    ) -> Result<ProviderCount, ProviderError> {
        let body = json!({
            "contents": [{
                "parts": [{ "text": text }],
            }],
        });
        // The model path segment may carry a leading "models/" prefix in
        // the wild; normalise so callers can pass either form.
        let model_segment = model.strip_prefix("models/").unwrap_or(model);
        let url = format!(
            "{}/v1/models/{}:countTokens",
            self.base_url, model_segment,
        );

        let start = Instant::now();
        let resp = self
            .http
            .post(&url)
            // Use query() so the api_key never leaks into the &str url
            // we accidentally log.
            .query(&[("key", &self.api_key)])
            .json(&body)
            .send()
            .await
            .map_err(map_send_error)?;
        let latency = start.elapsed();

        let status = resp.status();

        if status.is_success() {
            let parsed: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| ProviderError::Schema(format!("read body: {e}")))?;
            // Gemini's countTokens response uses `totalTokens`.
            let count = parsed
                .get("totalTokens")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| {
                    ProviderError::Schema(format!(
                        "missing or non-u64 `totalTokens` in: {parsed}"
                    ))
                })?;
            return Ok(ProviderCount {
                input_tokens: count,
                // Gemini does not return a stable request id in
                // countTokens responses; leave None.
                request_id: None,
                latency,
            });
        }

        if status == StatusCode::TOO_MANY_REQUESTS {
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(Duration::from_secs)
                .unwrap_or(Duration::from_secs(30));
            return Err(ProviderError::RateLimit { retry_after });
        }
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(ProviderError::Auth(format!(
                "gemini {} {}",
                status,
                truncate_body(resp.text().await.unwrap_or_default())
            )));
        }
        Err(ProviderError::Other(format!(
            "gemini {} {}",
            status,
            truncate_body(resp.text().await.unwrap_or_default())
        )))
    }
}

fn map_send_error(e: reqwest::Error) -> ProviderError {
    if e.is_timeout() {
        ProviderError::Timeout
    } else {
        ProviderError::Other(format!("gemini send: {e}"))
    }
}

fn truncate_body(body: String) -> String {
    if body.len() <= 512 {
        body
    } else {
        format!("{}…(truncated {}B)", &body[..512], body.len() - 512)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn client_for_server(server: &MockServer) -> GeminiClient {
        GeminiClient::with_base_url("test-key", server.uri()).expect("client builds")
    }

    #[tokio::test]
    async fn count_tokens_success_returns_total_tokens() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/models/gemini-1.5-flash:countTokens"))
            .and(query_param("key", "test-key"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({
                    "totalTokens": 17,
                    "totalBillableCharacters": 60
                })),
            )
            .mount(&server)
            .await;
        let c = client_for_server(&server).await;
        let resp = c.count_tokens("gemini-1.5-flash", "hello").await.expect("ok");
        assert_eq!(resp.input_tokens, 17);
        assert!(resp.request_id.is_none());
    }

    #[tokio::test]
    async fn count_tokens_strips_models_prefix_from_model_path() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/models/gemini-1.5-flash:countTokens"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "totalTokens": 1 })),
            )
            .mount(&server)
            .await;
        let c = client_for_server(&server).await;
        let resp = c
            .count_tokens("models/gemini-1.5-flash", "x")
            .await
            .expect("ok");
        assert_eq!(resp.input_tokens, 1);
    }

    #[tokio::test]
    async fn count_tokens_schema_drift_returns_schema_variant() {
        let server = MockServer::start().await;
        // Vendor changed key from totalTokens to tokenCount.
        Mock::given(method("POST"))
            .and(path("/v1/models/g:countTokens"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "tokenCount": 5 })),
            )
            .mount(&server)
            .await;
        let c = client_for_server(&server).await;
        let err = c.count_tokens("g", "x").await.expect_err("schema");
        match err {
            ProviderError::Schema(_) => {}
            other => panic!("expected Schema, got {other:?}"),
        }
        assert!(!err.counts_as_breaker_failure());
    }

    #[tokio::test]
    async fn count_tokens_403_returns_auth_variant() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/models/g:countTokens"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let c = client_for_server(&server).await;
        let err = c.count_tokens("g", "x").await.expect_err("403");
        assert!(matches!(err, ProviderError::Auth(_)));
    }

    #[tokio::test]
    async fn count_tokens_429_returns_rate_limit() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/models/g:countTokens"))
            .respond_with(
                ResponseTemplate::new(429).insert_header("retry-after", "8"),
            )
            .mount(&server)
            .await;
        let c = client_for_server(&server).await;
        let err = c.count_tokens("g", "x").await.expect_err("429");
        match err {
            ProviderError::RateLimit { retry_after } => {
                assert_eq!(retry_after, Duration::from_secs(8));
            }
            other => panic!("expected RateLimit, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn count_tokens_500_returns_other_variant() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/models/g:countTokens"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let c = client_for_server(&server).await;
        let err = c.count_tokens("g", "x").await.expect_err("500");
        assert!(matches!(err, ProviderError::Other(_)));
        assert!(err.counts_as_breaker_failure());
    }

    #[test]
    fn empty_api_key_rejected() {
        let err = GeminiClient::new("").expect_err("empty key");
        assert!(matches!(err, ProviderError::Auth(_)));
    }
}
