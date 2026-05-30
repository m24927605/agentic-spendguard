//! Per-(tenant, model) sample rate + cool-down state.
//!
//! Spec refs:
//!   - `tokenizer-service-spec-v1alpha1.md` §4.1 (sample rate strategy)
//!   - `tokenizer-service-spec-v1alpha1.md` §4.3 (1h cool-down window)
//!
//! ## Behaviour
//!
//! Default behaviour for every (tenant, model) is 1% sampling. When the
//! shadow worker detects drift > per-kind threshold (per spec §4.2) it
//! calls [`SampleRateState::enter_cool_down`] which:
//!   * Sets rate to 100% (every Tier 2 call is shadowed).
//!   * Records `cool_down_until = now() + cool_down_window`.
//!
//! On every `should_sample` call we eagerly check whether the cool-down
//! has elapsed; if so, the entry reverts to the default rate.
//!
//! Cool-down extension: if a new drift alert fires during a cool-down,
//! [`enter_cool_down`] resets `cool_down_until` to `now() + window`
//! (fresh 1h). This matches the spec §4.3 wording — "若 1 hour 內 ≥ 3
//! 次新 alert → 持續維持 100%". We do not separately count alerts; each
//! alert simply refreshes the window.
//!
//! ## Multi-tenant isolation
//!
//! State is keyed on [`ShadowKey { tenant_id, model }`]. There is no
//! global rate state — one tenant's cool-down does not affect any other
//! tenant. The HashMap key includes both fields so cross-tenant rate
//! leakage is structurally impossible.
//!
//! ## Hot path invariant
//!
//! This module is referenced ONLY from `shadow::worker` and from the
//! gRPC `Tokenize` handler when deciding whether to send a shadow event.
//! It is NEVER referenced from `services/sidecar/` or
//! `services/egress_proxy/`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use rand::Rng;

/// Per-(tenant, model) state key. Strings rather than UUIDs because
/// tenant ids in some SpendGuard deployments are non-UUID (per SLICE_01
/// R7 audit convention) and model strings are vendor opaque.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct ShadowKey {
    pub tenant_id: String,
    pub model: String,
}

/// Sample-rate configuration loaded from env / Config.
#[derive(Debug, Clone)]
pub struct SampleRateConfig {
    /// Default rate outside the cool-down window. Per spec §4.1 = 0.01
    /// (1%). Override via control plane API (Phase F).
    pub default_rate: f64,
    /// Cool-down window duration. Per spec §4.3 = 1 hour. Tests
    /// override via this knob.
    pub cool_down: Duration,
    /// Rate during cool-down. Per spec §4.3 = 1.0 (100%).
    pub cool_down_rate: f64,
}

impl Default for SampleRateConfig {
    fn default() -> Self {
        Self {
            default_rate: 0.01,
            cool_down: Duration::from_secs(3600),
            cool_down_rate: 1.0,
        }
    }
}

/// Per-entry mutable state for one (tenant, model) pair.
#[derive(Debug, Clone)]
struct SampleRateEntry {
    /// Override rate (control plane API in Phase F overrides
    /// `default_rate` per-(tenant, model)). `None` = use config default.
    override_rate: Option<f64>,
    /// `Some(deadline)` while we are in a cool-down window started by
    /// a drift alert; `None` when cool-down has elapsed or never fired.
    cool_down_until: Option<Instant>,
    /// Diagnostic: last time we entered a cool-down. Surfaces in
    /// future control plane state-dump endpoints.
    last_alert_at: Option<Instant>,
}

impl Default for SampleRateEntry {
    fn default() -> Self {
        Self {
            override_rate: None,
            cool_down_until: None,
            last_alert_at: None,
        }
    }
}

/// Snapshot of one (tenant, model) entry — for control plane GET
/// endpoint (Phase F) and tests.
#[derive(Debug, Clone, PartialEq)]
pub struct SampleRateSnapshot {
    pub effective_rate: f64,
    pub in_cool_down: bool,
    pub cool_down_remaining: Option<Duration>,
}

/// Shared per-(tenant, model) state. Wrapped in `Arc<RwLock<…>>` so
/// the gRPC handler + shadow worker + control plane handler can all
/// share read-mostly access cheaply. Phase D + E add further mutators.
#[derive(Debug)]
pub struct SampleRateState {
    config: SampleRateConfig,
    entries: RwLock<HashMap<ShadowKey, SampleRateEntry>>,
}

impl SampleRateState {
    pub fn new(config: SampleRateConfig) -> Arc<Self> {
        Arc::new(Self {
            config,
            entries: RwLock::new(HashMap::new()),
        })
    }

    pub fn config(&self) -> &SampleRateConfig {
        &self.config
    }

    /// Decide whether the current request should be sampled. Returns
    /// true with probability `effective_rate(key)`. Uses thread-local
    /// rng so we don't lock-contend across hot-path requests.
    ///
    /// Eagerly expires cool-downs whose deadline has passed (this is
    /// the only way the entry reverts from 100% → 1%).
    pub fn should_sample(&self, key: &ShadowKey) -> bool {
        let rate = self.effective_rate(key);
        if rate >= 1.0 {
            return true;
        }
        if rate <= 0.0 {
            return false;
        }
        let mut rng = rand::thread_rng();
        rng.gen::<f64>() < rate
    }

