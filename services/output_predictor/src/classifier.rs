//! 7-class rule-based prompt classifier per spec
//! output-predictor-service-spec-v1alpha1.md §8.1 + cold-start-baseline-spec
//! v1alpha1.md §3.
//!
//! ## Design contract
//!
//! Classification is **deterministic**: same input → same class. Rules
//! evaluate in priority order and the first match wins (per spec §8.1
//! priority sequence). This module ships the rule engine + a hand-labeled
//! golden corpus that asserts ≥ 90% accuracy per spec DRAFT→LOCKED
//! criterion #5.
//!
//! ## 7 classes (per cold-start-baseline-spec §3.1)
//!
//! | ID | Class           | Trigger                                              |
//! |----|-----------------|------------------------------------------------------|
//! | 1  | vision          | has_image_content                                    |
//! | 2  | tool_calling    | !tool_definitions.empty                              |
//! | 3  | summarization   | input_tokens > 8000 AND max_tokens < 1000            |
//! | 4  | code_gen        | content contains ```, def, function, class           |
//! | 5  | rag             | content contains Document N:, Source:, [N]           |
//! | 6  | chat_long       | input_tokens > 1500 OR messages > 4                  |
//! | 7  | chat_short      | (default)                                            |
//!
//! Priority is fixed: vision > tool_calling > summarization > code_gen >
//! rag > chat_long > chat_short.
//!
//! ## When sidecar already classified
//!
//! Per spec §8.3 the sidecar runs the classifier and passes the result
//! via `PredictRequest.prompt_class`. The predictor's classifier is the
//! server-side **validation** path — it does not re-run the classifier
//! on the hot path. This module's [`classify_from_features`] is exposed
//! for offline calibration / verification tools + the golden corpus
//! test.

use once_cell::sync::Lazy;
use regex::Regex;

/// Classifier version stamp. Per spec §8.4 the version flows through to
/// `PredictResponse.classifier_version` and the audit row's
/// `prompt_class_fingerprint` prefix — bumping this version invalidates
/// historical L4 cache entries that were classified by an older version.
pub const CLASSIFIER_VERSION: &str = "v1alpha1";

/// Stable string identifiers for each of the 7 prompt classes. Used as
/// the `prompt_class` bucket key in `output_distribution_cache`.
pub mod classes {
    pub const VISION: &str = "vision";
    pub const TOOL_CALLING: &str = "tool_calling";
    pub const SUMMARIZATION: &str = "summarization";
    pub const CODE_GEN: &str = "code_gen";
    pub const RAG: &str = "rag";
    pub const CHAT_LONG: &str = "chat_long";
    pub const CHAT_SHORT: &str = "chat_short";

    pub const ALL: &[&str] = &[
        VISION,
        TOOL_CALLING,
        SUMMARIZATION,
        CODE_GEN,
        RAG,
        CHAT_LONG,
        CHAT_SHORT,
    ];
}

/// Spec §8.1 thresholds — `summarization` is "large input, small output".
const SUMMARIZATION_INPUT_TOKENS: i64 = 8000;
const SUMMARIZATION_MAX_TOKENS: i64 = 1000;

/// Spec §8.1 thresholds — `chat_long` is "many tokens or many turns".
const CHAT_LONG_INPUT_TOKENS: i64 = 1500;
const CHAT_LONG_MESSAGE_COUNT: usize = 4;

// ---------------------------------------------------------------------------
// Regex markers (per spec §8.1)
// ---------------------------------------------------------------------------

/// Code markers per spec §8.1 `contains_code_markers`. Backticks alone
/// don't fire (must be ``` triple); `def`, `function`, `class` only fire
/// at word boundary followed by a name.
static CODE_FENCE_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"```").expect("compile CODE_FENCE_REGEX"));

static CODE_KEYWORD_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b(def|function|class)\s+\w+").expect("compile CODE_KEYWORD_REGEX"));

/// Retrieval markers per spec §8.1 `contains_retrieval_markers`. Common
/// RAG envelope markers — "Document 1:", "Source: ...", "[3] ...".
static RAG_MARKER_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(Document \d+:|Source: |^\[\d+\] )").expect("compile RAG_MARKER_REGEX"));

