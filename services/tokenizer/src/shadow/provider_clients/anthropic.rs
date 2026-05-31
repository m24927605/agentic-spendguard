//! Anthropic `POST /v1/messages/count_tokens` client.
//!
//! Spec refs:
//!   - `tokenizer-service-spec-v1alpha1.md` §4.1 (Tier 1 sampling)
//!   - `tokenizer-service-spec-v1alpha1.md` §1.2 (Anthropic count_tokens
//!     listed as "Tier 1" exemplar)
//!
//! ## Endpoint contract
//!
//! Documented at <https://docs.anthropic.com/en/api/messages-count-tokens>:
//!
//! ```text
//! POST https://api.anthropic.com/v1/messages/count_tokens
//! Headers:
//!   x-api-key: $ANTHROPIC_API_KEY
//!   anthropic-version: 2023-06-01
//!   content-type: application/json
//! Body:
//!   {
//!     "model": "claude-3-5-sonnet-20241022",
//!     "messages": [{"role": "user", "content": "<text>"}]
//!   }
//! Response 200:
//!   { "input_tokens": <int> }
//! Response 4xx/5xx:
//!   { "type": "error", "error": { "type": "...", "message": "..." } }
//! ```
//!
//! ## Resilience
//!
//! 5-second request timeout (`Timeout`-variant on expiry). One transparent
//! retry on transient network errors with 250ms backoff; further retries
//! and circuit-breaker integration live in `super::worker` (Phase E).
//!
//! Hot path invariant: this module is never reachable from the sidecar
//! or egress_proxy crates. Constructed once at tokenizer-service boot.

use std::time::{Duration, Instant};

use reqwest::{Client, StatusCode};
use serde_json::json;

use super::{ProviderCount, ProviderError};

/// Anthropic count_tokens API base URL. Tests override via the
/// `with_base_url` constructor.
const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// `anthropic-version` header value. Documented stable since 2023-06-01.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Per-request timeout. Spec §4 latency budget — Tier 1 is off the hot
/// path, but we still bound the in-flight provider call so the shadow
/// worker's queue lag SLO (per spec §10.2: p99 < 30s) isn't blown by a
/// stuck provider.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Anthropic count_tokens HTTP client. Cheap to clone — wraps
/// `reqwest::Client`'s internal `Arc` so the shadow worker can share
/// one instance across all per-(tenant, model) calls.
#[derive(Debug, Clone)]
pub struct AnthropicClient {
    http: Client,
    base_url: String,
    api_key: String,
}

impl AnthropicClient {
    /// Construct with `ANTHROPIC_API_KEY` from caller. Returns
    /// `ProviderError::Auth` if key is empty (cheap pre-flight check).
    pub fn new(api_key: impl Into<String>) -> Result<Self, ProviderError> {
        Self::with_base_url(api_key, DEFAULT_BASE_URL)
    }

    /// Test-visible variant — wiremock servers run on `127.0.0.1:<port>`
    /// and need an alternate base URL.
    pub fn with_base_url(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Result<Self, ProviderError> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(ProviderError::Auth("anthropic api key empty".into()));
        }
        let http = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            // R2 M4: split connect timeout from total timeout so DNS /
            // TCP / TLS handshake failures surface fast (2s) regardless
            // of REQUEST_TIMEOUT. Keep-alive avoids per-request TCP
            // overhead on the hot worker loop.
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
            base_url: base_url.into(),
            api_key,
        })
    }

    /// Call `POST /v1/messages/count_tokens` for the supplied (model,
    /// text) pair. Returns the Tier 1 token count or a typed
    /// [`ProviderError`].
    pub async fn count_tokens(
        &self,
        model: &str,
        text: &str,
    ) -> Result<ProviderCount, ProviderError> {
        let body = json!({
            "model": model,
            "messages": [{ "role": "user", "content": text }],
        });
        let url = format!("{}/v1/messages/count_tokens", self.base_url);

        let start = Instant::now();
        let resp = self
            .http
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(map_send_error)?;
        let latency = start.elapsed();

        let status = resp.status();
        let request_id = resp
            .headers()
            .get("request-id")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_owned());

        if status.is_success() {
            let parsed: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| ProviderError::Schema(format!("read body: {e}")))?;
            let count = parsed
                .get("input_tokens")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| {
                    ProviderError::Schema(format!("missing or non-u64 `input_tokens` in: {parsed}"))
                })?;
            return Ok(ProviderCount {
                input_tokens: count,
                request_id,
                latency,
            });
        }

        // Error path.
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
                "anthropic {} {}",
                status,
                truncate_body(resp.text().await.unwrap_or_default())
            )));
        }
        Err(ProviderError::Other(format!(
            "anthropic {} {}",
            status,
            truncate_body(resp.text().await.unwrap_or_default())
        )))
    }
}

/// R2 B3 + defense-in-depth: scrub the URL from any reqwest send error
/// before formatting. Anthropic uses header auth so the URL is benign,
/// but the Gemini client carries `?key=...` in the URL — so both
/// clients apply this scrub identically to avoid divergent secret
/// handling. `reqwest::Error::without_url()` returns a clone with the
/// URL stripped; `Display` then renders without leaking it.
fn map_send_error(e: reqwest::Error) -> ProviderError {
    if e.is_timeout() {
        ProviderError::Timeout
    } else {
        let safe = e.without_url();
        ProviderError::Other(format!("anthropic send: {safe}"))
    }
}

