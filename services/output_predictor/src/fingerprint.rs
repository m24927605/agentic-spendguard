//! Phase B placeholder — full fingerprint arrives in Phase C.
//!
//! Spec ref output-predictor-service-spec-v1alpha1.md §8.2.

/// Fingerprint version stamp written into the response. Per spec §8.2 the
/// fingerprint string itself is also prefixed with this version so v2
/// classifier upgrades can identify legacy rows.
pub const FINGERPRINT_VERSION: &str = "v1alpha1";

/// Phase B stub — Phase C lands the real SHA-256 over canonical template.
/// The stub is deterministic over (model, class) so unit tests that
/// touch the server boot path still see consistent fingerprints.
pub fn compute_fingerprint(model: &str, prompt_class: &str) -> String {
    format!("{FINGERPRINT_VERSION}:stub:{model}:{prompt_class}")
}
