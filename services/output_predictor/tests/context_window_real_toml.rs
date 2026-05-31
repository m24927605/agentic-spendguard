//! Integration test: the bundled `data/model_context_window.toml` must
//! parse and contain the canonical vendor models listed in Phase C.
//!
//! This guards against accidental corruption of the bundled asset and
//! catches schema drift if a refresh introduces unrelated keys.

use spendguard_output_predictor::context_window::ContextWindowTable;

#[test]
fn bundled_toml_loads_and_contains_canonical_models() {
    // Resolve relative to the crate root (CARGO_MANIFEST_DIR) so the
    // test works regardless of the working directory at `cargo test`
    // invocation.
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/model_context_window.toml");
    let table = ContextWindowTable::load_from_path(&path);

    // Canonical entries Phase C ships — these vendor identifiers are
    // long-lived; if any disappear, refresh must call them out in a
    // dedicated migration commit.
    let canonical: &[(&str, i64)] = &[
        ("gpt-4o", 128_000),
        ("gpt-4o-mini", 128_000),
        ("claude-3-5-sonnet-20240620", 200_000),
        ("claude-3-5-sonnet-20241022", 200_000),
        ("gemini-1.5-pro", 2_000_000),
        ("gemini-1.5-flash", 1_000_000),
    ];
    for (model, expected) in canonical {
        let got = table.lookup(model);
        assert_eq!(
            got,
            Some(*expected),
            "bundled toml lost entry for `{model}` — expected {expected}, got {got:?}"
        );
    }
}