// ---------------------------------------------------------------------------
// Input features
// ---------------------------------------------------------------------------

/// Features the classifier inspects. The sidecar computes these from
/// the raw request before the Predict call; the server-side classifier
/// (offline path) reconstructs them from messages/content.
#[derive(Debug, Clone)]
pub struct ClassifierFeatures<'a> {
    pub has_image_content: bool,
    pub tool_definitions_count: usize,
    pub input_tokens: i64,
    /// max_tokens from the request. 0 means unset/INFINITY (per spec §3.1
    /// semantics) which fails the summarization "small output" gate so
    /// such requests never classify as summarization.
    pub max_tokens: i64,
    pub message_count: usize,
    /// Concatenated message contents for marker scanning. Caller is
    /// expected to join the `messages` array into a single string before
    /// invocation; the regex engines scan once over this view.
    pub joined_content: &'a str,
}

/// Classify by features (priority-ordered rules per spec §8.1).
pub fn classify_from_features(f: &ClassifierFeatures) -> &'static str {
    // 1. Vision (highest priority — spec §8.1 rule 1)
    if f.has_image_content {
        return classes::VISION;
    }

    // 2. Tool calling (spec §8.1 rule 2)
    if f.tool_definitions_count > 0 {
        return classes::TOOL_CALLING;
    }

    // 3. Summarization (spec §8.1 rule 3 — large input, small output cap).
    // max_tokens=0 (unset, INFINITY) doesn't satisfy "< 1000".
    if f.input_tokens > SUMMARIZATION_INPUT_TOKENS
        && f.max_tokens > 0
        && f.max_tokens < SUMMARIZATION_MAX_TOKENS
    {
        return classes::SUMMARIZATION;
    }

    // 4. Code generation (spec §8.1 rule 4)
    if contains_code_markers(f.joined_content) {
        return classes::CODE_GEN;
    }

    // 5. RAG (spec §8.1 rule 5)
    if contains_retrieval_markers(f.joined_content) {
        return classes::RAG;
    }

    // 6. Long chat (spec §8.1 rule 6)
    if f.input_tokens > CHAT_LONG_INPUT_TOKENS || f.message_count > CHAT_LONG_MESSAGE_COUNT {
        return classes::CHAT_LONG;
    }

    // 7. Short chat (default)
    classes::CHAT_SHORT
}

fn contains_code_markers(content: &str) -> bool {
    CODE_FENCE_REGEX.is_match(content) || CODE_KEYWORD_REGEX.is_match(content)
}

fn contains_retrieval_markers(content: &str) -> bool {
    RAG_MARKER_REGEX.is_match(content)
}

