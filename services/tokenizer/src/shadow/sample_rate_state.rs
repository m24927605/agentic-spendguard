//! Phase A placeholder — full implementation in Phase B.
//!
//! Reserves the minimal type surface that [`super::mod.rs`] re-exports
//! so Phase A compiles cleanly. Phase B fills in:
//!   * `should_sample` / `enter_cool_down` / `exit_cool_down_if_expired`
//!   * Cool-down extension semantics (spec §4.3)
//!   * Per-(tenant, model) state map under `Arc<RwLock<HashMap<…>>>`
//!   * Unit tests for the four state transitions
//!
//! See `tokenizer-service-spec-v1alpha1.md` §4.1 / §4.3.

use std::sync::Arc;

/// Per-(tenant, model) state key. Strings rather than UUIDs because
/// tenant ids in some SpendGuard deployments are non-UUID (per SLICE_01
/// R7 audit convention) and model strings are vendor opaque.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct ShadowKey {
    pub tenant_id: String,
    pub model: String,
}

/// Sample-rate configuration. Phase B threads this through `Config`.
#[derive(Debug, Clone)]
pub struct SampleRateConfig {
    pub default_rate: f64,
    pub cool_down: std::time::Duration,
}

impl Default for SampleRateConfig {
    fn default() -> Self {
        Self {
            // Per spec §4.1 — default 1% sampling outside cool-down.
            default_rate: 0.01,
            // Per spec §4.3 — 1 hour cool-down window after a drift
            // alert; sample rate rises to 100% during the window.
            cool_down: std::time::Duration::from_secs(3600),
        }
    }
}

/// Placeholder type — Phase B replaces the body with the per-(tenant,
/// model) state map.
#[derive(Debug, Clone, Default)]
pub struct SampleRateState {
    /// Phase A keeps a smoke field so the `Arc<SampleRateState>` clone
    /// pattern is exercised end-to-end; Phase B replaces with the
    /// real `Arc<RwLock<HashMap<ShadowKey, SampleRateEntry>>>`.
    config: SampleRateConfig,
}

impl SampleRateState {
    pub fn new(config: SampleRateConfig) -> Arc<Self> {
        Arc::new(Self { config })
    }

    pub fn config(&self) -> &SampleRateConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_per_spec() {
        let cfg = SampleRateConfig::default();
        // Spec §4.1 — 1% default sampling.
        assert!((cfg.default_rate - 0.01).abs() < f64::EPSILON);
        // Spec §4.3 — 1 hour cool-down window.
        assert_eq!(cfg.cool_down, std::time::Duration::from_secs(3600));
    }

    #[test]
    fn arc_wrapping_compiles() {
        let s = SampleRateState::new(SampleRateConfig::default());
        let cloned = Arc::clone(&s);
        assert_eq!(s.config().default_rate, cloned.config().default_rate);
    }
}
