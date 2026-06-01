use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
struct Manifest {
    schema_version: u32,
    kinds: BTreeMap<String, KindCases>,
}

#[derive(Debug, Deserialize)]
struct KindCases {
    cases: Vec<FixtureCase>,
}

#[derive(Debug, Deserialize)]
struct FixtureCase {
    encoder: String,
    input: String,
    expected_token_ids: Vec<u32>,
}

#[test]
fn cross_check_manifest_covers_all_tokenizer_kinds() {
    let manifest: Manifest = serde_json::from_str(include_str!("fixtures/cross_check.json"))
        .expect("cross-check manifest parses");

    assert_eq!(manifest.schema_version, 1);
    let expected_kinds = [
        "ANTHROPIC_BPE",
        "COHERE_BPE",
        "GEMINI_BPE",
        "OPENAI_TIKTOKEN",
        "SENTENCEPIECE_LLAMA",
    ];
    let actual_kinds: Vec<&str> = manifest.kinds.keys().map(String::as_str).collect();
    assert_eq!(actual_kinds, expected_kinds);

    for (kind, cases) in &manifest.kinds {
        assert!(
            !cases.cases.is_empty(),
            "{kind} must define at least one case"
        );
        for case in &cases.cases {
            assert!(
                !case.encoder.trim().is_empty(),
                "{kind} case missing encoder"
            );
            assert!(!case.input.is_empty(), "{kind} case missing input");
            assert!(
                !case.expected_token_ids.is_empty(),
                "{kind}/{} case missing token vector",
                case.encoder
            );
        }
    }
}

#[test]
fn cross_check_manifest_keeps_anthropic_and_gemini_extensible() {
    let manifest: Manifest = serde_json::from_str(include_str!("fixtures/cross_check.json"))
        .expect("cross-check manifest parses");

    let anthropic = manifest
        .kinds
        .get("ANTHROPIC_BPE")
        .expect("Anthropic fixture kind exists");
    assert_eq!(anthropic.cases[0].encoder, "anthropic-claude3");

    let gemini = manifest
        .kinds
        .get("GEMINI_BPE")
        .expect("Gemini fixture kind exists");
    assert_eq!(gemini.cases[0].encoder, "gemini-1.5");
}
