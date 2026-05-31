//! Strategy B — SQL P95 lookup with cold-start chain.
//!
//! Spec refs:
//!   * output-predictor-service-spec-v1alpha1.md §4 (cache lookup)
//!   * output-predictor-service-spec-v1alpha1.md §7 (cold-start chain
//!     L4 → L3 → L2 → L1)
//!   * cold-start-baseline-spec-v1alpha1.md §2.5 (lookup algorithm)
//!
//! ## SLICE_08 chain coverage
//!
//! SLICE_06 shipped L4 (cache hit + sample_size_30d >= 30) and L1 (None
//! return). SLICE_07 added Strategy C as a parallel branch (not a chain
//! layer). SLICE_08 wires **L2 fallback** via the embedded
//! `model_default_distribution.toml`:
//!
//!   * L4 (cache hit + sample_size_30d >= 30): supported since SLICE_06
//!   * L3 (federated aggregate): deferred per spec §2.2 — returns None
//!   * L2 (model_default_distribution.toml): SLICE_08 — lookup hits
//!     the embedded TOML when L4 cache miss
//!   * L1 (hard fallback): supported since SLICE_06 — represented by
//!     `compute_b` returning None; the audit row writes
//!     `cold_start_layer_used = 'L1'` per spec §7.1.
//!
//! ## Layer codes in PredictionB.layer
//!
//! Per spec §7.1 truth table:
//!   * Some(L4): cache hit → audit `cold_start_layer_used` = NULL
//!   * Some(L3): federated → audit `cold_start_layer_used` = 'L3'
//!   * Some(L2): TOML → audit `cold_start_layer_used` = 'L2'
//!   * None (L1): caller maps to audit `cold_start_layer_used` = 'L1'
//!
//! ## Confidence derivation (spec §4.4 + cold-start-baseline-spec §4.3)
//!
//! - L4: `confidence = min(1.0, sample_size_30d / 200.0)` — 200 samples
//!   saturates confidence; tunable post-calibration-report
//! - L2: `confidence = toml.entry.confidence * sample_size_weight` where
//!   `sample_size_weight = min(1.0, sample_size / 1000.0)` — the TOML's
//!   reviewer-judged confidence × a damper that prevents tiny benchmark
//!   sets from claiming high confidence

use std::sync::Arc;

use uuid::Uuid;

use crate::cache::{CacheRow, OutputDistributionCache};
use crate::cold_start_loader::{BaselineEntry, ModelDefaultDistribution};

/// Strategy B output. Mirrors the spec §4.2 cache row distilled to the
/// minimum subset the predictor + audit chain need at decision time.
#[derive(Debug, Clone)]
pub struct PredictionB {
    /// P95 token count from the chosen layer. L4 paths populate from
    /// the customer's own distribution; L2 paths populate from the
    /// model_default_distribution.toml entry.
    pub value: i64,
    /// Per spec §4.4. f32 since the proto `optional float confidence`
    /// field is f32.
    pub confidence: f32,
    /// Per spec §4.4 — sample_size_30d for L4; TOML entry sample_size
    /// for L2.
    pub sample_size: i32,
    /// Layer label for audit row mapping per spec §7.1. Always Some
    /// when `PredictionB` is itself Some (L4 / L2 / L3); L1 is
    /// represented by `compute_b` returning None.
    pub layer: Option<String>,
}

/// L4 confidence per spec §4.4 — `min(1.0, n / 200.0)`. 200 samples
/// saturates confidence.
fn derive_l4_confidence(sample_size_30d: i32) -> f32 {
    let raw = (sample_size_30d as f32) / 200.0;
    if raw > 1.0 {
        1.0
    } else if raw < 0.0 {
        0.0
    } else {
        raw
    }
}

/// L2 confidence per cold-start-baseline-spec-v1alpha1.md §4.3 —
/// `entry.confidence * sample_size_weight` where sample_size_weight is
/// a damper that scales 1000 samples → 1.0, fewer samples → < 1.0.
///
/// Rationale: TOML reviewer-judged confidence is already 0.3-0.65 for
/// v1alpha1 (per source quality bar). Damping by sample_size prevents
/// a 500-sample baseline from claiming the same authority as a hypothetical
/// 5000-sample one. The damper saturates at 1000 samples (which is the
/// spec §7.3 quality bar where confidence stabilises).
fn derive_l2_confidence(entry_confidence: f32, sample_size: i32) -> f32 {
    let sample_size_weight = ((sample_size as f32) / 1000.0).clamp(0.0, 1.0);
    let raw = entry_confidence * sample_size_weight;
    raw.clamp(0.0, 1.0)
}

/// Compute Strategy B per spec §7 lookup algorithm.
///
/// Order: L4 → L3 (deferred None) → L2 → L1 (None return).
pub async fn compute_b(
    cache: &OutputDistributionCache,
    cold_start: Option<&Arc<ModelDefaultDistribution>>,
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

    // L3: federated aggregate (deferred per spec §2.2). Always None
    // until post-launch SLICE_extra_L3.
    // if L3_ENABLED { ... }

    // L2: model_default_distribution.toml. SLICE_08 — when the L4 cache
    // misses (or empty in skeleton mode), look up the embedded baseline
    // for (model, prompt_class). None on miss → fall to L1.
    if let Some(table) = cold_start {
        if let Some(entry) = table.lookup(model, prompt_class) {
            return Some(promote_l2(entry));
        }
    }

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
        confidence: derive_l4_confidence(row.sample_size_30d),
        sample_size: row.sample_size_30d,
        layer: Some("L4".to_string()),
    }
}

