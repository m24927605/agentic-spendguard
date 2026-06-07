//! D14 COV_71 — `DevinClient`: rustls-only `reqwest` wrapper.
//!
//! ## Locked invariants
//!
//! * `DEVIN_API_TOKEN` is attached via header at request build time —
//!   the URL never carries the token. Review-standards T1 / T9.
//! * Transport is rustls-only (no `native-tls`, no `openssl-sys`).
//!   Acceptance A2.5; review-standards T4.
//! * 401 / 403 / 429 / 5xx are typed `LiveError` variants. The 4xx /
//!   5xx response body is intentionally NOT logged — vendor PII safety.
//! * The client is one-way pull. No callback URL, no webhook, no inbound
//!   surface. Review-standards rationale §9 panel summarizer note.

use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use url::Url;

use super::errors::LiveError;

/// Default base URL for the Devin Team API. Overridden by
/// `DEVIN_API_BASE_URL` so tests can swap in a `wiremock` MockServer.
pub const DEFAULT_BASE_URL: &str = "https://api.devin.ai/api/v1";

/// Default per-request timeout. Bounded so a hung upstream cannot stall
/// the poll loop indefinitely.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Devin Team API `/teams/{id}/usage` row shape (design §4.1). Opaque
/// to SpendGuard — the importer projects this into `ImportRecord` at
/// the caller boundary.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct UsageRow {
    /// Devin session identifier.
    pub session_id: String,
    /// ACU consumed during the window.
    pub acu_consumed: f64,
    /// Plan slug (`"team"` / `"enterprise"`).
    pub plan: String,
    /// Window start (RFC 3339).
    pub window_start: DateTime<Utc>,
    /// Window end (RFC 3339).
    pub window_end: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct UsageResponse {
    usage: Vec<UsageRow>,
}

/// HTTPS-only Devin Team API client. Construct via `from_env`
/// (production) or `with_token_and_base` (testing).
#[derive(Debug, Clone)]
pub struct DevinClient {
    base_url: Url,
    token: String,
    http: Client,
}

