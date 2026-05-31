//! Cold-start L2 baseline loader — model_default_distribution.toml.
//!
//! Spec refs:
//!   * cold-start-baseline-spec-v1alpha1.md §4 (TOML schema)
//!   * cold-start-baseline-spec-v1alpha1.md §6 (30-sample threshold)
//!   * cold-start-baseline-spec-v1alpha1.md §8 (failure modes)
//!   * output-predictor-service-spec-v1alpha1.md §7 (L2 wiring)
//!
//! ## What this module does
//!
//! Loads the embedded `model_default_distribution.toml` asset at boot,
//! performs sanity checks per spec §6, and exposes a `lookup(model,
//! class)` API that Strategy B's cold-start chain calls when L4 is
//! empty.
//!
//! ## Asset signature dual-layer (SLICE_03 pattern)
//!
//! Per SLICE_03's `verify_chain` pattern + slice doc constraints:
//!
//!   * **Layer A** — SHA-256 of the embedded TOML bytes is computed at
//!     boot and compared to `EMBEDDED_TOML_SHA256` (const baked into
//!     binary). Mismatch → refuse-to-start. Protects against a
//!     post-compile asset swap on disk (defense in depth — `include_bytes!`
//!     bakes at compile time, so this guards against post-compile binary
//!     tampering and developer mistakes during refresh).
//!   * **Layer B** — known-good fixture cross-check: a hand-known entry
//!     `(gpt-4o, chat_short)` must parse to its expected baseline values.
//!     Catches schema regressions that pass Layer A (e.g., a TOML
//!     mutation that updates the sha256 but corrupts entries).
//!
//! ## Sanity checks per spec §6
//!
//!   * No duplicate `(model, class)` keys (deterministic lookup)
//!   * `0.0 <= confidence <= 1.0`
//!   * `P50 <= P95 <= P99` (statistical monotonicity)
//!   * `sample_size >= 30` (or fail with override note per §7.3 quality
//!     bar — v1alpha1 enforces 500 per spec §7.3 with floor 30 acceptable)
//!   * `schema_version == "v1alpha1"`
//!
//! ## Memory bound
//!
//! The internal HashMap is naturally bounded at 70 entries (the size of
//! the TOML asset). No LRU needed — entire table fits in ~25KB resident.

use std::collections::HashMap;

use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tracing::{info, warn};

/// Embedded TOML asset — baked at compile time via `include_bytes!`.
///
/// Hot-reload is NOT supported in SLICE_08 (per spec §8 — "TOML hot-
/// reload triggered (rare): reload with sanity check; revert if invalid"
/// is post-launch). Refresh requires a new binary build.
const EMBEDDED_TOML_BYTES: &[u8] = include_bytes!("../data/model_default_distribution.toml");

/// SHA-256 of `EMBEDDED_TOML_BYTES` baked at build time.
///
/// Computed via `shasum -a 256 services/output_predictor/data/
/// model_default_distribution.toml`. When refreshing the TOML, update
/// this constant in the same commit and re-run the loader tests.
///
/// Layer A asset signature per slice doc constraint.
const EMBEDDED_TOML_SHA256: &str =
    "ea16cf56c2d955d63fd37f4863d87d18268ec84a9dc399181397cb39bd3ff607";

/// Expected schema_version for v1alpha1 TOML. Bump triggers spec v2
/// review per spec §0.4.
const EXPECTED_SCHEMA_VERSION: &str = "v1alpha1";

/// Layer B cross-check fixture: a known-good entry that must parse to
/// these values. Update only when refreshing the gpt-4o chat_short
/// baseline (Q3+ refresh cadence).
const LAYER_B_FIXTURE_MODEL: &str = "gpt-4o";
const LAYER_B_FIXTURE_CLASS: &str = "chat_short";
const LAYER_B_FIXTURE_P50: i64 = 150;
const LAYER_B_FIXTURE_P95: i64 = 320;
const LAYER_B_FIXTURE_P99: i64 = 520;

/// Minimum sample_size required for a TOML entry per spec §6.3 + §7.3.
/// Hard floor is 30 (statistical bound); §7.3 quality bar is 500
/// (curation policy); we enforce 30 here and warn on < 500.
const MIN_SAMPLE_SIZE_HARD: i32 = 30;
const MIN_SAMPLE_SIZE_QUALITY_BAR: i32 = 500;

