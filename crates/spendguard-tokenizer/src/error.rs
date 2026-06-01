//! Tokenizer error types.
//!
//! Spec ref `tokenizer-service-spec-v1alpha1.md` §8 failure modes.
//! All variants are non-recoverable from the caller's point of view
//! except for `UnknownModel` (which the caller never sees because
//! it's mapped to Tier 3 fallback at the dispatch boundary).

use thiserror::Error;

/// The single error type returned by [`crate::Tokenizer`] methods.
#[derive(Debug, Error)]
pub enum TokenizerError {
    /// Asset signature / sha256 check failed at boot. Maps to the
    /// per-spec §7.4 fail-fast requirement. Tokenizer service refuses
    /// to start in this state.
    #[error(
        "asset integrity check failed for encoder `{encoder}`: \
         expected sha256 {expected}, got {actual}"
    )]
    AssetSignatureMismatch {
        encoder: &'static str,
        expected: &'static str,
        actual: String,
    },

    /// Underlying BPE library returned an error during encode. Per
    /// spec §8 "Tier 2 encoder panic during tokenize" — the sidecar
    /// should map this to fail-closed reservation rather than
    /// silently fall back to Tier 3 (panic may indicate input
    /// anomaly that needs escalation).
    #[error("encoder error for kind `{kind}`: {message}")]
    EncoderInternal { kind: &'static str, message: String },

    /// Dispatch table failed to compile its regex patterns. This is
    /// a programmer error (only triggered if a SLICE_04+ contributor
    /// breaks a pattern); never expected in production.
    #[error("dispatch table pattern `{pattern}` failed to compile: {source}")]
    DispatchPatternInvalid {
        pattern: String,
        #[source]
        source: regex::Error,
    },

    /// The requested encoder asset failed to load. Per spec §8
    /// "Tier 2 encoder load failure → refuse to start"; the service
    /// surfaces this as fail-fast at construction time.
    #[error("encoder asset load failed for `{encoder}`: {message}")]
    AssetLoadFailed {
        encoder: &'static str,
        message: String,
    },
}
