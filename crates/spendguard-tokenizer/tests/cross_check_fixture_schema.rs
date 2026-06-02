use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use tiktoken_rs::{cl100k_base_singleton, o200k_base_singleton, p50k_base_singleton};

#[derive(Debug, Deserialize)]
struct Manifest {
    schema_version: u32,
    reference_generator: String,
    kinds: BTreeMap<String, KindCases>,
}

#[derive(Debug, Deserialize)]
struct KindCases {
    cases: Vec<FixtureCase>,
}

#[derive(Debug, Deserialize)]
struct FixtureCase {
    case_id: String,
    encoder: String,
    input: String,
    coverage: Vec<String>,
    rationale: String,
    expected_token_count: usize,
    expected_token_ids: Vec<u32>,
}

#[test]
fn cross_check_manifest_covers_all_tokenizer_kinds() {
    let manifest: Manifest = serde_json::from_str(include_str!("fixtures/cross_check.json"))
        .expect("cross-check manifest parses");

    assert_eq!(manifest.schema_version, 1);
    assert!(
        manifest
            .reference_generator
            .contains("regenerate_openai_cross_check.py"),
        "manifest must name the committed reference generator"
    );
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
                !case.case_id.trim().is_empty(),
                "{kind} case missing case_id"
            );
            assert!(
                !case.encoder.trim().is_empty(),
                "{kind} case missing encoder"
            );
            assert!(!case.input.is_empty(), "{kind} case missing input");
            assert!(
                !case.coverage.is_empty(),
                "{kind}/{} case missing coverage tags",
                case.case_id
            );
            assert!(
                !case.rationale.trim().is_empty(),
                "{kind}/{} case missing rationale",
                case.case_id
            );
            assert!(
                !case.expected_token_ids.is_empty(),
                "{kind}/{} case missing token vector",
                case.encoder
            );
            assert_eq!(
                case.expected_token_ids.len(),
                case.expected_token_count,
                "{kind}/{} token count must match token vector length",
                case.case_id
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

#[test]
fn openai_cross_check_manifest_vectors_match_tiktoken_rs() {
    let manifest: Manifest = serde_json::from_str(include_str!("fixtures/cross_check.json"))
        .expect("cross-check manifest parses");
    let openai = manifest
        .kinds
        .get("OPENAI_TIKTOKEN")
        .expect("OpenAI fixture kind exists");

    for case in &openai.cases {
        let actual = match case.encoder.as_str() {
            "cl100k_base" => cl100k_base_singleton().encode_with_special_tokens(&case.input),
            "o200k_base" => o200k_base_singleton().encode_with_special_tokens(&case.input),
            "p50k_base" => p50k_base_singleton().encode_with_special_tokens(&case.input),
            other => panic!("unsupported OpenAI fixture encoder `{other}`"),
        };
        assert_eq!(
            actual, case.expected_token_ids,
            "{} vector drifted",
            case.case_id
        );
    }
}

#[test]
fn openai_unicode_adversarial_cases_cover_issue_109() {
    let manifest: Manifest = serde_json::from_str(include_str!("fixtures/cross_check.json"))
        .expect("cross-check manifest parses");
    let openai = manifest
        .kinds
        .get("OPENAI_TIKTOKEN")
        .expect("OpenAI fixture kind exists");

    let adversarial_case_ids: BTreeSet<&str> = openai
        .cases
        .iter()
        .filter(|case| case.coverage.iter().any(|tag| tag == "unicode_adversarial"))
        .map(|case| case.case_id.as_str())
        .collect();
    assert!(
        adversarial_case_ids.len() >= 6,
        "issue #109 requires at least 6 adversarial UTF-8 fixture cases"
    );

    let mut coverage_tags = BTreeSet::new();
    for case in openai
        .cases
        .iter()
        .filter(|case| case.coverage.iter().any(|tag| tag == "unicode_adversarial"))
    {
        for tag in &case.coverage {
            coverage_tags.insert(tag.as_str());
        }
    }

    for required in [
        "four_byte_utf8",
        "zwj_sequence",
        "rtl_script",
        "cjk_bidi_mix",
        "combining_marks",
        "bom_prefix",
        "mixed_noise",
    ] {
        assert!(
            coverage_tags.contains(required),
            "issue #109 coverage tag `{required}` missing"
        );
    }

    assert!(
        openai
            .cases
            .iter()
            .any(|case| case.input.contains('\u{200d}')),
        "ZWJ fixture input missing"
    );
    assert!(
        openai
            .cases
            .iter()
            .any(|case| case.input.chars().any(|ch| ch as u32 > 0xffff)),
        "4-byte UTF-8 fixture input missing"
    );
    assert!(
        openai.cases.iter().any(|case| {
            case.input
                .chars()
                .any(|ch| matches!(ch as u32, 0x0590..=0x08ff))
        }),
        "RTL fixture input missing"
    );
    assert!(
        openai.cases.iter().any(|case| {
            case.input
                .chars()
                .any(|ch| matches!(ch as u32, 0x4e00..=0x9fff))
        }),
        "CJK fixture input missing"
    );
    assert!(
        openai.cases.iter().any(|case| {
            case.input
                .chars()
                .any(|ch| matches!(ch as u32, 0x0300..=0x036f))
        }),
        "combining-mark fixture input missing"
    );
    assert!(
        openai
            .cases
            .iter()
            .any(|case| case.input.contains('\u{feff}')),
        "BOM fixture input missing"
    );
}