/// Map a TOML L2 baseline entry to PredictionB. Direct copy of the
/// entry's P95 — no rounding (entries are i64 in the TOML schema).
fn promote_l2(entry: &BaselineEntry) -> PredictionB {
    PredictionB {
        value: entry.p95.max(1),
        confidence: derive_l2_confidence(entry.confidence, entry.sample_size),
        sample_size: entry.sample_size,
        layer: Some("L2".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn derive_l4_confidence_saturates_at_one() {
        assert_eq!(derive_l4_confidence(200), 1.0);
        assert_eq!(derive_l4_confidence(500), 1.0);
        assert_eq!(derive_l4_confidence(i32::MAX), 1.0);
    }

    #[test]
    fn derive_l4_confidence_proportional_below_saturation() {
        // 100 / 200 = 0.5; allow tiny float epsilon
        assert!((derive_l4_confidence(100) - 0.5).abs() < 1e-6);
        // 30 / 200 = 0.15
        assert!((derive_l4_confidence(30) - 0.15).abs() < 1e-6);
    }

    #[test]
    fn derive_l4_confidence_zero_clamps() {
        assert_eq!(derive_l4_confidence(0), 0.0);
        assert_eq!(derive_l4_confidence(-5), 0.0);
    }

    #[test]
    fn derive_l2_confidence_saturates_at_full_sample_size() {
        // sample_size_weight = min(1.0, 1000/1000) = 1.0
        // → returns entry.confidence
        assert!((derive_l2_confidence(0.6, 1000) - 0.6).abs() < 1e-6);
        assert!((derive_l2_confidence(0.6, 2000) - 0.6).abs() < 1e-6);
    }

    #[test]
    fn derive_l2_confidence_dampens_small_sample_sizes() {
        // 500 / 1000 = 0.5 weight; 0.6 * 0.5 = 0.30
        assert!((derive_l2_confidence(0.6, 500) - 0.30).abs() < 1e-6);
        // 100 / 1000 = 0.1 weight; 0.5 * 0.1 = 0.05
        assert!((derive_l2_confidence(0.5, 100) - 0.05).abs() < 1e-6);
    }

    #[test]
    fn derive_l2_confidence_clamps_to_unit_interval() {
        // Inputs that could escape [0,1]
        assert_eq!(derive_l2_confidence(0.0, 1000), 0.0);
        assert_eq!(derive_l2_confidence(1.0, 1000), 1.0);
        assert_eq!(derive_l2_confidence(0.5, 0), 0.0);
        assert_eq!(derive_l2_confidence(0.5, -100), 0.0);
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

    #[test]
    fn promote_l2_uses_entry_p95() {
        let entry = BaselineEntry {
            p50: 150,
            p95: 320,
            p99: 520,
            sample_size: 1500,
            confidence: 0.65,
            source: "MT-Bench-2024-q4".to_string(),
        };
        let pred = promote_l2(&entry);
        assert_eq!(pred.value, 320);
        assert_eq!(pred.sample_size, 1500);
        assert_eq!(pred.layer, Some("L2".to_string()));
        // 1500 / 1000 = saturates at 1.0; 0.65 * 1.0 = 0.65
        assert!((pred.confidence - 0.65).abs() < 1e-6);
    }

    #[test]
    fn promote_l2_zero_p95_floors_to_one() {
        let entry = BaselineEntry {
            p50: 0,
            p95: 0,
            p99: 0,
            sample_size: 500,
            confidence: 0.4,
            source: "test".to_string(),
        };
        let pred = promote_l2(&entry);
        assert_eq!(pred.value, 1);
    }

    #[tokio::test]
    async fn compute_b_returns_none_when_cache_empty_and_no_cold_start() {
        // Skeleton mode (no DB pool + no cold_start table) → compute_b
        // always None → L1.
        let cache = OutputDistributionCache::new(None, Duration::from_secs(300));
        let tenant = Uuid::new_v4();
        let r = compute_b(&cache, None, tenant, "gpt-4o", "agent-a", "chat_short").await;
        assert!(
            r.is_none(),
            "skeleton mode must return None (L1 cold-start)"
        );
    }

    #[tokio::test]
    async fn compute_b_returns_l2_when_cache_empty_and_cold_start_hits() {
        // SLICE_08 — cache empty (skeleton mode), but cold_start table
        // has the (model, class) entry → L2 fallback fires.
        let cache = OutputDistributionCache::new(None, Duration::from_secs(300));
        let table =
            Arc::new(ModelDefaultDistribution::load_embedded().expect("load embedded toml"));
        let tenant = Uuid::new_v4();
        let r = compute_b(
            &cache,
            Some(&table),
            tenant,
            "gpt-4o",
            "agent-a",
            "chat_short",
        )
        .await;
        let pred = r.expect("L2 must hit");
        assert_eq!(pred.layer, Some("L2".to_string()));
        // gpt-4o / chat_short fixture P95 from Layer B check
        assert_eq!(pred.value, 320);
        assert!(pred.sample_size >= 500);
        assert!(pred.confidence > 0.0);
    }

    #[tokio::test]
    async fn compute_b_returns_l1_when_cold_start_table_lacks_entry() {
        // SLICE_08 — cold_start table loaded but (model, class) absent →
        // L2 misses → L1 fallback (None return).
        let cache = OutputDistributionCache::new(None, Duration::from_secs(300));
        let table =
            Arc::new(ModelDefaultDistribution::load_embedded().expect("load embedded toml"));
        let tenant = Uuid::new_v4();
        let r = compute_b(
            &cache,
            Some(&table),
            tenant,
            "nonexistent-model-xyz",
            "agent-a",
            "chat_short",
        )
        .await;
        assert!(
            r.is_none(),
            "unknown model must return None (L1 cold-start)"
        );
    }
}
