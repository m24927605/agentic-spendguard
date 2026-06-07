//! Typed `LiveError`s for the Manus admin REST client.
//!
//! Review-standards §1:
//!
//! * `T1` — `Display` MUST NOT leak the bearer token value.
//!   `MissingToken` only names the env var (`"MANUS_API_TOKEN"`),
//!   never its contents.
//! * `L4` — 401 / 403 / 429 / 5xx surface as structured variants, NOT
//!   `anyhow::Error` strings that might include the response body.

use reqwest::StatusCode;

/// Failures surfaced by `ManusClient`. Display impls are sanitized:
/// they reveal the failure class but NEVER the bearer token or the
/// response body.
#[derive(Debug, thiserror::Error)]
pub enum LiveError {
    /// `MANUS_API_TOKEN` environment variable is not set or empty.
    /// The error only names the env var, never its value.
    #[error("MANUS_API_TOKEN environment variable not set; set it to a Team+ admin API token before running with --mode live")]
    MissingToken,
    /// `MANUS_API_BASE_URL` could not be parsed.
    #[error("MANUS_API_BASE_URL is not a valid URL")]
    InvalidBaseUrl,
    /// 401 — bad token. Body is intentionally discarded so vendor PII
    /// in the error response cannot leak via logs.
    #[error("Manus API rejected the bearer token (401 Unauthorized); rotate MANUS_API_TOKEN")]
    Unauthorized,
    /// 403 — token authenticated but lacks Team+ admin scope.
    #[error(
        "Manus API forbade the request (403 Forbidden); scope MANUS_API_TOKEN to Team+ admin"
    )]
    Forbidden,
    /// 429 — rate limited. `retry_after_secs` is `0` if header absent.
    #[error("Manus API rate-limited (429); retry after {retry_after_secs}s")]
    RateLimited {
        /// Seconds before retry. `0` if no `Retry-After` header.
        retry_after_secs: u32,
    },
    /// 5xx — upstream failure.
    #[error("Manus API upstream failure ({status})")]
    Upstream {
        /// Returned status code.
        status: StatusCode,
    },
    /// Transport-level failure (DNS, TLS handshake, timeout, etc.).
    /// We intentionally NEVER include the URL or request body in the
    /// error chain.
    #[error("Manus API transport failure: {kind}")]
    Transport {
        /// One of the high-level failure classes; the underlying
        /// reqwest error is preserved as `source` for tracing /
        /// debugging but is not formatted into `Display`.
        kind: TransportKind,
        /// Underlying transport error chain.
        #[source]
        source: reqwest::Error,
    },
    /// The pagination cursor loop exceeded its hard upper bound — a
    /// vendor-misbehaving `next_cursor` that would loop forever.
    /// Review-standards L2.
    #[error("Manus API cursor pagination exceeded hard cap of {cap} pages")]
    CursorCapExceeded {
        /// The hard cap that was tripped.
        cap: usize,
    },
}

/// Coarse classification of `reqwest::Error`. Exposes only the shape —
/// never the URL or response body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    /// Connection-level failure (DNS, TCP, TLS).
    Connect,
    /// Request body / headers serialization failure.
    Encode,
    /// Response body decode / JSON parse failure.
    Decode,
    /// Server hung up or read timeout.
    TimeoutOrIo,
}

impl std::fmt::Display for TransportKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Connect => "connect",
            Self::Encode => "encode",
            Self::Decode => "decode",
            Self::TimeoutOrIo => "timeout_or_io",
        };
        f.write_str(s)
    }
}

impl From<reqwest::Error> for LiveError {
    fn from(err: reqwest::Error) -> Self {
        let kind = if err.is_connect() {
            TransportKind::Connect
        } else if err.is_decode() {
            TransportKind::Decode
        } else if err.is_timeout() {
            TransportKind::TimeoutOrIo
        } else {
            TransportKind::Encode
        };
        LiveError::Transport { kind, source: err }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_token_never_leaks_value() {
        // Sanity: the Display string mentions the env var NAME but
        // never any plausible token shape. Defensive against future
        // refactors that try to interpolate the value.
        let msg = format!("{}", LiveError::MissingToken);
        assert!(msg.contains("MANUS_API_TOKEN"));
        assert!(!msg.contains("Bearer"));
        assert!(!msg.contains("sk-"));
        assert!(!msg.contains("Token "));
    }

    #[test]
    fn rate_limited_carries_retry_after_only() {
        let msg = format!(
            "{}",
            LiveError::RateLimited {
                retry_after_secs: 30
            },
        );
        assert!(msg.contains("30s"));
        assert!(msg.contains("429"));
    }

    #[test]
    fn unauthorized_does_not_leak_token_or_body() {
        let msg = format!("{}", LiveError::Unauthorized);
        assert!(!msg.contains("Bearer"));
        assert!(!msg.contains("body"));
        assert!(msg.contains("401"));
    }

    #[test]
    fn forbidden_message_is_actionable() {
        let msg = format!("{}", LiveError::Forbidden);
        assert!(msg.contains("403"));
        assert!(msg.contains("MANUS_API_TOKEN"));
    }

    #[test]
    fn cursor_cap_exceeded_reports_cap() {
        let msg = format!("{}", LiveError::CursorCapExceeded { cap: 10_000 });
        assert!(msg.contains("10000"));
    }

    #[test]
    fn transport_kind_display_strings_locked() {
        assert_eq!(format!("{}", TransportKind::Connect), "connect");
        assert_eq!(format!("{}", TransportKind::Encode), "encode");
        assert_eq!(format!("{}", TransportKind::Decode), "decode");
        assert_eq!(format!("{}", TransportKind::TimeoutOrIo), "timeout_or_io");
    }
}
