//! Strategy B — SQL P95 lookup with cold-start chain.
//!
//! Spec refs:
//!   * output-predictor-service-spec-v1alpha1.md §4 (cache lookup)
//!   * output-predictor-service-spec-v1alpha1.md §7 (cold-start chain
//!     L4 → L3 → L2 → L1)
//!   * cold-start-baseline-spec-v1alpha1.md §2.5 (lookup algorithm)
//!
//! ## SLICE_06 chain coverage
//!
//! Per slice doc:
//!   * L4 (cache hit + sample_size_30d >= 30): supported
//!   * L3 (federated aggregate): deferred per spec §2.2 — returns None
//!   * L2 (model_default_distribution.toml): deferred to SLICE_08 —
//!     returns None
//!   * L1 (hard fallback): supported — represented by `compute_b`
//!     returning None itself; the audit row writes `cold_start_layer_used
//!     = 'L1'` per spec §7.1.
//!
//! ## Layer codes in PredictionB.layer
//!
//! Per spec §7.1 truth table:
//!   * Some(L4): cache hit → audit `cold_start_layer_used` = NULL
//!   * Some(L3): federated → audit `cold_start_layer_used` = 'L3'
//!   * Some(L2): TOML → audit `cold_start_layer_used` = 'L2'
//!   * None (L1): caller maps to audit `cold_start_layer_used` = 'L1'
//!
//! ## Confidence derivation (spec §4.4)
//!
//! `confidence = min(1.0, sample_size_30d / 200.0)`. Heuristic mapping
//! 200 samples = full confidence; tunable post-calibration-report.

use uuid::Uuid;

use crate::cache::{CacheRow, OutputDistributionCache};

/// Strategy B output. Mirrors the spec §4.2 cache row distilled to the
/// minimum subset the predictor + audit chain need at decision time.
#[derive(Debug, Clone)]
pub struct PredictionB {
    /// P95 token count from the chosen layer. SLICE_06: only L4 paths
    /// populate this (L2/L3 return None).
    pub value: i64,
    /// Per spec §4.4. f32 since the proto `optional float confidence`
    /// field is f32.
    pub confidence: f32,
    /// Per spec §4.4 — sample_size_30d for L4. L2/L3 would carry their
    /// own counts.
    pub sample_size: i32,
    /// Layer label for audit row mapping per spec §7.1. Always Some
    /// when `PredictionB` is itself Some (L4 / L2 / L3); L1 is
    /// represented by `compute_b` returning None.
    pub layer: Option<String>,
}

/// Per spec §4.4 — `min(1.0, n / 200.0)`. 200 samples saturates confidence.
fn derive_confidence(sample_size_30d: i32) -> f32 {
    let raw = (sample_size_30d as f32) / 200.0;
    if raw > 1.0 {
        1.0
    } else if raw < 0.0 {
        0.0
    } else {
        raw
    }
}

/// Compute Strategy B per spec §7 lookup algorithm.
pub async fn compute_b(
    cache: &OutputDistributionCache,
    tenant_id: Uuid,
    model: &str,
    agent_id: &str,
    prompt_class: &str,
) -> Option<PredictionB> {
    // L4: cache row with sufficient samples. The cache lookup itself
    // enforces sample_size_30d >= 30 in the SQL WHERE clause so a Some
    // here is already L4-eligible.
    if let Some(row) = cache.lookup(tenant_id, model, agent_id, prompt_class).await {
        return Some(promote_l4(row));
    }

    // L3: federated aggregate (deferred per spec §2.2). Always None in
    // SLICE_06.
    // if L3_ENABLED { ... } — Phase post-launch

    // L2: model_default_distribution.toml. Deferred to SLICE_08 per
    // slice doc §3. Always None in SLICE_06.
    // if let Some(entry) = MODEL_DEFAULT_DIST.get(model, prompt_class) { ... }

    // L1: hard fallback. compute_b returns None; the caller (server.rs)
    // maps this to `cold_start_layer_used = 'L1'` per spec §7.1.
    None
}

/// Map a L4 cache row to PredictionB. The p95 in the cache row is a
/// REAL (f32) — we round to nearest i64 for the reservation. f32 →
/// i64 conversion saturates at i64::MAX for absurdly large values
/// (defense against accidental DB poisoning).
fn promote_l4(row: CacheRow) -> PredictionB {
    let value_f = row.p95_30d.max(1.0);
    // i64::MAX as f32 is 9.223372e18 — values above that saturate.
    let value = if value_f > i64::MAX as f32 {
        i64::MAX
    } else {
        value_f.round() as i64
    };
    PredictionB {
        value,
        confidence: derive_confidence(row.sample_size_30d),
        sample_size: row.sample_size_30d,
        layer: Some("L4".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn derive_confidence_saturates_at_one() {
        assert_eq!(derive_confidence(200), 1.0);
        assert_eq!(derive_confidence(500), 1.0);
        assert_eq!(derive_confidence(i32::MAX), 1.0);
    }

    #[test]
    fn derive_confidence_proportional_below_saturation() {
        // 100 / 200 = 0.5; allow tiny float epsilon
        assert!((derive_confidence(100) - 0.5).abs() < 1e-6);
        // 30 / 200 = 0.15
        assert!((derive_confidence(30) - 0.15).abs() < 1e-6);
    }

    #[test]
    fn derive_confidence_zero_clamps() {
        assert_eq!(derive_confidence(0), 0.0);
        assert_eq!(derive_confidence(-5), 0.0);
    }

    #[test]
    fn promote_l4_rounds_p95() {
        let row = CacheRow {
            p95_30d: 123.7,
            sample_size_30d: 50,
        };
        let pred = promote_l4(row);
        assert_eq!(pred.value, 124);
        assert_eq!(pred.sample_size, 50);
        assert_eq!(pred.layer, Some("L4".to_string()));
    }

    #[test]
    fn promote_l4_saturates_huge_values() {
        let row = CacheRow {
            p95_30d: f32::MAX,
            sample_size_30d: 100,
        };
        let pred = promote_l4(row);
        assert_eq!(pred.value, i64::MAX);
    }

    #[test]
    fn promote_l4_floors_to_one_if_zero() {
        // Defensive: a zero p95 should never reach reservation as 0.
        let row = CacheRow {
            p95_30d: 0.0,
            sample_size_30d: 50,
        };
        let pred = promote_l4(row);
        assert_eq!(pred.value, 1);
    }

    #[tokio::test]
    async fn compute_b_returns_none_when_cache_empty() {
        // Skeleton mode (no DB pool) — compute_b always None → L1.
        let cache = OutputDistributionCache::new(None, Duration::from_secs(300));
        let tenant = Uuid::new_v4();
        let r = compute_b(&cache, tenant, "gpt-4o", "agent-a", "chat_short").await;
        assert!(r.is_none(), "skeleton mode must return None (L1 cold-start)");
    }
}