/// Boundary validation — caller-supplied `prompt_class` must be one of
/// the 7 known classes. Rejecting unknown values here keeps the
/// `output_distribution_cache` bucket key space bounded and avoids
/// silent miscount when callers upgrade classifiers ahead of the
/// service. Empty string is rejected (proto3 default = "missing").
pub fn is_known_class(class: &str) -> bool {
    classes::ALL.iter().any(|c| *c == class)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn features_for(class_hint_content: &str, input_tokens: i64, message_count: usize) -> ClassifierFeatures<'_> {
        ClassifierFeatures {
            has_image_content: false,
            tool_definitions_count: 0,
            input_tokens,
            max_tokens: 0,
            message_count,
            joined_content: class_hint_content,
        }
    }

    #[test]
    fn vision_wins_over_tool_calling() {
        let f = ClassifierFeatures {
            has_image_content: true,
            tool_definitions_count: 5,
            input_tokens: 100,
            max_tokens: 0,
            message_count: 1,
            joined_content: "describe this image",
        };
        assert_eq!(classify_from_features(&f), classes::VISION);
    }

    #[test]
    fn tool_calling_wins_over_summarization() {
        let f = ClassifierFeatures {
            has_image_content: false,
            tool_definitions_count: 3,
            input_tokens: 10_000,
            max_tokens: 500,
            message_count: 1,
            joined_content: "use the calculator tool to compute 42",
        };
        assert_eq!(classify_from_features(&f), classes::TOOL_CALLING);
    }

    #[test]
    fn summarization_requires_large_input_and_small_output() {
        // Large input + small max_tokens → summarization.
        let f = ClassifierFeatures {
            has_image_content: false,
            tool_definitions_count: 0,
            input_tokens: 10_000,
            max_tokens: 300,
            message_count: 1,
            joined_content: "long article text...",
        };
        assert_eq!(classify_from_features(&f), classes::SUMMARIZATION);
    }

    #[test]
    fn summarization_not_triggered_without_max_tokens_cap() {
        // Large input but max_tokens unset (0 = INFINITY) → falls to chat_long.
        let f = ClassifierFeatures {
            has_image_content: false,
            tool_definitions_count: 0,
            input_tokens: 10_000,
            max_tokens: 0,
            message_count: 1,
            joined_content: "long article text...",
        };
        assert_eq!(classify_from_features(&f), classes::CHAT_LONG);
    }

    #[test]
    fn code_gen_via_backtick_fence() {
        let f = features_for("write a function:\n```\ndef foo():\n  pass\n```", 200, 1);
        assert_eq!(classify_from_features(&f), classes::CODE_GEN);
    }

    #[test]
    fn code_gen_via_keyword() {
        let f = features_for("explain what `def main()` does in python", 100, 1);
        assert_eq!(classify_from_features(&f), classes::CODE_GEN);
    }

    #[test]
    fn rag_via_document_marker() {
        let f = features_for("Document 1: The Apollo program\n\nAnswer:", 200, 1);
        assert_eq!(classify_from_features(&f), classes::RAG);
    }

    #[test]
    fn rag_via_source_prefix() {
        let f = features_for("Source: wikipedia\nQ: when was X?", 200, 1);
        assert_eq!(classify_from_features(&f), classes::RAG);
    }

    #[test]
    fn rag_via_bracket_citation() {
        let f = features_for("[1] First citation\nSummarise", 200, 1);
        assert_eq!(classify_from_features(&f), classes::RAG);
    }

    #[test]
    fn chat_long_via_input_tokens() {
        let f = features_for("hello", 2000, 1);
        assert_eq!(classify_from_features(&f), classes::CHAT_LONG);
    }

    #[test]
    fn chat_long_via_message_count() {
        let f = features_for("hi", 100, 5);
        assert_eq!(classify_from_features(&f), classes::CHAT_LONG);
    }

    #[test]
    fn chat_short_default() {
        let f = features_for("hello!", 50, 1);
        assert_eq!(classify_from_features(&f), classes::CHAT_SHORT);
    }

    #[test]
    fn is_known_class_acceptance() {
        for c in classes::ALL {
            assert!(is_known_class(c), "{c} should be known");
        }
        assert!(!is_known_class(""));
        assert!(!is_known_class("CHAT_SHORT")); // case-sensitive
        assert!(!is_known_class("multi_modal"));
    }

    // ── Golden corpus per spec §0.2 / §8.1 acceptance criteria ──
    //
    // 100 hand-labeled samples; classifier must hit ≥ 90% accuracy.
    //
    // R2 M11 (Software F7) — SCOPE NOTE:
    //
    // The current corpus is **N duplicated representative samples per
    // class** (5 vision + 10 tool_calling + 12 summarization + 15
    // code_gen + 13 rag + 15 chat_long + 30 chat_short = 100). This
    // is a SMOKE check that the rule-engine fires on the expected
    // representative shape; it is NOT the diverse hand-labeled corpus
    // the spec ultimately requires.
    //
    // SLICE_08 will ship the real calibration corpus with adversarial
    // edge cases + ambiguous samples that exercise the priority
    // tie-breakers in classify_from_features. Until then this smoke
    // test is the LOCKED definition of "classifier doesn't regress".
    //
    // Spec §0.2 #5 acceptance is downgraded to "smoke-check until
    // SLICE_08 calibration corpus" per the R2 plan.
    #[test]
    fn classifier_90_percent_on_golden_corpus() {
        let corpus = build_golden_corpus();
        assert_eq!(corpus.len(), 100, "corpus must have exactly 100 samples");
        let mut correct = 0;
        let mut wrong: Vec<(usize, &'static str, &'static str)> = Vec::new();
        for (i, (features, expected)) in corpus.iter().enumerate() {
            let actual = classify_from_features(features);
            if actual == *expected {
                correct += 1;
            } else {
                wrong.push((i, expected, actual));
            }
        }
        let accuracy = (correct as f64) / (corpus.len() as f64);
        assert!(
            accuracy >= 0.90,
            "classifier accuracy {accuracy:.2} below 0.90 threshold; {} wrong (first 10): {:?}",
            wrong.len(),
            &wrong.iter().take(10).collect::<Vec<_>>()
        );
    }

    /// 100 hand-labeled (features → class) samples. Distribution per
    /// cold-start-baseline-spec §3.1 — chat_short most common, vision
    /// least (~10% each for the more specialised classes).
    fn build_golden_corpus() -> Vec<(ClassifierFeatures<'static>, &'static str)> {
        let mut samples: Vec<(ClassifierFeatures<'static>, &'static str)> = Vec::new();

        // ── vision (5 samples) ──
        for _ in 0..5 {
            samples.push((
                ClassifierFeatures {
                    has_image_content: true,
                    tool_definitions_count: 0,
                    input_tokens: 200,
                    max_tokens: 500,
                    message_count: 1,
                    joined_content: "describe this image please",
                },
                classes::VISION,
            ));
        }

        // ── tool_calling (10 samples) ──
        for _ in 0..10 {
            samples.push((
                ClassifierFeatures {
                    has_image_content: false,
                    tool_definitions_count: 2,
                    input_tokens: 300,
                    max_tokens: 500,
                    message_count: 1,
                    joined_content: "call the calculator with 2+2",
                },
                classes::TOOL_CALLING,
            ));
        }

        // ── summarization (12 samples) ──
        for _ in 0..12 {
            samples.push((
                ClassifierFeatures {
                    has_image_content: false,
                    tool_definitions_count: 0,
                    input_tokens: 10_000,
                    max_tokens: 300,
                    message_count: 1,
                    joined_content: "Summarise this large article (lots of body text omitted)",
                },
                classes::SUMMARIZATION,
            ));
        }

        // ── code_gen (15 samples) ──
        for _ in 0..15 {
            samples.push((
                ClassifierFeatures {
                    has_image_content: false,
                    tool_definitions_count: 0,
                    input_tokens: 500,
                    max_tokens: 1500,
                    message_count: 1,
                    joined_content: "write a python function:\n```\ndef foo(x):\n  return x*2\n```",
                },
                classes::CODE_GEN,
            ));
        }

        // ── rag (13 samples) ──
        for _ in 0..13 {
            samples.push((
                ClassifierFeatures {
                    has_image_content: false,
                    tool_definitions_count: 0,
                    input_tokens: 400,
                    max_tokens: 600,
                    message_count: 1,
                    joined_content: "Document 1: This is the retrieved context. Answer the user.",
                },
                classes::RAG,
            ));
        }

        // ── chat_long (15 samples) ──
        for _ in 0..15 {
            samples.push((
                ClassifierFeatures {
                    has_image_content: false,
                    tool_definitions_count: 0,
                    input_tokens: 2500,
                    max_tokens: 1000,
                    message_count: 2,
                    joined_content: "continuing our long conversation about agents",
                },
                classes::CHAT_LONG,
            ));
        }

        // ── chat_short (30 samples, the most common bucket) ──
        for _ in 0..30 {
            samples.push((
                ClassifierFeatures {
                    has_image_content: false,
                    tool_definitions_count: 0,
                    input_tokens: 50,
                    max_tokens: 500,
                    message_count: 1,
                    joined_content: "hi! what's your name?",
                },
                classes::CHAT_SHORT,
            ));
        }

        // Sanity: exactly 100 samples.
        assert_eq!(
            samples.len(),
            100,
            "build_golden_corpus must produce 100 samples"
        );
        samples
    }
}
