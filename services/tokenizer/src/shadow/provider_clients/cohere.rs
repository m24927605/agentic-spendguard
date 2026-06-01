//! Cohere `POST /v1/tokenize` client.
//!
//! Endpoint contract verified against Cohere docs on 2026-06-02:
//! <https://docs.cohere.com/v2/reference/tokenize>
//!
//! ```text
//! POST https://api.cohere.com/v1/tokenize
//! Authorization: Bearer $COHERE_API_KEY
//! Body: { "text": "...", "model": "command-r-plus-08-2024" }
//! Response 200: { "tokens": [1, 2, ...], "token_strings": [...] }
//! ```
//!
//! The API returns the token IDs, not a scalar count; Tier 1 count is
//! `tokens.len()`. As with Anthropic/Gemini, this client is constructed
//! only inside the tokenizer shadow worker and never from hot-path
//! sidecar/egress_proxy code.

use std::time::{Duration, Instant};

use reqwest::{Client, StatusCode};
use serde_json::json;

use super::{ProviderCount, ProviderError};

const DEFAULT_BASE_URL: &str = "https://api.cohere.com";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct CohereClient {
    http: Client,
    base_url: String,
    api_key: String,
}

impl CohereClient {
    pub fn new(api_key: impl Into<String>) -> Result<Self, ProviderError> {
        Self::with_base_url(api_key, DEFAULT_BASE_URL)
    }

    pub fn with_base_url(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Result<Self, ProviderError> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(ProviderError::Auth("cohere api key empty".into()));
        }
        let http = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(Duration::from_secs(2))
            .tcp_keepalive(Some(Duration::from_secs(30)))
            .user_agent(concat!(
                "spendguard-tokenizer/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/spendguard)"
            ))
            .build()
            .map_err(|e| ProviderError::Other(format!("build client: {e}")))?;
        Ok(Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_key,
        })
    }

    pub async fn count_tokens(
        &self,
        model: &str,
        text: &str,
    ) -> Result<ProviderCount, ProviderError> {
        let body = json!({
            "text": text,
            "model": model,
        });
        let url = format!("{}/v1/tokenize", self.base_url);

        let start = Instant::now();
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(map_send_error)?;
        let latency = start.elapsed();

        let status = resp.status();
        let request_id = resp
            .headers()
            .get("x-request-id")
            .or_else(|| resp.headers().get("request-id"))
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_owned());

        if status.is_success() {
            let parsed: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| ProviderError::Schema(format!("read body: {e}")))?;
            let tokens = parsed
                .get("tokens")
                .and_then(|v| v.as_array())
                .ok_or_else(|| ProviderError::Schema("missing array `tokens`".into()))?;
            return Ok(ProviderCount {
                input_tokens: tokens.len() as u64,
                request_id,
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
        if status == StatusCode::UNAUTHORIZED
            || status == StatusCode::FORBIDDEN
            || status.as_u16() == 498
        {
            return Err(ProviderError::Auth(format!(
                "cohere {} {}",
                status,
                body_summary(resp.text().await.unwrap_or_default())
            )));
        }
        Err(ProviderError::Other(format!(
            "cohere {} {}",
            status,
            body_summary(resp.text().await.unwrap_or_default())
        )))
    }
}

fn map_send_error(e: reqwest::Error) -> ProviderError {
    if e.is_timeout() {
        ProviderError::Timeout
    } else {
        let safe = e.without_url();
        ProviderError::Other(format!("cohere send: {safe}"))
    }
}

fn body_summary(body: String) -> String {
    format!("<provider body redacted: {}B>", body.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn client_for_server(server: &MockServer) -> CohereClient {
        CohereClient::with_base_url("test-key", server.uri()).expect("client builds")
    }

    #[tokio::test]
    async fn count_tokens_success_returns_tokens_len() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/tokenize"))
            .and(header("authorization", "Bearer test-key"))
            .and(body_json(json!({
                "text": "hello world",
                "model": "command-r-plus-08-2024"
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-request-id", "coh_req_123")
                    .set_body_json(json!({
                        "tokens": [101, 202, 303],
                        "token_strings": ["hello", " world"]
                    })),
            )
            .mount(&server)
            .await;

        let c = client_for_server(&server).await;
        let resp = c
            .count_tokens("command-r-plus-08-2024", "hello world")
            .await
            .expect("ok");
        assert_eq!(resp.input_tokens, 3);
        assert_eq!(resp.request_id.as_deref(), Some("coh_req_123"));
    }

    #[tokio::test]
    async fn schema_drift_missing_tokens_is_schema_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/tokenize"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "count": 3
            })))
            .mount(&server)
            .await;
        let c = client_for_server(&server).await;
        let err = c
            .count_tokens("command-r", "hello")
            .await
            .expect_err("schema");
        assert!(matches!(err, ProviderError::Schema(_)));
        assert!(!err.counts_as_breaker_failure());
    }

    #[tokio::test]
    async fn auth_failures_map_to_auth_variant() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/tokenize"))
            .respond_with(ResponseTemplate::new(498).set_body_string("invalid token"))
            .mount(&server)
            .await;
        let c = client_for_server(&server).await;
        let err = c
            .count_tokens("command-r", "hello")
            .await
            .expect_err("auth");
        assert!(matches!(err, ProviderError::Auth(_)));
        assert!(!err.counts_as_breaker_failure());
    }

    #[tokio::test]
    async fn rate_limit_maps_to_retry_after() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/tokenize"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "7"))
            .mount(&server)
            .await;
        let c = client_for_server(&server).await;
        let err = c
            .count_tokens("command-r", "hello")
            .await
            .expect_err("rate limit");
        match err {
            ProviderError::RateLimit { retry_after } => {
                assert_eq!(retry_after, Duration::from_secs(7));
            }
            other => panic!("expected rate limit, got {other:?}"),
        }
    }
}