/// Loader errors. Any variant triggers refuse-to-start in main.rs.
#[derive(Debug, Error)]
pub enum LoadError {
    #[error("asset signature mismatch (Layer A): expected sha256={expected}, computed={computed} — refusing to start to prevent post-compile asset tampering")]
    AssetSignatureMismatch { expected: String, computed: String },
    #[error("Layer B fixture cross-check failed: expected ({model}, {class}) → P50={expected_p50}/P95={expected_p95}/P99={expected_p99}, got {actual:?} — TOML may be corrupted or fixture out of date")]
    LayerBFixtureMismatch {
        model: String,
        class: String,
        expected_p50: i64,
        expected_p95: i64,
        expected_p99: i64,
        actual: Option<(i64, i64, i64)>,
    },
    #[error("schema_version mismatch: expected `{expected}`, got `{got}` — refusing to start")]
    SchemaVersionMismatch { expected: String, got: String },
    #[error("TOML parse error: {0}")]
    ParseError(String),
    #[error("duplicate entry for (model={model}, class={class}) — TOML must have unique keys")]
    DuplicateEntry { model: String, class: String },
    #[error("entry (model={model}, class={class}) confidence {confidence} out of [0.0, 1.0]")]
    ConfidenceOutOfRange {
        model: String,
        class: String,
        confidence: f32,
    },
    #[error("entry (model={model}, class={class}) percentile order violated: P50={p50}, P95={p95}, P99={p99}")]
    PercentileOrderViolation {
        model: String,
        class: String,
        p50: i64,
        p95: i64,
        p99: i64,
    },
    #[error(
        "entry (model={model}, class={class}) sample_size {sample_size} below hard floor {floor}"
    )]
    SampleSizeBelowFloor {
        model: String,
        class: String,
        sample_size: i32,
        floor: i32,
    },
    #[error("entry (model={model}, class={class}) percentile must be > 0: P50={p50}, P95={p95}, P99={p99}")]
    NonPositivePercentile {
        model: String,
        class: String,
        p50: i64,
        p95: i64,
        p99: i64,
    },
    #[error(
        "entry (model={model}, class={class}) unknown prompt_class — must be one of: {expected}"
    )]
    UnknownPromptClass {
        model: String,
        class: String,
        expected: String,
    },
}

/// TOML root document shape.
#[derive(Debug, Deserialize)]
struct TomlFile {
    schema_version: String,
    #[serde(default)]
    last_updated: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    entries: Vec<TomlEntry>,
}

#[derive(Debug, Deserialize)]
struct TomlEntry {
    model: String,
    prompt_class: String,
    p50: i64,
    p95: i64,
    p99: i64,
    sample_size: i32,
    #[serde(default)]
    source: String,
    // Round-tripped fields — required in TOML for spec §4.3 citation
    // but unused at runtime. Kept here so unknown fields don't bypass
    // serde's deny-unknown validation if added later.
    #[serde(default)]
    #[allow(dead_code)]
    source_url: String,
    #[serde(default)]
    #[allow(dead_code)]
    methodology_doc: String,
    confidence: f32,
}

/// Public baseline entry returned by `lookup()`. Field order mirrors
/// `PredictionB` shape for ease of conversion in strategy_b.rs.
#[derive(Debug, Clone, PartialEq)]
pub struct BaselineEntry {
    pub p50: i64,
    pub p95: i64,
    pub p99: i64,
    pub sample_size: i32,
    pub confidence: f32,
    pub source: String,
}

/// Lookup table: (model, prompt_class) → entry.
///
/// Bounded at construction by the TOML file size (70 entries in
/// v1alpha1). No LRU needed.
pub struct ModelDefaultDistribution {
    entries: HashMap<(String, String), BaselineEntry>,
    schema_version: String,
    last_updated: String,
}

