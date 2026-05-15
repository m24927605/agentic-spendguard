//! HTTP pass-through forwarder to upstream LLM provider (OpenAI v0.1).
//!
//! Slice 3 deliverable: accept `POST /v1/chat/completions`, forward
//! byte-identically to `https://api.openai.com/v1/chat/completions`
//! using reqwest. NO SpendGuard gating (slice 4 wires that).
//!
//! Spec §3.2, §3.4, §3.3 (CONTINUE + upstream errors).
//!
//! Spec invariants enforced here:
//! - Body byte-identity (no mutation; reqwest receives the body bytes
//!   we received from the client).
//! - Authorization byte-identity (wrapped in RedactedAuth, forwarded
//!   via expose_secret() at the single call site).
//! - `stream: true` → 501 (codex slice-1 r2 P2-r2.B: detect both
//!   pre-call from body AND post-response from Content-Type).
//! - Upstream Content-Type is verified to be application/json before
//!   returning (text/event-stream → 502 unexpected-streaming).
//! - Body size limit 16 MB per spec §10.

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::sync::Arc;
use thiserror::Error;
use tracing::{debug, info, warn};

use crate::redacted_auth::RedactedAuth;

const UPSTREAM_URL: &str = "https://api.openai.com/v1/chat/completions";
const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone)]
pub struct ForwardState {
    pub http_client: reqwest::Client,
}

impl ForwardState {
    pub fn new() -> Result<Self, anyhow::Error> {
        let http_client = reqwest::Client::builder()
            .user_agent(format!(
                "spendguard-egress-proxy/{}",
                env!("CARGO_PKG_VERSION")
            ))
            // Connect + total timeouts. v0.1 hard-codes; v0.2 makes
            // configurable.
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(120))
            .build()?;
        Ok(Self { http_client })
    }
}

#[derive(Error, Debug)]
pub enum ForwardError {
    #[error("body too large ({size} bytes > {limit} max)")]
    BodyTooLarge { size: usize, limit: usize },

    #[error("malformed JSON body: {0}")]
    MalformedJson(String),

    #[error("streaming requests (stream=true) unsupported in v0.1")]
    StreamingUnsupported,

    #[error("missing Authorization header")]
    MissingAuth,

    #[error("upstream HTTP error: {0}")]
    Upstream(#[from] reqwest::Error),

    #[error("upstream returned unexpected Content-Type: {0}")]
    UnexpectedContentType(String),
}

impl IntoResponse for ForwardError {
    fn into_response(self) -> Response {
        let (status, code, message) = match &self {
            Self::BodyTooLarge { .. } => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "spendguard_body_too_large",
                self.to_string(),
            ),
            Self::MalformedJson(_) => (
                StatusCode::BAD_REQUEST,
                "spendguard_malformed_json",
                self.to_string(),
            ),
            Self::StreamingUnsupported => (
                StatusCode::NOT_IMPLEMENTED,
                "spendguard_streaming_unsupported",
                "set stream=false until v0.2".to_string(),
            ),
            Self::MissingAuth => (
                StatusCode::UNAUTHORIZED,
                "spendguard_missing_authorization",
                self.to_string(),
            ),
            Self::Upstream(_) => (
                StatusCode::BAD_GATEWAY,
                "spendguard_upstream_failure",
                self.to_string(),
            ),
            Self::UnexpectedContentType(_) => (
                StatusCode::BAD_GATEWAY,
                "spendguard_unexpected_streaming_response",
                self.to_string(),
            ),
        };
        let body = Json(json!({
            "error": {
                "code": code,
                "type": code,
                "message": message,
            }
        }));
        (status, body).into_response()
    }
}

