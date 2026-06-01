//! Provider HTTP clients for Tier 1 count_tokens APIs.
//!
//! Spec refs:
//!   - `tokenizer-service-spec-v1alpha1.md` §4 (Tier 1 shadow architecture)
//!   - `tokenizer-service-spec-v1alpha1.md` §4.5 (circuit breaker — these
//!     clients report into the breaker via the typed error variants)
//!
//! ## Error mapping → circuit breaker
//!
//! Phase D maps these variants:
//!   * `Timeout` / `Other` (5xx / connection refused) → failure count++
//!   * `Schema` → drop the sample but DON'T trip the breaker (likely
//!     vendor API drift; spec §7 "Tier 1 provider returns different
//!     schema" → emit metric + skip sample)
//!   * `Auth` → fail closed; alert operator (mis-rotated key)
//!   * `RateLimit` → backoff respect; failure count++ if persistent
//!
//! ## Hot path invariant
//!
//! These clients are constructed once at boot and held inside the
//! shadow worker. They are NEVER reachable from
//! `services/sidecar/` or `services/egress_proxy/`.

pub mod anthropic;
pub mod cohere;
pub mod gemini;
pub mod llama;

use std::time::Duration;

/// Typed errors emitted by both provider clients. The shadow worker
/// (Phase E) inspects the variant to drive circuit-breaker accounting
/// and metric emission.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// Request timed out (5s default per spec §4 latency budget). The
    /// circuit breaker counts this as a failure.
    #[error("provider request timed out")]
    Timeout,
    /// Response did not match the documented count_tokens schema. Most
    /// likely cause is vendor API drift (e.g. Anthropic adding a new
    /// required field). Per spec §7 we emit a
    /// `provider_count_tokens_schema_drift` metric + skip the sample;
    /// the breaker does NOT count this as a failure because re-trying
    /// the same broken vendor response won't help.
    #[error("provider count_tokens schema unexpected: {0}")]
    Schema(String),
    /// Authentication failed. Typically a mis-rotated key. The shadow
    /// worker logs prominently + skips the sample; operators see the
    /// metric and rotate keys.
    #[error("provider auth failed: {0}")]
    Auth(String),
    /// 429 Too Many Requests with a parsed Retry-After hint (or
    /// default 30s if the header was absent / unparseable). The
    /// circuit breaker counts this as a failure so back-off cascades
    /// honour the breaker semantics.
    #[error("provider rate-limited; retry after {retry_after:?}")]
    RateLimit { retry_after: Duration },
    /// Catch-all for other 4xx/5xx / network / DNS / TLS errors. The
    /// circuit breaker counts this as a failure.
    #[error("provider unexpected: {0}")]
    Other(String),
}

impl ProviderError {
    /// True if this variant should bump the circuit breaker's failure
    /// counter. Per the spec §4.5 + §7 semantics: timeout / rate-limit
    /// / other count; schema drift / auth do NOT (they need operator
    /// attention, not auto-recovery).
    pub fn counts_as_breaker_failure(&self) -> bool {
        match self {
            ProviderError::Timeout | ProviderError::RateLimit { .. } | ProviderError::Other(_) => {
                true
            }
            ProviderError::Schema(_) | ProviderError::Auth(_) => false,
        }
    }
}

/// Successful provider count_tokens response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCount {
    /// Tokens reported by the provider's count_tokens API.
    pub input_tokens: u64,
    /// Provider-side request id (Anthropic returns it in
    /// `request-id` header; Gemini in the response `name` field).
    /// Optional because some providers omit it on error responses.
    pub request_id: Option<String>,
    /// Wall latency end-to-end for the provider call (for metrics).
    pub latency: Duration,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breaker_failure_classification() {
        assert!(ProviderError::Timeout.counts_as_breaker_failure());
        assert!(ProviderError::RateLimit {
            retry_after: Duration::from_secs(10)
        }
        .counts_as_breaker_failure());
        assert!(ProviderError::Other("dns".into()).counts_as_breaker_failure());
        assert!(!ProviderError::Schema("x".into()).counts_as_breaker_failure());
        assert!(!ProviderError::Auth("y".into()).counts_as_breaker_failure());
    }
}
