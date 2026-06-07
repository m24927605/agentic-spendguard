//! D16 COV_87 — `GensparkClient`: rustls-only `reqwest` wrapper.
//!
//! ## Locked invariants
//!
//! * `GENSPARK_API_TOKEN` is attached via header at request build
//!   time — the URL never carries the token. Review-standards T1.
//! * `GENSPARK_API_TOKEN` runtime gate rejects (a) absent var, (b)
//!   empty-after-trim, (c) length < 32. Review-standards T2, L5, L6.
//! * Transport is rustls-only (no `native-tls`, no `openssl-sys`).
//!   Review-standards T4.
//! * 401 / 403 / 429 / 5xx are typed `LiveError` variants. The 4xx /
//!   5xx response body is intentionally NOT logged — vendor PII safety.
//! * The client is one-way pull. No callback URL, no webhook, no
//!   inbound surface.

use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use url::Url;

use super::errors::LiveError;

/// Default base URL for the Genspark admin API. Overridden by
/// `GENSPARK_API_BASE_URL` so tests can swap in a `wiremock` MockServer.
pub const DEFAULT_BASE_URL: &str = "https://api.genspark.ai/v1";

/// Default per-request timeout. Bounded so a hung upstream cannot
/// stall the poll loop indefinitely.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Minimum bearer-token length. Locked in D16 design §6 decision #4 —
/// catches placeholder strings like `"TODO"` / `"changeme"`.
pub const MIN_TOKEN_LEN: usize = 32;

/// Genspark admin API `/admin/usage` row shape (design §3). Opaque
/// to SpendGuard — the importer projects this into `ImportRecord` at
/// the caller boundary.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct UsageRow {
    /// Genspark workspace identifier.
    pub workspace_id: String,
    /// Genspark task identifier.
    pub task_id: String,
    /// Credits consumed during the window.
    pub credits_consumed: f64,
    /// Plan slug (`"plus"` / `"pro"` / `"premium"`).
    pub plan: String,
    /// Optional task category (`"research"` / `"code_generation"`).
    #[serde(default)]
    pub task_category: Option<String>,
    /// Window start (RFC 3339).
    pub window_start: DateTime<Utc>,
    /// Window end (RFC 3339).
    pub window_end: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct UsageResponse {
    usage: Vec<UsageRow>,
}

/// HTTPS-only Genspark admin API client. Construct via `from_env`
/// (production) or `with_token_and_base` (testing).
#[derive(Debug, Clone)]
pub struct GensparkClient {
    base_url: Url,
    token: String,
    http: Client,
}

