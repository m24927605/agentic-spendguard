//! D18 §3 — experimental codec two-channel opt-in gate.
//!
//! Per design.md §3 decision 1: routing rows tagged `experimental:
//! true` are refused unless BOTH:
//!
//! 1. The env var `SPENDGUARD_EXPERIMENTAL_CODECS=1` is set, AND
//! 2. The `spendguard.toml` config has
//!    `[experimental.windsurf_codec] enabled = true`.
//!
//! Either alone is insufficient.

use serde::Deserialize;

/// Experimental codec configuration parsed from `spendguard.toml`.
///
/// Mirrors the D17 cursor codec opt-in so a single `[experimental]`
/// section holds both gates.
#[derive(Debug, Deserialize, Default, Clone)]
pub struct ExperimentalConfig {
    /// Windsurf / Codeium Cascade codec opt-in (D18).
    #[serde(default)]
    pub windsurf_codec: WindsurfExperimentalConfig,
}

/// Per-codec opt-in flag for the Windsurf Cascade codec.
#[derive(Debug, Deserialize, Default, Clone)]
pub struct WindsurfExperimentalConfig {
    /// `false` by default. Must be explicitly set to `true` in
    /// `spendguard.toml` AND the env var must agree before the codec
    /// will accept Cascade routes.
    #[serde(default)]
    pub enabled: bool,
}

/// Returns true iff BOTH the env var AND the config say yes.
///
/// Per design.md §3 decision 1: the env var is the deploy-time gate
/// and the config is the runtime gate. Both must agree.
pub fn windsurf_codec_enabled(cfg: &ExperimentalConfig) -> bool {
    let env_ok = std::env::var("SPENDGUARD_EXPERIMENTAL_CODECS").as_deref() == Ok("1");
    env_ok && cfg.windsurf_codec.enabled
}

/// Emit the experimental codec boot warning to stderr.
///
/// Per design.md §3 decision 2: exactly one structured `WARN` per
/// boot when the codec is enabled. Test-asserted by
/// `tests/passthrough_byte_equivalence.rs` (indirectly — the warning
/// itself goes to stderr; tests assert the gate fired).
pub fn emit_boot_warning(last_verified_capture: &str) {
    eprintln!(
        "[EXPERIMENTAL] kind=\"experimental_codec_enabled\" \
         codec=\"windsurf_managed_cascade\" \
         vendor_protocol=\"undocumented\" \
         support_tier=\"sow_only\" \
         last_verified_capture=\"{last_verified_capture}\" \
         msg=\"experimental Windsurf codec enabled — vendor wire may change without notice\""
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialise env-var mutation across tests. `std::env::set_var`
    /// and `remove_var` are process-global; cargo test runs tests in
    /// parallel by default, so concurrent mutation races. This mutex
    /// makes the env-touching tests safe.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// (1) Gate closed by default.
    #[test]
    fn default_config_gate_closed() {
        let _g = ENV_LOCK.lock().unwrap();
        let cfg = ExperimentalConfig::default();
        std::env::remove_var("SPENDGUARD_EXPERIMENTAL_CODECS");
        assert!(!windsurf_codec_enabled(&cfg));
    }

    /// (2) Env var alone does not open the gate.
    #[test]
    fn env_var_alone_does_not_open_gate() {
        let _g = ENV_LOCK.lock().unwrap();
        let cfg = ExperimentalConfig::default();
        std::env::set_var("SPENDGUARD_EXPERIMENTAL_CODECS", "1");
        let result = windsurf_codec_enabled(&cfg);
        std::env::remove_var("SPENDGUARD_EXPERIMENTAL_CODECS");
        assert!(!result);
    }

    /// (3) Config alone does not open the gate.
    #[test]
    fn config_alone_does_not_open_gate() {
        let _g = ENV_LOCK.lock().unwrap();
        let cfg = ExperimentalConfig {
            windsurf_codec: WindsurfExperimentalConfig { enabled: true },
        };
        std::env::remove_var("SPENDGUARD_EXPERIMENTAL_CODECS");
        assert!(!windsurf_codec_enabled(&cfg));
    }

    /// (4) Both channels agree → gate open.
    #[test]
    fn both_channels_open_the_gate() {
        let _g = ENV_LOCK.lock().unwrap();
        let cfg = ExperimentalConfig {
            windsurf_codec: WindsurfExperimentalConfig { enabled: true },
        };
        std::env::set_var("SPENDGUARD_EXPERIMENTAL_CODECS", "1");
        let result = windsurf_codec_enabled(&cfg);
        std::env::remove_var("SPENDGUARD_EXPERIMENTAL_CODECS");
        assert!(result);
    }

    /// (5) Env var set to "0" does not open the gate.
    #[test]
    fn env_var_zero_does_not_open_gate() {
        let _g = ENV_LOCK.lock().unwrap();
        let cfg = ExperimentalConfig {
            windsurf_codec: WindsurfExperimentalConfig { enabled: true },
        };
        std::env::set_var("SPENDGUARD_EXPERIMENTAL_CODECS", "0");
        let result = windsurf_codec_enabled(&cfg);
        std::env::remove_var("SPENDGUARD_EXPERIMENTAL_CODECS");
        assert!(!result);
    }

    /// (6) Boot warning emits to stderr without panicking.
    #[test]
    fn boot_warning_does_not_panic() {
        emit_boot_warning("synthetic — 2026-06-07");
    }
}
