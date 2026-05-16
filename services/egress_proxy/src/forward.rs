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
use thiserror::Error;
use tracing::{debug, info, warn};

use crate::decision::{self, DecisionInputs};
use crate::proto::sidecar_adapter::v1::decision_response::Decision;
use crate::redacted_auth::RedactedAuth;
use crate::AppState;

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

    #[error("missing identification — set SPENDGUARD_PROXY_DEFAULT_* env or X-SpendGuard-* headers")]
    MissingIdentification,

    #[error("upstream HTTP error: {0}")]
    Upstream(#[from] reqwest::Error),

    #[error("upstream returned unexpected Content-Type: {0}")]
    UnexpectedContentType(String),

    // Slice 4c — fail-closed decision routing per spec §4.2
    #[error("SpendGuard sidecar unavailable: {0}")]
    SidecarUnavailable(String),

    #[error("SpendGuard blocked: {reason_codes:?}")]
    Blocked {
        decision_id: String,
        reason_codes: Vec<String>,
        matched_rule_ids: Vec<String>,
    },

    #[error("SpendGuard returned unsupported decision (REQUIRE_APPROVAL/DEGRADE): {reason_codes:?}")]
    UnsupportedDecision {
        decision_id: String,
        reason_codes: Vec<String>,
    },

    #[error("SpendGuard decision SKIP: this trigger boundary skipped")]
    Skipped { decision_id: String },

    #[error("internal: {0}")]
    Internal(String),
}

impl IntoResponse for ForwardError {
    fn into_response(self) -> Response {
        let (status, code, message, retry_after, extra_details): (
            StatusCode,
            &'static str,
            String,
            Option<&'static str>,
            Option<Value>,
        ) = match &self {
            Self::BodyTooLarge { .. } => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "spendguard_body_too_large",
                self.to_string(),
                None,
                None,
            ),
            Self::MalformedJson(_) => (
                StatusCode::BAD_REQUEST,
                "spendguard_malformed_json",
                self.to_string(),
                None,
                None,
            ),
            Self::StreamingUnsupported => (
                StatusCode::NOT_IMPLEMENTED,
                "spendguard_streaming_unsupported",
                "set stream=false until v0.2".to_string(),
                None,
                None,
            ),
            Self::MissingAuth => (
                StatusCode::UNAUTHORIZED,
                "spendguard_missing_authorization",
                self.to_string(),
                None,
                None,
            ),
            Self::MissingIdentification => (
                StatusCode::BAD_REQUEST,
                "spendguard_missing_identification",
                self.to_string(),
                None,
                None,
            ),
            Self::Upstream(_) => (
                StatusCode::BAD_GATEWAY,
                "spendguard_upstream_failure",
                self.to_string(),
                None,
                None,
            ),
            Self::UnexpectedContentType(_) => (
                StatusCode::BAD_GATEWAY,
                "spendguard_unexpected_streaming_response",
                self.to_string(),
                None,
                None,
            ),
            Self::SidecarUnavailable(_) => (
                StatusCode::BAD_GATEWAY,
                "spendguard_sidecar_unavailable",
                self.to_string(),
                None,
                None,
            ),
            Self::Blocked {
                decision_id,
                reason_codes,
                matched_rule_ids,
            } => (
                StatusCode::TOO_MANY_REQUESTS,
                "spendguard_blocked",
                "request blocked by SpendGuard policy".to_string(),
                // Spec §3.3: hard-cap STOP gets Retry-After: 86400.
                // openai-python's clamp (~60s) renders this informational.
                // Real retry control is client-side max_retries=0.
                Some("86400"),
                Some(json!({
                    "decision_id": decision_id,
                    "reason_codes": reason_codes,
                    "matched_rule_ids": matched_rule_ids,
                })),
            ),
            Self::UnsupportedDecision {
                decision_id,
                reason_codes,
            } => (
                StatusCode::SERVICE_UNAVAILABLE,
                "spendguard_unsupported_decision",
                "egress-proxy mode does not support REQUIRE_APPROVAL/DEGRADE; use SDK wrapper".to_string(),
                None,
                Some(json!({
                    "decision_id": decision_id,
                    "reason_codes": reason_codes,
                    "hint": "use SDK wrapper for approval / degrade flows",
                })),
            ),
            Self::Skipped { decision_id } => (
                StatusCode::TOO_MANY_REQUESTS,
                "spendguard_skipped",
                "SpendGuard returned SKIP for this trigger boundary".to_string(),
                None,
                Some(json!({"decision_id": decision_id})),
            ),
            Self::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "spendguard_internal_error",
                self.to_string(),
                None,
                None,
            ),
        };

        let mut error_obj = json!({
            "code": code,
            "type": code,
            "message": message,
        });
        if let (Some(details), Some(obj)) = (extra_details, error_obj.as_object_mut()) {
            obj.insert("details".to_string(), details);
        }
        let body = Json(json!({"error": error_obj}));

        let mut builder = axum::response::Response::builder().status(status);
        if let Some(retry) = retry_after {
            builder = builder.header("Retry-After", retry);
        }
        builder = builder.header(axum::http::header::CONTENT_TYPE, "application/json");
        builder.body(axum::body::Body::from(serde_json::to_vec(&body.0).unwrap())).unwrap()
    }
}

/// POST /v1/chat/completions handler.
///
/// Slice 3: forward byte-identically to OpenAI. NO SpendGuard gating
/// (slice 4 adds the sidecar UDS call before this forward).
pub async fn chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ForwardError> {
    // Slice 3 path: just forwards. Slice 4c will branch on sidecar
    // decision BEFORE this forward block runs.
    let state = &state.forward;
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

/// Header helpers used by slice 4c routing. Slice 6 will refactor
/// these into a dedicated identification module with full env-default
/// + override semantics.

fn header_int(headers: &HeaderMap, name: &str) -> Option<i64> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())
}

fn resolve_budget_id(headers: &HeaderMap) -> String {
    headers
        .get("x-spendguard-budget-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .or_else(|| std::env::var("SPENDGUARD_PROXY_DEFAULT_BUDGET_ID").ok())
        .unwrap_or_default()
}

fn resolve_window_instance_id(headers: &HeaderMap) -> String {
    headers
        .get("x-spendguard-window-instance-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .or_else(|| std::env::var("SPENDGUARD_PROXY_DEFAULT_WINDOW_INSTANCE_ID").ok())
        .unwrap_or_default()
}

fn resolve_unit_id(headers: &HeaderMap) -> String {
    headers
        .get("x-spendguard-unit-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .or_else(|| std::env::var("SPENDGUARD_PROXY_DEFAULT_UNIT_ID").ok())
        .unwrap_or_default()
}

/// Allowlist of request headers forwarded to OpenAI.
///
/// Codex slice-3 r1 P2-S3.B fix: explicit denylist (defensive vs the
/// allowlist growing to include `authorization` in a future PR that
/// would silently bypass the single-expose_secret invariant).
fn should_forward_header(name: &HeaderName) -> bool {
    let lower = name.as_str().to_ascii_lowercase();
    // Explicit deny: even if a future allowlist entry would match,
    // these are NEVER forwarded by this function:
    // - authorization: forwarded ONLY at the explicit RedactedAuth.expose_secret()
    //   boundary, not via generic header iteration
    // - host / content-length / content-type: reqwest sets these itself
    if matches!(
        lower.as_str(),
        "authorization" | "host" | "content-length" | "content-type"
    ) {
        return false;
    }
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