impl GensparkClient {
    /// Build a client from environment variables. Requires
    /// `GENSPARK_API_TOKEN`; reads optional `GENSPARK_API_BASE_URL`
    /// (defaults to `https://api.genspark.ai/v1`).
    ///
    /// Returns distinct error variants for missing / empty / too-short
    /// token so operators can debug a misconfig (T2 / L5).
    pub fn from_env() -> Result<Self, LiveError> {
        let raw_token = std::env::var("GENSPARK_API_TOKEN").map_err(|_| LiveError::MissingToken)?;
        let trimmed = raw_token.trim().to_string();
        if trimmed.is_empty() {
            return Err(LiveError::EmptyToken);
        }
        if trimmed.len() < MIN_TOKEN_LEN {
            return Err(LiveError::TokenTooShort {
                actual: trimmed.len(),
                expected: MIN_TOKEN_LEN,
            });
        }
        let base_raw = std::env::var("GENSPARK_API_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        let base_url = Url::parse(&base_raw).map_err(|_| LiveError::InvalidBaseUrl)?;
        Self::with_token_and_base(trimmed, base_url)
    }

    /// Construct directly. Used by tests to inject the wiremock URL.
    /// Default-feature builds NEVER reach this constructor — the
    /// entire `live::client` module is `cfg(feature = "live")` gated.
    pub fn with_token_and_base(token: String, base_url: Url) -> Result<Self, LiveError> {
        let http = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            // rustls is the only TLS backend the crate ever links —
            // enforced by `reqwest = { default-features = false,
            //                          features = ["json", "rustls-tls"] }`
            // in Cargo.toml.
            .build()?;
        Ok(Self {
            base_url,
            token,
            http,
        })
    }

    /// `GET /admin/usage?workspace=&from=&to=`.
    ///
    /// 401 → `LiveError::Unauthorized`. The response body is
    /// intentionally discarded to keep vendor PII out of error logs.
    pub async fn fetch_workspace_usage(
        &self,
        workspace_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<UsageRow>, LiveError> {
        let mut url = self
            .base_url
            .join("admin/usage")
            .map_err(|_| LiveError::InvalidBaseUrl)?;
        url.query_pairs_mut()
            .append_pair("workspace", workspace_id)
            .append_pair("from", &from.to_rfc3339())
            .append_pair("to", &to.to_rfc3339());

        let resp = self
            .http
            .get(url)
            // Attach token via header — review-standards T1 / L4.
            .bearer_auth(&self.token)
            .send()
            .await?;

        match resp.status() {
            s if s.is_success() => {
                let body: UsageResponse = resp.json().await?;
                Ok(body.usage)
            }
            StatusCode::UNAUTHORIZED => Err(LiveError::Unauthorized),
            StatusCode::FORBIDDEN => Err(LiveError::Forbidden),
            StatusCode::TOO_MANY_REQUESTS => {
                let retry_after_secs = resp
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0);
                Err(LiveError::RateLimited { retry_after_secs })
            }
            s => Err(LiveError::Upstream { status: s }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn t(y: i32, mo: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, 0, 0, 0).unwrap()
    }

    fn token_32() -> String {
        // 32-char synthetic token — meets MIN_TOKEN_LEN.
        "FAKE_GENSPARK_TOKEN_00000000_001a".into()
    }

    async fn make_client(base_url: Url) -> GensparkClient {
        GensparkClient::with_token_and_base(token_32(), base_url).unwrap()
    }

    // ── 1/6: happy path ────────────────────────────────────────────
    #[tokio::test]
    async fn fetch_workspace_usage_returns_parsed_rows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/admin/usage"))
            .and(header(
                "authorization",
                "Bearer FAKE_GENSPARK_TOKEN_00000000_001a",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "usage": [{
                    "workspace_id": "FAKE_ws_001",
                    "task_id": "FAKE_task_001",
                    "credits_consumed": 3200.0,
                    "plan": "plus",
                    "task_category": "research",
                    "window_start": "2026-06-01T00:00:00Z",
                    "window_end": "2026-06-01T01:00:00Z"
                }]
            })))
            .mount(&server)
            .await;

        let url = Url::parse(&format!("{}/", server.uri())).unwrap();
        let c = make_client(url).await;
        let rows = c
            .fetch_workspace_usage("FAKE_ws_001", t(2026, 6, 1), t(2026, 6, 2))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].workspace_id, "FAKE_ws_001");
        assert_eq!(rows[0].task_id, "FAKE_task_001");
        assert_eq!(rows[0].credits_consumed, 3200.0);
        assert_eq!(rows[0].plan, "plus");
    }

    // ── 2/6: 401 → Unauthorized (typed) ────────────────────────────
    #[tokio::test]
    async fn fetch_workspace_usage_401_typed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(401).set_body_string("bad token internal detail"),
            )
            .mount(&server)
            .await;
        let url = Url::parse(&format!("{}/", server.uri())).unwrap();
        let c = make_client(url).await;
        let err = c
            .fetch_workspace_usage("FAKE_ws_001", t(2026, 6, 1), t(2026, 6, 2))
            .await
            .unwrap_err();
        assert!(matches!(err, LiveError::Unauthorized));
        // The body string must NOT leak through Display.
        let msg = format!("{err}");
        assert!(!msg.contains("bad token internal detail"));
    }

    // ── 3/6: 403 → Forbidden ───────────────────────────────────────
    #[tokio::test]
    async fn fetch_workspace_usage_403_typed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let url = Url::parse(&format!("{}/", server.uri())).unwrap();
        let c = make_client(url).await;
        let err = c
            .fetch_workspace_usage("FAKE_ws_001", t(2026, 6, 1), t(2026, 6, 2))
            .await
            .unwrap_err();
        assert!(matches!(err, LiveError::Forbidden));
    }

    // ── 4/6: 429 with Retry-After header ──────────────────────────
    #[tokio::test]
    async fn fetch_workspace_usage_429_with_retry_after() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "42"))
            .mount(&server)
            .await;
        let url = Url::parse(&format!("{}/", server.uri())).unwrap();
        let c = make_client(url).await;
        let err = c
            .fetch_workspace_usage("FAKE_ws_001", t(2026, 6, 1), t(2026, 6, 2))
            .await
            .unwrap_err();
        match err {
            LiveError::RateLimited { retry_after_secs } => {
                assert_eq!(retry_after_secs, 42);
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    // ── 5/6: 429 without Retry-After header ───────────────────────
    #[tokio::test]
    async fn fetch_workspace_usage_429_without_retry_after() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;
        let url = Url::parse(&format!("{}/", server.uri())).unwrap();
        let c = make_client(url).await;
        let err = c
            .fetch_workspace_usage("FAKE_ws_001", t(2026, 6, 1), t(2026, 6, 2))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            LiveError::RateLimited {
                retry_after_secs: 0
            }
        ));
    }

    // ── 6/6: 5xx → Upstream ────────────────────────────────────────
    #[tokio::test]
    async fn fetch_workspace_usage_500_typed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let url = Url::parse(&format!("{}/", server.uri())).unwrap();
        let c = make_client(url).await;
        let err = c
            .fetch_workspace_usage("FAKE_ws_001", t(2026, 6, 1), t(2026, 6, 2))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            LiveError::Upstream { status } if status.as_u16() == 503
        ));
    }

    // ── Security: from_env missing token ──────────────────────────
    #[test]
    fn live_client_missing_token_errors_clearly() {
        std::env::remove_var("GENSPARK_API_TOKEN");
        let err = GensparkClient::from_env().unwrap_err();
        assert!(matches!(err, LiveError::MissingToken));
        let msg = format!("{err}");
        assert!(!msg.contains("Bearer"));
        assert!(!msg.contains("sk-"));
        assert!(msg.contains("GENSPARK_API_TOKEN"));
    }

    #[test]
    fn live_client_empty_token_rejected_distinctly() {
        std::env::set_var("GENSPARK_API_TOKEN", "   ");
        let err = GensparkClient::from_env().unwrap_err();
        assert!(matches!(err, LiveError::EmptyToken));
        std::env::remove_var("GENSPARK_API_TOKEN");
    }

    #[test]
    fn live_client_short_token_rejected_distinctly() {
        std::env::set_var("GENSPARK_API_TOKEN", "TODO");
        let err = GensparkClient::from_env().unwrap_err();
        match err {
            LiveError::TokenTooShort { actual, expected } => {
                assert_eq!(actual, 4);
                assert_eq!(expected, MIN_TOKEN_LEN);
            }
            other => panic!("expected TokenTooShort, got {other:?}"),
        }
        std::env::remove_var("GENSPARK_API_TOKEN");
    }

    #[test]
    fn live_client_constructs_from_env_with_valid_token() {
        // 32-char synthetic token meets MIN_TOKEN_LEN.
        std::env::set_var("GENSPARK_API_TOKEN", "FAKE_GENSPARK_TOKEN_00000000_001a");
        std::env::set_var("GENSPARK_API_BASE_URL", "http://localhost:9876/v1");
        let c = GensparkClient::from_env().unwrap();
        assert_eq!(c.base_url.as_str(), "http://localhost:9876/v1");
        std::env::remove_var("GENSPARK_API_TOKEN");
        std::env::remove_var("GENSPARK_API_BASE_URL");
    }

    #[test]
    fn live_client_token_not_in_base_url() {
        let url = Url::parse("http://localhost:9876/v1/").unwrap();
        let c =
            GensparkClient::with_token_and_base("FAKE_GENSPARK_TOKEN_LEAK_PROBE_001a".into(), url)
                .unwrap();
        assert!(!c.base_url.as_str().contains("FAKE_GENSPARK_TOKEN"));
    }

    #[test]
    fn min_token_len_is_32() {
        // L6: constant pinned at 32. Documented in D16 design §6 #4.
        assert_eq!(MIN_TOKEN_LEN, 32);
    }
}
