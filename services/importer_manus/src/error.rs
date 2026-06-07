//! D15 — Typed errors for the Manus importer.
//!
//! Two error families:
//!
//! * [`ImporterError`] — wraps fixture loader / live-client failures.
//! * [`MeterError`] — narrow pricing-conversion failures
//!   (review-standards T6, T13: unknown-tier skip + WARN, NEVER fabricate
//!   a USD amount; saturating overflow guard).
//!
//! Display impls are sanitized: they reveal the failure class but
//! NEVER the bearer token (review-standards T1), the response body
//! (review-standards L4), or PII / customer identifiers (T8).

use crate::record::Tier;

/// Top-level failures the importer pipeline can surface.
#[derive(Debug, thiserror::Error)]
pub enum ImporterError {
    /// I/O failure (typically reading a fixture file from disk).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Underlying JSON parse failure.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// `MANUS_API_TOKEN` environment variable is unset / empty.
    ///
    /// The `Display` only names the env var, never its value
    /// (review-standards T1).
    #[error("MANUS_API_TOKEN environment variable not set; set it to a Team+ admin API token before running with --mode live")]
    MissingToken,

    /// `MANUS_API_BASE_URL` could not be parsed.
    #[error("MANUS_API_BASE_URL is not a valid URL")]
    InvalidBaseUrl,

    /// A fixture record carried a tier this importer does not know
    /// (review-standards T6 — skip + WARN, never fabricate).
    #[error("unknown Manus tier '{0}' (skipped); update assets/price_table.toml")]
    UnknownTier(String),

    /// A fixture record carried a session status this importer does
    /// not know.
    #[error("unknown Manus session status '{0}'")]
    UnknownStatus(String),

    /// A fixture record carried `credits_consumed < 0`.
    #[error("credits_consumed must be non-negative")]
    NegativeCredits,

    /// Pricing conversion failure.
    #[error(transparent)]
    Meter(#[from] MeterError),
}

/// Pricing-conversion failures. Review-standards T6 / T13: NEVER
/// fabricate a USD amount; ALWAYS use saturating arithmetic.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum MeterError {
    /// The tier was not declared in the embedded price table.
    /// Caller emits a WARN and skips the row.
    #[error("unknown tier in price table: {0:?}")]
    UnknownTier(Tier),

    /// The saturating multiply produced a negative result — only
    /// possible when `credits_consumed < 0` slipped through fixture
    /// validation. Defensive belt-and-suspenders (review-standards T13).
    #[error("conversion produced a negative amount; credits or rate corrupted")]
    NegativeAmount,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_token_never_leaks_value() {
        // T1: error MUST NOT carry any token-looking substring.
        let msg = format!("{}", ImporterError::MissingToken);
        assert!(msg.contains("MANUS_API_TOKEN"));
        assert!(!msg.contains("Bearer"));
        assert!(!msg.contains("sk-"));
        assert!(!msg.contains("Token "));
    }

    #[test]
    fn unknown_tier_meter_error_displays_safely() {
        let err = MeterError::UnknownTier(Tier::TeamPlan);
        let msg = format!("{err}");
        assert!(msg.contains("TeamPlan"));
        assert!(!msg.contains("Bearer"));
    }

    #[test]
    fn negative_amount_meter_error_is_terse() {
        let msg = format!("{}", MeterError::NegativeAmount);
        assert!(msg.contains("negative"));
        // Defensive: no token shape.
        assert!(!msg.contains("sk-"));
    }

    #[test]
    fn invalid_base_url_does_not_leak_input() {
        // T1 spirit: don't echo a (possibly token-laden) input string.
        let msg = format!("{}", ImporterError::InvalidBaseUrl);
        assert!(msg.contains("MANUS_API_BASE_URL"));
        assert!(!msg.contains("Bearer"));
    }
}
