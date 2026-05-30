//! Phase A placeholder — full implementation in Phase C.
//!
//! Provider HTTP clients for Anthropic + Gemini count_tokens APIs.
//! Per spec §4 the shadow worker calls these asynchronously OFF the
//! hot path; circuit breaker integration lives in Phase D.
//!
//! See `tokenizer-service-spec-v1alpha1.md` §4.

pub mod anthropic;
pub mod gemini;

/// Phase C wires real provider error variants. Phase A just claims the
/// type so the worker stub can reference it.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider request timed out")]
    Timeout,
    #[error("provider schema unexpected: {0}")]
    Schema(String),
    #[error("provider auth failed: {0}")]
    Auth(String),
    #[error("provider rate-limited; retry after {retry_after:?}")]
    RateLimit {
        retry_after: std::time::Duration,
    },
    #[error("provider unexpected: {0}")]
    Other(String),
}