    /// Compute the effective sample rate for this (tenant, model).
    /// Side effect: if a cool-down has elapsed since the last check,
    /// clear it lazily so the next call returns the default rate.
    pub fn effective_rate(&self, key: &ShadowKey) -> f64 {
        self.effective_rate_at(key, Instant::now())
    }

    /// Test-visible variant that pinpoints `now` so we can drive
    /// cool-down expiry without sleeping.
    pub fn effective_rate_at(&self, key: &ShadowKey, now: Instant) -> f64 {
        // First read under read-lock — fast path for non-cool-down case.
        {
            let entries = self.entries.read();
            if let Some(entry) = entries.get(key) {
                if let Some(until) = entry.cool_down_until {
                    if until > now {
                        return self.config.cool_down_rate;
                    }
                    // Cool-down expired — drop to write-lock to clear.
                } else {
                    return entry.override_rate.unwrap_or(self.config.default_rate);
                }
            } else {
                return self.config.default_rate;
            }
        }
        // Slow path: clear elapsed cool-down + return default.
        let mut entries = self.entries.write();
        if let Some(entry) = entries.get_mut(key) {
            if let Some(until) = entry.cool_down_until {
                if until <= now {
                    entry.cool_down_until = None;
                }
            }
            entry.override_rate.unwrap_or(self.config.default_rate)
        } else {
            self.config.default_rate
        }
    }

    /// Enter (or extend) the cool-down window for this (tenant, model).
    /// Per spec §4.3 the rate jumps to 100% for `config.cool_down`. If
    /// a cool-down is already active, the deadline is refreshed to
    /// `now + cool_down` — this is the "cool-down extension" rule.
    pub fn enter_cool_down(&self, key: &ShadowKey) {
        self.enter_cool_down_at(key, Instant::now());
    }

    /// Test-visible variant that pinpoints `now`.
    pub fn enter_cool_down_at(&self, key: &ShadowKey, now: Instant) {
        let mut entries = self.entries.write();
        let entry = entries.entry(key.clone()).or_default();
        entry.cool_down_until = Some(now + self.config.cool_down);
        entry.last_alert_at = Some(now);
    }

    /// Apply a control plane override (Phase F). `rate` is the new
    /// baseline outside cool-down. Pass `None` to clear the override
    /// back to config default.
    pub fn set_override_rate(&self, key: &ShadowKey, rate: Option<f64>) {
        let mut entries = self.entries.write();
        let entry = entries.entry(key.clone()).or_default();
        entry.override_rate = rate;
    }

    /// Snapshot for control plane GET / tests.
    pub fn snapshot(&self, key: &ShadowKey) -> SampleRateSnapshot {
        self.snapshot_at(key, Instant::now())
    }

