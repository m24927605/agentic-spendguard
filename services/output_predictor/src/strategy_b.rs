//! Phase B placeholder — full Strategy B arrives in Phase D.
//!
//! Spec ref output-predictor-service-spec-v1alpha1.md §4 + §7.

use crate::cache::OutputDistributionCache;

/// Carries the strategy-B value, its confidence/sample-size, and the
/// cold-start layer the lookup landed in.
#[derive(Debug, Clone)]
pub struct PredictionB {
    pub value: i64,
    pub confidence: f32,
    pub sample_size: i32,
    /// Layer label per spec §7.1:
    ///   Some("L4") — cache hit + sample_size_30d >= 30
    ///   Some("L2") — TOML hit (SLICE_08+; SLICE_06 never returns this)
    ///   Some("L3") — federated (deferred per spec §2.2; SLICE_06 never returns this)
    ///   None       — sentinel; should never happen for `Some(PredictionB)`.
    /// L1 is represented by `compute_b` returning `None` itself.
    pub layer: Option<String>,
}

/// Phase B stub — returns None unconditionally. Phase D wires the real
/// cache lookup + L1/L4 cold-start chain. The cache parameter is held
/// so the signature is stable across phases.
pub async fn compute_b(
    _cache: &OutputDistributionCache,
    _tenant_id: uuid::Uuid,
    _model: &str,
    _agent_id: &str,
    _prompt_class: &str,
) -> Option<PredictionB> {
    None
}
