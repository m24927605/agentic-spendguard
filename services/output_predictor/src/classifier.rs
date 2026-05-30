//! Phase B placeholder — full classifier arrives in Phase C.
//!
//! Spec ref output-predictor-service-spec-v1alpha1.md §8.1.

/// Classifier version stamp written into the response so calibration-report
/// + verify-chain capture classifier-version-at-decision. Bump on rule
/// changes; tag the audit row "v1" prefix per spec §8.4.
pub const CLASSIFIER_VERSION: &str = "v1alpha1";
