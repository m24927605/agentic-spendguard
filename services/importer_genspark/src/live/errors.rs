//! Typed `LiveError`s for the Genspark admin API client.
//!
//! Review-standards §1 / §5:
//!
//! * `T1` — `Display` MUST NOT leak the bearer token value.
//!   `MissingToken` only names the env var (`"GENSPARK_API_TOKEN"`),
//!   never its contents.
//! * `T2` — Distinguishes missing / empty / too-short token cases so
//!   operators can debug the misconfig.
//! * `L5` — 401 / 403 / 429 / 5xx surface as structured variants, NOT
//!   `anyhow::Error` strings that might include the response body.

use reqwest::StatusCode;

/// Failures surfaced by `GensparkClient`. Display impls are sanitized:
/// they reveal the failure class but NEVER the bearer token or the
/// response body.
#[derive(Debug, thiserror::Error)]
pub enum LiveError {
    /// `GENSPARK_API_TOKEN` environment variable is not set. The error
    /// only names the env var, never its value.
    #[error("GENSPARK_API_TOKEN environment variable not set; set it to a Genspark Admin API token before running with --mode live")]
    MissingToken,
    /// `GENSPARK_API_TOKEN` is set but empty (after trim). Distinct
    /// from `MissingToken` to help operators debug.
    #[error("GENSPARK_API_TOKEN is empty after trim; check for accidental whitespace-only value")]
    EmptyToken,
    /// `GENSPARK_API_TOKEN` is shorter than the minimum length (32).
    /// Catches placeholders like `"TODO"` / `"changeme"`.
    #[error("GENSPARK_API_TOKEN is too short ({actual} chars); expected at least {expected}. Did you paste a placeholder?")]
    TokenTooShort {
        /// Observed length.
        actual: usize,
        /// Minimum required length.
        expected: usize,
    },
    /// `GENSPARK_API_BASE_URL` could not be parsed.
    #[error("GENSPARK_API_BASE_URL is not a valid URL")]
    InvalidBaseUrl,
    /// 401 — bad token. Body is intentionally discarded so vendor PII
    /// in the error response cannot leak via logs.
    #[error("Genspark API rejected the bearer token (401 Unauthorized); rotate GENSPARK_API_TOKEN")]
    Unauthorized,
    /// 403 — token authenticated but lacks Admin scope.
    #[error("Genspark API forbade the request (403 Forbidden); scope GENSPARK_API_TOKEN to Admin Usage API")]
    Forbidden,
    /// 429 — rate limited. `retry_after_secs` is `0` if header absent.
    #[error("Genspark API rate-limited (429); retry after {retry_after_secs}s")]
    RateLimited {
        /// Seconds before retry. `0` if no `Retry-After` header.
        retry_after_secs: u32,
    },
    /// 5xx — upstream failure.
    #[error("Genspark API upstream failure ({status})")]
    Upstream {
        /// Returned status code.
        status: StatusCode,
    },
    /// Transport-level failure (DNS, TLS handshake, timeout, etc.).
    /// We intentionally NEVER include the URL or request body in the
    /// error chain.
    #[error("Genspark API transport failure: {kind}")]
    Transport {
        /// One of the high-level failure classes.
        kind: TransportKind,
        /// Underlying transport error chain.
        #[source]
        source: reqwest::Error,
    },
}

/// Coarse classification of `reqwest::Error`. We expose only the
/// shape — never the URL or response body.
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
        let msg = format!("{}", LiveError::MissingToken);
        assert!(msg.contains("GENSPARK_API_TOKEN"));
        assert!(!msg.contains("Bearer"));
        assert!(!msg.contains("sk-"));
        assert!(!msg.contains("Token "));
    }

    #[test]
    fn empty_token_is_distinct_from_missing() {
        let m = format!("{}", LiveError::MissingToken);
        let e = format!("{}", LiveError::EmptyToken);
        assert_ne!(m, e);
        assert!(e.contains("empty"));
    }

    #[test]
    fn too_short_token_names_expected_length() {
        let msg = format!(
            "{}",
            LiveError::TokenTooShort {
                actual: 4,
                expected: 32
            },
        );
        assert!(msg.contains("4 chars"));
        assert!(msg.contains("32"));
        // L6: must reference the expected length so operators can debug.
        assert!(!msg.contains("Bearer"));
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
    fn forbidden_message_documents_scope() {
        let msg = format!("{}", LiveError::Forbidden);
        assert!(msg.contains("403"));
        assert!(msg.contains("Admin"));
    }

    #[test]
    fn invalid_base_url_distinct_from_token_errors() {
        let m = format!("{}", LiveError::InvalidBaseUrl);
        assert!(m.contains("GENSPARK_API_BASE_URL"));
        assert!(!m.contains("Bearer"));
    }
}
