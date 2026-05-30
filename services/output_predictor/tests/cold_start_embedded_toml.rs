//! Integration test: the embedded `data/model_default_distribution.toml`
//! must load, pass Layer A + Layer B signature checks, and contain the
//! 10 × 7 = 70 baseline entries SLICE_08 ships.
//!
//! Spec ref `cold-start-baseline-spec-v1alpha1.md` §0.2 (LOCKED
//! criteria) + §4 (TOML schema).

use spendguard_output_predictor::classifier;
use spendguard_output_predictor::cold_start_loader::ModelDefaultDistribution;

#[test]
fn embedded_toml_loads_and_has_70_entries() {
    let table = ModelDefaultDistribution::load_embedded()
        .expect("embedded TOML must load (Layer A + Layer B + sanity checks pass)");
    assert_eq!(table.len(), 70);
    assert_eq!(table.schema_version(), "v1alpha1");
}

#[test]
fn embedded_toml_covers_required_10_models() {
    let table = ModelDefaultDistribution::load_embedded().expect("load");
    // Per slice doc: 10 models required for SLICE_08.
    let required = [
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
    for model in &required {
        // Each model must have all 7 classes — covered by the lib test
        // covers_all_required_models_and_classes; here we just verify
        // a single-class hit exists for each model.
        assert!(
            table.lookup(model, "chat_short").is_some(),
            "missing baseline for model `{model}`"
        );
    }
}

#[test]
fn embedded_toml_covers_all_7_prompt_classes_for_canonical_model() {
    let table = ModelDefaultDistribution::load_embedded().expect("load");
    // Per spec §3 — 7 prompt classes.
    for class in classifier::classes::ALL {
        assert!(
            table.lookup("gpt-4o", class).is_some(),
            "gpt-4o missing class `{class}`"
        );
    }
}

#[test]
fn embedded_toml_layer_b_fixture_is_canonical_baseline() {
    // The gpt-4o chat_short entry serves as the Layer B fixture per
    // cold_start_loader. Its values are part of the cold-start
    // contract — moving them requires updating the fixture constants
    // in the loader, which forces reviewer attention.
    let table = ModelDefaultDistribution::load_embedded().expect("load");
    let e = table.lookup("gpt-4o", "chat_short").expect("must exist");
    assert_eq!(e.p50, 150, "Layer B fixture: gpt-4o chat_short P50");
    assert_eq!(e.p95, 320, "Layer B fixture: gpt-4o chat_short P95");
    assert_eq!(e.p99, 520, "Layer B fixture: gpt-4o chat_short P99");
    assert!(e.sample_size >= 500, "spec §7.3 sample_size quality bar");
    assert!(
        (0.0..=1.0).contains(&e.confidence),
        "confidence in unit interval"
    );
}