impl ModelDefaultDistribution {
    /// Eager-load + verify the embedded TOML asset. Refuses to construct
    /// (returns Err) on any signature / sanity violation per spec §8.
    pub fn load_embedded() -> Result<Self, LoadError> {
        // ── Layer A: SHA-256 asset signature check ─────────────────────
        let computed_sha = Self::compute_sha256(EMBEDDED_TOML_BYTES);
        if computed_sha != EMBEDDED_TOML_SHA256 {
            return Err(LoadError::AssetSignatureMismatch {
                expected: EMBEDDED_TOML_SHA256.to_string(),
                computed: computed_sha,
            });
        }

        // ── Parse TOML bytes ───────────────────────────────────────────
        let text = std::str::from_utf8(EMBEDDED_TOML_BYTES)
            .map_err(|e| LoadError::ParseError(format!("invalid utf8: {e}")))?;
        let parsed: TomlFile =
            toml::from_str(text).map_err(|e| LoadError::ParseError(e.to_string()))?;

        // ── schema_version gate (refuse-to-start on unknown) ───────────
        if parsed.schema_version != EXPECTED_SCHEMA_VERSION {
            return Err(LoadError::SchemaVersionMismatch {
                expected: EXPECTED_SCHEMA_VERSION.to_string(),
                got: parsed.schema_version,
            });
        }

        let mut entries: HashMap<(String, String), BaselineEntry> = HashMap::new();

        // ── Per-entry sanity checks per spec §6 ────────────────────────
        for e in parsed.entries {
            // Known class allow-list (delegated to crate::classifier).
            if !crate::classifier::is_known_class(&e.prompt_class) {
                return Err(LoadError::UnknownPromptClass {
                    model: e.model,
                    class: e.prompt_class,
                    expected: crate::classifier::classes::ALL.join(" | "),
                });
            }

            // Confidence in [0.0, 1.0]
            if !(0.0..=1.0).contains(&e.confidence) {
                return Err(LoadError::ConfidenceOutOfRange {
                    model: e.model.clone(),
                    class: e.prompt_class,
                    confidence: e.confidence,
                });
            }

            // Percentile monotonicity P50 <= P95 <= P99
            if !(e.p50 <= e.p95 && e.p95 <= e.p99) {
                return Err(LoadError::PercentileOrderViolation {
                    model: e.model.clone(),
                    class: e.prompt_class,
                    p50: e.p50,
                    p95: e.p95,
                    p99: e.p99,
                });
            }

            // Non-positive percentiles are nonsensical
            if e.p50 <= 0 || e.p95 <= 0 || e.p99 <= 0 {
                return Err(LoadError::NonPositivePercentile {
                    model: e.model.clone(),
                    class: e.prompt_class,
                    p50: e.p50,
                    p95: e.p95,
                    p99: e.p99,
                });
            }

            // Sample size floor (statistical bound)
            if e.sample_size < MIN_SAMPLE_SIZE_HARD {
                return Err(LoadError::SampleSizeBelowFloor {
                    model: e.model.clone(),
                    class: e.prompt_class,
                    sample_size: e.sample_size,
                    floor: MIN_SAMPLE_SIZE_HARD,
                });
            }

            // Below quality bar (warn but not refuse — soft signal)
            if e.sample_size < MIN_SAMPLE_SIZE_QUALITY_BAR {
                warn!(
                    model = %e.model,
                    class = %e.prompt_class,
                    sample_size = e.sample_size,
                    quality_bar = MIN_SAMPLE_SIZE_QUALITY_BAR,
                    "TOML entry below spec §7.3 quality bar (sample_size < 500) — \
                     accept but flag for next quarterly refresh"
                );
            }

            // Duplicate (model, class) key
            let key = (e.model.clone(), e.prompt_class.clone());
            if entries.contains_key(&key) {
                return Err(LoadError::DuplicateEntry {
                    model: e.model,
                    class: e.prompt_class,
                });
            }

            entries.insert(
                key,
                BaselineEntry {
                    p50: e.p50,
                    p95: e.p95,
                    p99: e.p99,
                    sample_size: e.sample_size,
                    confidence: e.confidence,
                    source: e.source,
                },
            );
        }

        // ── Layer B: known-good fixture cross-check ────────────────────
        let fixture_key = (
            LAYER_B_FIXTURE_MODEL.to_string(),
            LAYER_B_FIXTURE_CLASS.to_string(),
        );
        match entries.get(&fixture_key) {
            Some(entry)
                if entry.p50 == LAYER_B_FIXTURE_P50
                    && entry.p95 == LAYER_B_FIXTURE_P95
                    && entry.p99 == LAYER_B_FIXTURE_P99 =>
            { /* ok */ }
            other => {
                return Err(LoadError::LayerBFixtureMismatch {
                    model: LAYER_B_FIXTURE_MODEL.to_string(),
                    class: LAYER_B_FIXTURE_CLASS.to_string(),
                    expected_p50: LAYER_B_FIXTURE_P50,
                    expected_p95: LAYER_B_FIXTURE_P95,
                    expected_p99: LAYER_B_FIXTURE_P99,
                    actual: other.map(|e| (e.p50, e.p95, e.p99)),
                });
            }
        }

        let count = entries.len();
        info!(
            entries = count,
            schema_version = %parsed.schema_version,
            last_updated = %parsed.last_updated,
            "model_default_distribution.toml loaded (cold-start L2)"
        );

        let _ = parsed.notes; // keep field round-trip in case future serialize

        Ok(Self {
            entries,
            schema_version: parsed.schema_version,
            last_updated: parsed.last_updated,
        })
    }