/// POST /v1/chat/completions handler.
///
/// Slice 3: forward byte-identically to OpenAI. NO SpendGuard gating
/// (slice 4 adds the sidecar UDS call before this forward).
pub async fn chat_completions(
    State(state): State<Arc<ForwardState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ForwardError> {
    // 16 MB body limit (spec §9).
    if body.len() > MAX_BODY_BYTES {
        return Err(ForwardError::BodyTooLarge {
            size: body.len(),
            limit: MAX_BODY_BYTES,
        });
    }

    // Parse body to inspect `stream` field. We don't modify it.
    let parsed: Value =
        serde_json::from_slice(&body).map_err(|e| ForwardError::MalformedJson(e.to_string()))?;
    if parsed.get("stream").and_then(|v| v.as_bool()).unwrap_or(false) {
        return Err(ForwardError::StreamingUnsupported);
    }

    // Extract + wrap Authorization. Per spec §3.4: forwarded byte-identical.
    let auth = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(RedactedAuth::new)
        .ok_or(ForwardError::MissingAuth)?;

    // Forward to OpenAI. We use reqwest's `bytes()` body to preserve
    // byte-identity (no serde re-encode in the request path).
    let mut req = state
        .http_client
        .post(UPSTREAM_URL)
        .header(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )
        .body(body.clone());

    // The ONLY call site of expose_secret() — codex audit grep
    // target. RedactedAuth's compile-time guarantee depends on this
    // being the single boundary.
    req = req.header("Authorization", auth.expose_secret());

    // Pass through OpenAI-specific headers (organization, project,
    // beta, etc.). Strict allowlist to avoid leaking SpendGuard
    // internal headers upstream.
    for (name, value) in &headers {
        if should_forward_header(name) {
            req = req.header(name, value);
        }
    }

    debug!(upstream = UPSTREAM_URL, body_bytes = body.len(), "forwarding to OpenAI");

    let resp = req.send().await?;
    let upstream_status = resp.status();
    let upstream_headers = resp.headers().clone();
    let upstream_body = resp.bytes().await?;

    // Codex slice-1 r2 P2-r2.B: verify upstream Content-Type before
    // returning. SSE upgrades (even with stream:false in request)
    // would break downstream usage parsing.
    if let Some(ct) = upstream_headers.get(axum::http::header::CONTENT_TYPE) {
        let ct_str = ct.to_str().unwrap_or("");
        if ct_str.starts_with("text/event-stream") {
            warn!(content_type = ct_str, "upstream returned SSE unexpectedly");
            return Err(ForwardError::UnexpectedContentType(ct_str.to_string()));
        }
    }

    info!(
        upstream_status = upstream_status.as_u16(),
        upstream_body_bytes = upstream_body.len(),
        "forwarded"
    );

    // Build response with upstream status + content-type.
    let mut response = Response::builder().status(upstream_status);
    if let Some(ct) = upstream_headers.get(axum::http::header::CONTENT_TYPE) {
        response = response.header(axum::http::header::CONTENT_TYPE, ct);
    }
    Ok(response
        .body(axum::body::Body::from(upstream_body))
        .unwrap())
}

/// Allowlist of request headers forwarded to OpenAI.
///
/// Skip:
/// - host / content-length (reqwest computes)
/// - x-spendguard-* (internal, slice 6 reads these but they don't go upstream)
/// - authorization (forwarded via explicit RedactedAuth boundary above)
fn should_forward_header(name: &HeaderName) -> bool {
    let lower = name.as_str().to_ascii_lowercase();
    if lower.starts_with("x-spendguard-") {
        return false;
    }
    matches!(
        lower.as_str(),
        // OpenAI-recognized headers (non-exhaustive; expand as needed)
        "openai-organization"
            | "openai-project"
            | "openai-beta"
            | "user-agent"
            | "accept"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_forward_header_allows_openai_org() {
        let h = HeaderName::from_static("openai-organization");
        assert!(should_forward_header(&h));
    }

    #[test]
    fn should_forward_header_blocks_x_spendguard() {
        let h = HeaderName::from_static("x-spendguard-tenant-id");
        assert!(!should_forward_header(&h));
    }

    #[test]
    fn should_forward_header_blocks_unknown() {
        let h = HeaderName::from_static("x-internal-token");
        assert!(!should_forward_header(&h));
    }

    #[test]
    fn body_too_large_renders_413() {
        let err = ForwardError::BodyTooLarge {
            size: 100,
            limit: 50,
        };
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn streaming_unsupported_renders_501() {
        let err = ForwardError::StreamingUnsupported;
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);
    }

    #[test]
    fn malformed_json_renders_400() {
        let err = ForwardError::MalformedJson("trailing comma".into());
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn upstream_error_renders_502() {
        // Build a fake reqwest error by attempting to GET an invalid URL.
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let err = runtime
            .block_on(async {
                reqwest::Client::new()
                    .get("not-a-url")
                    .send()
                    .await
                    .unwrap_err()
            });
        let resp: Response = ForwardError::Upstream(err).into_response();
        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn unexpected_content_type_renders_502() {
        let err = ForwardError::UnexpectedContentType("text/event-stream".into());
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    }
}
