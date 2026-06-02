const LICENSE_NOTICES: &str = include_str!("../LICENSE_NOTICES.md");
const README: &str = include_str!("../../../README.md");
const TOKENIZER_SPEC: &str = include_str!("../../../docs/tokenizer-service-spec-v1alpha1.md");

fn llama_notice_section() -> &'static str {
    let start = LICENSE_NOTICES
        .find("### Meta Llama 3.1 SentencePiece")
        .expect("Llama notice section exists");
    let end = LICENSE_NOTICES[start..]
        .find("## Reproducibility & refresh")
        .map(|offset| start + offset)
        .expect("Llama notice section terminator exists");
    &LICENSE_NOTICES[start..end]
}

#[test]
fn llama_license_notice_lists_post_ga_10_operational_clauses() {
    let section = llama_notice_section();
    for needle in [
        "Built with Llama",
        "700 million monthly active users",
        "Acceptable Use Policy",
        "https://github.com/meta-llama/llama-models/blob/main/models/llama3_1/LICENSE",
        "https://llama.meta.com/llama3_1/use-policy",
        "not legal advice",
        "Meta attribution notice",
    ] {
        assert!(section.contains(needle), "missing Llama notice `{needle}`");
    }

    assert!(
        !section.contains("restrictions kick in for model weight redistribution"),
        "Llama notice must not understate license obligations as weight-only"
    );
}

#[test]
fn readme_carries_llama_product_notice_and_operator_pointer() {
    for needle in [
        "Built with Llama",
        "700 million monthly active users",
        "Acceptable Use Policy",
        "crates/spendguard-tokenizer/LICENSE_NOTICES.md",
    ] {
        assert!(README.contains(needle), "README missing `{needle}`");
    }
}

#[test]
fn tokenizer_spec_carries_current_llama_license_disclosure() {
    for needle in [
        "Llama 3.1 Community License",
        "Built with Llama",
        "700 million monthly",
        "Acceptable Use Policy",
        "not legal advice",
    ] {
        assert!(TOKENIZER_SPEC.contains(needle), "spec missing `{needle}`");
    }

    assert!(
        !TOKENIZER_SPEC.contains("Llama 2 Community License"),
        "tokenizer spec must not describe Llama 3.1 assets as Llama 2 licensed"
    );
}