    /// Test-visible variant that pinpoints `now`.
    pub fn snapshot_at(&self, key: &ShadowKey, now: Instant) -> SampleRateSnapshot {
        let entries = self.entries.read();
        match entries.get(key) {
            Some(entry) => {
                let in_cool_down = entry
                    .cool_down_until
                    .map(|until| until > now)
                    .unwrap_or(false);
                let effective_rate = if in_cool_down {
                    self.config.cool_down_rate
                } else {
                    entry.override_rate.unwrap_or(self.config.default_rate)
                };
                let cool_down_remaining = if in_cool_down {
                    entry.cool_down_until.map(|until| until.saturating_duration_since(now))
                } else {
                    None
                };
                SampleRateSnapshot {
                    effective_rate,
                    in_cool_down,
                    cool_down_remaining,
                }
            }
            None => SampleRateSnapshot {
                effective_rate: self.config.default_rate,
                in_cool_down: false,
                cool_down_remaining: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(tenant: &str, model: &str) -> ShadowKey {
        ShadowKey {
            tenant_id: tenant.into(),
            model: model.into(),
        }
    }

    fn state_with_short_window() -> Arc<SampleRateState> {
        SampleRateState::new(SampleRateConfig {
            default_rate: 0.01,
            cool_down: Duration::from_millis(100),
            cool_down_rate: 1.0,
        })
    }

    #[test]
    fn defaults_are_per_spec() {
        let cfg = SampleRateConfig::default();
        assert!((cfg.default_rate - 0.01).abs() < f64::EPSILON);
        assert_eq!(cfg.cool_down, Duration::from_secs(3600));
        assert!((cfg.cool_down_rate - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn unknown_key_returns_default_rate() {
        let s = SampleRateState::new(SampleRateConfig::default());
        let snap = s.snapshot(&key("t", "gpt-4o"));
        assert!((snap.effective_rate - 0.01).abs() < f64::EPSILON);
        assert!(!snap.in_cool_down);
        assert!(snap.cool_down_remaining.is_none());
    }

    #[test]
    fn cool_down_enter_lifts_rate_to_one() {
        let s = state_with_short_window();
        let k = key("t1", "claude-3-5-sonnet");
        s.enter_cool_down(&k);
        let snap = s.snapshot(&k);
        assert!((snap.effective_rate - 1.0).abs() < f64::EPSILON);
        assert!(snap.in_cool_down);
        assert!(snap.cool_down_remaining.unwrap() <= Duration::from_millis(100));
    }

    #[test]
    fn cool_down_expires_after_window() {
        let s = state_with_short_window();
        let k = key("t2", "gemini-1.5-pro");
        let t0 = Instant::now();
        s.enter_cool_down_at(&k, t0);
        // Just before deadline → still in cool-down.
        let mid = t0 + Duration::from_millis(50);
        let snap = s.snapshot_at(&k, mid);
        assert!(snap.in_cool_down);
        // After deadline → revert to default.
        let after = t0 + Duration::from_millis(200);
        let snap = s.snapshot_at(&k, after);
        assert!(!snap.in_cool_down);
        // Re-check via effective_rate_at which clears the elapsed entry.
        let rate = s.effective_rate_at(&k, after);
        assert!((rate - 0.01).abs() < f64::EPSILON);
    }

    #[test]
    fn cool_down_extension_resets_deadline() {
        // Spec §4.3: drift alert during cool-down extends the window
        // by a fresh `cool_down` duration.
        let s = state_with_short_window();
        let k = key("t3", "command-r-plus");
        let t0 = Instant::now();
        s.enter_cool_down_at(&k, t0);
        // Re-alert at t0 + 50ms — should reset to t0 + 50ms + 100ms.
        let t1 = t0 + Duration::from_millis(50);
        s.enter_cool_down_at(&k, t1);
        // At t0 + 130ms (post initial deadline, mid extension) we
        // must still be in cool-down because the extension reset.
        let t_check = t0 + Duration::from_millis(130);
        let snap = s.snapshot_at(&k, t_check);
        assert!(snap.in_cool_down, "cool-down extension must keep us active past the initial 100ms deadline");
    }

    #[test]
    fn cross_tenant_isolation() {
        // Tenant A's cool-down must not leak to Tenant B even on the
        // same model.
        let s = state_with_short_window();
        let k_a = key("tenant-a", "gpt-4o");
        let k_b = key("tenant-b", "gpt-4o");
        s.enter_cool_down(&k_a);
        let snap_a = s.snapshot(&k_a);
        let snap_b = s.snapshot(&k_b);
        assert!(snap_a.in_cool_down);
        assert!(!snap_b.in_cool_down);
        assert!((snap_b.effective_rate - 0.01).abs() < f64::EPSILON);
    }

    #[test]
    fn override_rate_persists_across_cool_down_expiry() {
        // If operator sets a 5% override and then a drift alert fires,
        // we should jump to 100% during cool-down then revert to 5%.
        let s = state_with_short_window();
        let k = key("t4", "claude-3-haiku");
        s.set_override_rate(&k, Some(0.05));
        let snap = s.snapshot(&k);
        assert!((snap.effective_rate - 0.05).abs() < f64::EPSILON);

        let t0 = Instant::now();
        s.enter_cool_down_at(&k, t0);
        let snap = s.snapshot_at(&k, t0);
        assert!((snap.effective_rate - 1.0).abs() < f64::EPSILON);

        let after = t0 + Duration::from_millis(200);
        let rate = s.effective_rate_at(&k, after);
        assert!((rate - 0.05).abs() < f64::EPSILON);
    }

    #[test]
    fn should_sample_obeys_endpoints() {
        let s = SampleRateState::new(SampleRateConfig::default());
        let k = key("t", "gpt-4o");
        // Override to 0 → never sample.
        s.set_override_rate(&k, Some(0.0));
        for _ in 0..100 {
            assert!(!s.should_sample(&k));
        }
        // Override to 1.0 → always sample.
        s.set_override_rate(&k, Some(1.0));
        for _ in 0..100 {
            assert!(s.should_sample(&k));
        }
    }

    #[test]
    fn should_sample_default_rate_approximates_one_percent() {
        // Probabilistic test — at 1% rate over 10000 samples we expect
        // 100 hits. Allow a generous range (50, 200) to keep CI green.
        let s = SampleRateState::new(SampleRateConfig::default());
        let k = key("t", "gpt-4o");
        let trials = 10_000;
        let mut hits = 0;
        for _ in 0..trials {
            if s.should_sample(&k) {
                hits += 1;
            }
        }
        assert!(
            hits >= 50 && hits <= 200,
            "expected ~100 hits at 1% rate, got {hits}"
        );
    }

    #[test]
    fn clear_override_reverts_to_default() {
        let s = SampleRateState::new(SampleRateConfig::default());
        let k = key("t", "gpt-4o");
        s.set_override_rate(&k, Some(0.5));
        assert!((s.snapshot(&k).effective_rate - 0.5).abs() < f64::EPSILON);
        s.set_override_rate(&k, None);
        assert!((s.snapshot(&k).effective_rate - 0.01).abs() < f64::EPSILON);
    }
}