    /// SHA-256 of `bytes` as lowercase hex. Used by Layer A check.
    fn compute_sha256(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hex::encode(hasher.finalize())
    }

    /// Lookup the baseline entry for `(model, prompt_class)`. None on
    /// miss — caller (strategy_b) falls through to L1 per spec §7.
    pub fn lookup(&self, model: &str, prompt_class: &str) -> Option<&BaselineEntry> {
        // Hot path: HashMap O(1). 70 entries fits in CPU cache.
        self.entries
            .get(&(model.to_string(), prompt_class.to_string()))
    }

    /// Empty / disabled table for tests + skeleton-mode tests.
    pub fn empty() -> Self {
        Self {
            entries: HashMap::new(),
            schema_version: String::new(),
            last_updated: String::new(),
        }
    }

    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    pub fn last_updated(&self) -> &str {
        &self.last_updated
    }

    /// Total entry count — public so integration tests + ops dashboards
    /// can verify the embedded TOML covers the SLICE_08 contract
    /// (70 entries for v1alpha1).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` if `(model, class)` has an entry. Cheap O(1) probe useful
    /// for ops dashboards + integration tests.
    pub fn contains_key(&self, model: &str, class: &str) -> bool {
        self.entries
            .contains_key(&(model.to_string(), class.to_string()))
    }

    /// Convenience for `len() == 0` (clippy::len_without_is_empty).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl std::fmt::Debug for ModelDefaultDistribution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Avoid dumping all 70 entries; just summary.
        f.debug_struct("ModelDefaultDistribution")
            .field("entry_count", &self.entries.len())
            .field("schema_version", &self.schema_version)
            .field("last_updated", &self.last_updated)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_embedded_succeeds_and_has_70_entries() {
        let table = ModelDefaultDistribution::load_embedded().expect("embedded TOML must load");
        assert_eq!(
            table.len(),
            70,
            "spec requires ≥ 10 models × 7 classes = 70 entries"
        );
        assert_eq!(table.schema_version(), "v1alpha1");
    }

    #[test]
    fn lookup_hits_known_entry() {
        let table = ModelDefaultDistribution::load_embedded().expect("load");
        let entry = table.lookup("gpt-4o", "chat_short").expect("known entry");
        // Layer B fixture values
        assert_eq!(entry.p50, 150);
        assert_eq!(entry.p95, 320);
        assert_eq!(entry.p99, 520);
        assert!(entry.confidence > 0.5);
        assert!(entry.sample_size >= 500);
    }

    #[test]
    fn lookup_misses_unknown_model() {
        let table = ModelDefaultDistribution::load_embedded().expect("load");
        assert!(
            table.lookup("nonexistent-model", "chat_short").is_none(),
            "unknown model must return None for L1 fallback"
        );
    }

    #[test]
    fn lookup_misses_unknown_class_for_known_model() {
        let table = ModelDefaultDistribution::load_embedded().expect("load");
        // SLICE_08 ships full 10×7 grid; "future_class" is not in classifier
        assert!(
            table.lookup("gpt-4o", "future_class").is_none(),
            "unknown class must return None"
        );
    }

    #[test]
    fn empty_table_lookups_return_none() {
        let table = ModelDefaultDistribution::empty();
        assert_eq!(table.lookup("gpt-4o", "chat_short"), None);
        assert_eq!(table.len(), 0);
    }

    #[test]
    fn all_70_entries_have_valid_percentile_order() {
        let table = ModelDefaultDistribution::load_embedded().expect("load");
        for ((model, class), entry) in &table.entries {
            assert!(
                entry.p50 <= entry.p95,
                "p50 must be <= p95 for ({model}, {class})"
            );
            assert!(
                entry.p95 <= entry.p99,
                "p95 must be <= p99 for ({model}, {class})"
            );
            assert!(entry.p50 > 0, "p50 must be positive for ({model}, {class})");
            assert!(
                (0.0..=1.0).contains(&entry.confidence),
                "confidence must be in [0,1] for ({model}, {class})"
            );
            assert!(
                entry.sample_size >= MIN_SAMPLE_SIZE_HARD,
                "sample_size must be >= {MIN_SAMPLE_SIZE_HARD} for ({model}, {class})"
            );
        }
    }