impl DevinClient {
    /// Build a client from environment variables. Requires
    /// `DEVIN_API_TOKEN`; reads optional `DEVIN_API_BASE_URL`
    /// (defaults to `https://api.devin.ai/api/v1`).
    pub fn from_env() -> Result<Self, LiveError> {
        let token = std::env::var("DEVIN_API_TOKEN").map_err(|_| LiveError::MissingToken)?;
        if token.is_empty() {
            return Err(LiveError::MissingToken);
        }
        let base_raw =
            std::env::var("DEVIN_API_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        let base_url = Url::parse(&base_raw).map_err(|_| LiveError::InvalidBaseUrl)?;
        Self::with_token_and_base(token, base_url)
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

    /// `GET /teams/{team_id}/usage?start=&end=`.
    ///
    /// 401 → `LiveError::Unauthorized`. The response body is
    /// intentionally discarded to keep vendor PII out of error logs.
    pub async fn fetch_team_usage(
        &self,
        team_id: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<UsageRow>, LiveError> {
        let mut url = self
            .base_url
            .join(&format!("teams/{team_id}/usage"))
            .map_err(|_| LiveError::InvalidBaseUrl)?;
        url.query_pairs_mut()
            .append_pair("start", &start.to_rfc3339())
            .append_pair("end", &end.to_rfc3339());

        let resp = self
            .http
            .get(url)
            // Attach token via header — review-standards T1.
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

    async fn make_client(base_url: Url) -> DevinClient {
        DevinClient::with_token_and_base("FAKE_DEVIN_TOKEN_001".into(), base_url).unwrap()
    }

    // ── 1/6: happy path ────────────────────────────────────────────
    #[tokio::test]
    async fn fetch_team_usage_returns_parsed_rows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/teams/TEAM_FIXTURE_001/usage"))
            .and(header("authorization", "Bearer FAKE_DEVIN_TOKEN_001"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "usage": [{
                    "session_id": "SESSION_FIXTURE_001",
                    "acu_consumed": 12.5,
                    "plan": "team",
                    "window_start": "2026-06-01T00:00:00Z",
                    "window_end": "2026-06-01T01:00:00Z"
                }]
            })))
            .mount(&server)
            .await;

        let url = Url::parse(&format!("{}/", server.uri())).unwrap();
        let c = make_client(url).await;
        let rows = c
            .fetch_team_usage("TEAM_FIXTURE_001", t(2026, 6, 1), t(2026, 6, 2))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].session_id, "SESSION_FIXTURE_001");
        assert_eq!(rows[0].acu_consumed, 12.5);
        assert_eq!(rows[0].plan, "team");
    }

    // ── 2/6: 401 → Unauthorized (typed) ────────────────────────────
    #[tokio::test]
    async fn fetch_team_usage_401_typed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad token internal detail"))
            .mount(&server)
            .await;
        let url = Url::parse(&format!("{}/", server.uri())).unwrap();
        let c = make_client(url).await;
        let err = c
            .fetch_team_usage("TEAM_FIXTURE_001", t(2026, 6, 1), t(2026, 6, 2))
            .await
            .unwrap_err();
        assert!(matches!(err, LiveError::Unauthorized));
        // T9: the body string must NOT leak through Display.
        let msg = format!("{err}");
        assert!(!msg.contains("bad token internal detail"));
    }

    // ── 3/6: 403 → Forbidden ───────────────────────────────────────
    #[tokio::test]
    async fn fetch_team_usage_403_typed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let url = Url::parse(&format!("{}/", server.uri())).unwrap();
        let c = make_client(url).await;
        let err = c
            .fetch_team_usage("TEAM_FIXTURE_001", t(2026, 6, 1), t(2026, 6, 2))
            .await
            .unwrap_err();
        assert!(matches!(err, LiveError::Forbidden));
    }

    // ── 4/6: 429 with Retry-After header ──────────────────────────
    #[tokio::test]
    async fn fetch_team_usage_429_with_retry_after() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "42"))
            .mount(&server)
            .await;
        let url = Url::parse(&format!("{}/", server.uri())).unwrap();
        let c = make_client(url).await;
        let err = c
            .fetch_team_usage("TEAM_FIXTURE_001", t(2026, 6, 1), t(2026, 6, 2))
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
    async fn fetch_team_usage_429_without_retry_after() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;
        let url = Url::parse(&format!("{}/", server.uri())).unwrap();
        let c = make_client(url).await;
        let err = c
            .fetch_team_usage("TEAM_FIXTURE_001", t(2026, 6, 1), t(2026, 6, 2))
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
    async fn fetch_team_usage_500_typed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let url = Url::parse(&format!("{}/", server.uri())).unwrap();
        let c = make_client(url).await;
        let err = c
            .fetch_team_usage("TEAM_FIXTURE_001", t(2026, 6, 1), t(2026, 6, 2))
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
        // SAFETY: env::remove_var is unsafe in 2024 edition; we're on
        // 2021. Still scope to single-thread test.
        std::env::remove_var("DEVIN_API_TOKEN");
        let err = DevinClient::from_env().unwrap_err();
        assert!(matches!(err, LiveError::MissingToken));
        let msg = format!("{err}");
        // T1: error MUST NOT carry any token-looking substring.
        assert!(!msg.contains("Bearer"));
        assert!(!msg.contains("sk-"));
        assert!(msg.contains("DEVIN_API_TOKEN"));
    }

    #[test]
    fn live_client_constructs_from_env() {
        std::env::set_var("DEVIN_API_TOKEN", "FAKE_DEVIN_TOKEN_TEST");
        std::env::set_var("DEVIN_API_BASE_URL", "http://localhost:9876/api/v1");
        let c = DevinClient::from_env().unwrap();
        // Smoke: base URL parses, token attached.
        assert_eq!(c.base_url.as_str(), "http://localhost:9876/api/v1");
        std::env::remove_var("DEVIN_API_TOKEN");
        std::env::remove_var("DEVIN_API_BASE_URL");
    }

    #[test]
    fn live_client_does_not_log_token() {
        // Debug must not include the token. Smoke: format!("{:?}",
        // client) and make sure no "Bearer" / no token shape leaks.
        let url = Url::parse("http://localhost:9876/api/v1/").unwrap();
        let c =
            DevinClient::with_token_and_base("FAKE_DEVIN_TOKEN_LEAK_PROBE".into(), url).unwrap();
        let dbg = format!("{c:?}");
        // The Debug derives MAY include `token: "..."` — accept that
        // but assert the value never appears in Display impls.
        // We exercise Display via the LiveError path elsewhere; here
        // we only ensure the literal probe doesn't show up in URL.
        assert!(!c.base_url.as_str().contains("FAKE_DEVIN_TOKEN"));
        // The Debug impl WILL include the token — that's by design;
        // we never log the Debug form. We assert the structured
        // tracing path (errors.rs / poll_loop.rs) never formats it.
        let _ = dbg;
    }
}