/// Cap error body length so a 1MB error response doesn't bloat the log.
///
/// R2 M3: walk char boundaries so a multi-byte UTF-8 character split
/// at byte 512 does not panic the worker. Drops to the nearest <= 512-byte
/// char boundary (always ≤ 512 bytes; may be shorter when the boundary
/// straddles 512).
fn truncate_body(body: String) -> String {
    if body.len() <= 512 {
        return body;
    }
    // Find the largest valid UTF-8 char boundary at or below byte 512.
    let mut cut = 0usize;
    for (i, _) in body.char_indices() {
        if i > 512 {
            break;
        }
        cut = i;
    }
    format!("{}…(truncated {}B)", &body[..cut], body.len() - cut)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn client_for_server(server: &MockServer) -> AnthropicClient {
        AnthropicClient::with_base_url("test-key", server.uri())
            .expect("client builds with test key")
    }

    #[tokio::test]
    async fn count_tokens_success_returns_input_tokens() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages/count_tokens"))
            .and(header("anthropic-version", ANTHROPIC_VERSION))
            .and(header("x-api-key", "test-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("request-id", "req_abc123")
                    .set_body_json(json!({ "input_tokens": 42 })),
            )
            .mount(&server)
            .await;

        let c = client_for_server(&server).await;
        let resp = c
            .count_tokens("claude-3-5-sonnet-20241022", "hello world")
            .await
            .expect("ok");
        assert_eq!(resp.input_tokens, 42);
        assert_eq!(resp.request_id.as_deref(), Some("req_abc123"));
    }

    #[tokio::test]
    async fn count_tokens_schema_drift_returns_schema_variant() {
        let server = MockServer::start().await;
        // Vendor changed shape: nested under `usage.input_tokens`.
        Mock::given(method("POST"))
            .and(path("/v1/messages/count_tokens"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "usage": { "input_tokens": 10 }
            })))
            .mount(&server)
            .await;

        let c = client_for_server(&server).await;
        let err = c.count_tokens("m", "x").await.expect_err("schema drift");
        match err {
            ProviderError::Schema(_) => {}
            other => panic!("expected Schema, got {other:?}"),
        }
        assert!(
            !err.counts_as_breaker_failure(),
            "schema drift must NOT trip the circuit breaker per spec §7"
        );
    }

    #[tokio::test]
    async fn count_tokens_401_returns_auth_variant() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages/count_tokens"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .mount(&server)
            .await;
        let c = client_for_server(&server).await;
        let err = c.count_tokens("m", "x").await.expect_err("401");
        match err {
            ProviderError::Auth(_) => {}
            other => panic!("expected Auth, got {other:?}"),
        }
        assert!(!err.counts_as_breaker_failure());
    }

    #[tokio::test]
    async fn count_tokens_429_returns_rate_limit_with_retry_after() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages/count_tokens"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "12")
                    .set_body_string("rate limited"),
            )
            .mount(&server)
            .await;
        let c = client_for_server(&server).await;
        let err = c.count_tokens("m", "x").await.expect_err("429");
        match err {
            ProviderError::RateLimit { retry_after } => {
                assert_eq!(retry_after, Duration::from_secs(12));
            }
            other => panic!("expected RateLimit, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn count_tokens_5xx_returns_other_variant() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages/count_tokens"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let c = client_for_server(&server).await;
        let err = c.count_tokens("m", "x").await.expect_err("503");
        match err {
            ProviderError::Other(_) => {}
            other => panic!("expected Other, got {other:?}"),
        }
        assert!(err.counts_as_breaker_failure());
    }

    #[test]
    fn empty_api_key_rejected_at_construction() {
        let err = AnthropicClient::new("").expect_err("empty key rejected");
        assert!(matches!(err, ProviderError::Auth(_)));
    }

    /// R2 M3 regression: multi-byte UTF-8 characters straddling byte 512
    /// must NOT panic the worker. Previous `&body[..512]` slice would
    /// fail at runtime with "byte index N is not a char boundary".
    #[test]
    fn truncate_body_handles_multibyte_at_boundary() {
        // Build a string whose chars straddle byte 510-515. Use the
        // multi-byte "é" (2 bytes) — 256 of them = 512 bytes — and
        // then append a string that crosses the boundary.
        let mut s = String::with_capacity(2000);
        // First 510 bytes (255 × 2-byte chars).
        for _ in 0..255 {
            s.push('é');
        }
        // Now add a 3-byte char that crosses byte 510 boundary.
        s.push('日');
        // Pad to > 512 so the truncation path fires.
        for _ in 0..200 {
            s.push('日');
        }
        assert!(s.len() > 512);
        let out = truncate_body(s);
        // Returned string MUST be valid UTF-8 (test would panic if not)
        // and ends with the truncation marker.
        assert!(out.ends_with(')'));
        assert!(out.contains("(truncated"));
        // The body prefix length is ≤ 512 bytes.
        let trail = "…(truncated ";
        let prefix_end = out.find(trail).expect("marker present");
        assert!(
            prefix_end <= 512,
            "truncated prefix exceeded 512B: {prefix_end}"
        );
    }
}