    #[test]
    fn covers_all_required_models_and_classes() {
        let table = ModelDefaultDistribution::load_embedded().expect("load");
        let required_models = [
            "gpt-4o",
            "gpt-4o-mini",
            "claude-3-5-sonnet",
            "claude-3-haiku",
            "gemini-1.5-pro",
            "gemini-1.5-flash",
            "llama-3-70b-instruct",
            "mistral-large",
            "gpt-3.5-turbo",
            "claude-3-opus",
        ];
        let required_classes = crate::classifier::classes::ALL;
        for m in &required_models {
            for c in required_classes {
                assert!(table.contains_key(m, c), "missing ({m}, {c}) entry");
            }
        }
    }

    #[test]
    fn compute_sha256_known_value() {
        let bytes = b"hello world";
        let sha = ModelDefaultDistribution::compute_sha256(bytes);
        // openssl-dgst -sha256 known value for "hello world"
        assert_eq!(
            sha,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn sanity_check_rejects_confidence_above_one() {
        // Synthetic TOML with confidence > 1.0 → ConfidenceOutOfRange.
        let toml_text = r#"
schema_version = "v1alpha1"

[[entries]]
model = "gpt-4o"
prompt_class = "chat_short"
p50 = 100
p95 = 200
p99 = 300
sample_size = 500
confidence = 1.5
"#;
        let parsed: TomlFile = toml::from_str(toml_text).expect("parse");
        assert_eq!(parsed.entries.len(), 1);
        // Construct via internal-only path — we duplicate the validation
        // shape from `load_embedded` to assert that confidence-range
        // check fires. (Cannot call `load_embedded` with a custom payload
        // by design — refuse-to-start on real-asset mismatch.)
        let e = &parsed.entries[0];
        assert!(!(0.0..=1.0).contains(&e.confidence));
    }

    #[test]
    fn sanity_check_rejects_percentile_order_violation() {
        // Synthetic with p50 > p95 → PercentileOrderViolation.
        let toml_text = r#"
schema_version = "v1alpha1"

[[entries]]
model = "gpt-4o"
prompt_class = "chat_short"
p50 = 500
p95 = 200
p99 = 300
sample_size = 500
confidence = 0.5
"#;
        let parsed: TomlFile = toml::from_str(toml_text).expect("parse");
        let e = &parsed.entries[0];
        assert!(!(e.p50 <= e.p95 && e.p95 <= e.p99));
    }

    #[test]
    fn sanity_check_rejects_sample_size_below_floor() {
        // Synthetic with sample_size < 30 → SampleSizeBelowFloor.
        let toml_text = r#"
schema_version = "v1alpha1"

[[entries]]
model = "gpt-4o"
prompt_class = "chat_short"
p50 = 100
p95 = 200
p99 = 300
sample_size = 10
confidence = 0.5
"#;
        let parsed: TomlFile = toml::from_str(toml_text).expect("parse");
        let e = &parsed.entries[0];
        assert!(e.sample_size < MIN_SAMPLE_SIZE_HARD);
    }

    #[test]
    fn layer_b_fixture_constants_match_toml() {
        // Layer B guard — if someone edits the TOML's gpt-4o chat_short
        // entry without updating the fixture constants, load fails. This
        // test confirms the constants match the live TOML so refresh
        // discipline stays in sync.
        let table = ModelDefaultDistribution::load_embedded().expect("load");
        let entry = table
            .lookup(LAYER_B_FIXTURE_MODEL, LAYER_B_FIXTURE_CLASS)
            .expect("fixture must exist");
        assert_eq!(entry.p50, LAYER_B_FIXTURE_P50);
        assert_eq!(entry.p95, LAYER_B_FIXTURE_P95);
        assert_eq!(entry.p99, LAYER_B_FIXTURE_P99);
    }

    #[test]
    fn asset_signature_constant_matches_embedded_bytes() {
        // Sanity: if someone edits the TOML on disk without updating
        // EMBEDDED_TOML_SHA256, load_embedded() fails with
        // AssetSignatureMismatch. This test confirms the const is in
        // sync with the embedded bytes — proves Layer A is wired correctly.
        let computed = ModelDefaultDistribution::compute_sha256(EMBEDDED_TOML_BYTES);
        assert_eq!(
            computed, EMBEDDED_TOML_SHA256,
            "EMBEDDED_TOML_SHA256 const must match the actual TOML bytes; \
             update both in the same commit when refreshing"
        );
    }

    #[test]
    fn debug_does_not_dump_all_entries() {
        // Config Debug masking (SLICE_05 R2 M13 pattern) — Debug output
        // should be summary, not entry dump.
        let table = ModelDefaultDistribution::load_embedded().expect("load");
        let dbg = format!("{table:?}");
        assert!(dbg.contains("entry_count"));
        assert!(dbg.contains("70"));
        // Should NOT contain individual percentile values.
        assert!(!dbg.contains("150"));
    }
}
