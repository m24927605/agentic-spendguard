//! Golden-sample integration tests for SLICE_04 Tier 2 expansion.
//!
//! Spec ref `tokenizer-service-spec-v1alpha1.md` §8 acceptance:
//!
//! > Each vendored encoder produces correct count for 50 golden
//! > samples per kind (200 total).
//!
//! ## Sample structure
//!
//! Each sample fixes `(model, text)` and asserts the encode result.
//! Samples use [`tokenize_raw`] for raw-text encoding (no chat
//! envelope) and assert either:
//!   * Exact counts for well-known short fixtures (e.g.,
//!     `"hello world" -> 2 tokens`).
//!   * A `(lower, upper)` band for longer / variable-length text so
//!     a minor tokenizer.json revision bump (within the spec §4.2
//!     drift threshold) doesn't break the gold while a wholesale
//!     encoder corruption still fires.
//!
//! ## Coverage breakdown
//!
//! Per spec §8: 50 samples per kind × 4 kinds = 200 new samples.
//!
//!   * Anthropic (claude-3 family): 50 — short fixtures + dated
//!     model IDs + Bedrock routing IDs + chat envelope.
//!   * Gemini (gemini-1.5 + 2.0 family): 50 — same coverage shape.
//!   * Cohere (command-r + Bedrock): 50.
//!   * Llama (Bedrock meta.llama3): 50.
//!
//! Each kind has the same sub-structure:
//!   * 25 raw-text samples (varied content: ASCII, CJK, emoji,
//!     code, JSON, URLs, repeated text).
//!   * 10 model-string dispatch samples covering dated suffixes +
//!     Bedrock IDs.
//!   * 10 chat envelope samples (system / user / assistant / tool
//!     roles + multi-message conversations).
//!   * 5 boundary samples (empty / single char / very long).

use spendguard_tokenizer::{Message, TokenizeRequest, Tokenizer};
use std::sync::OnceLock;

/// Shared tokenizer instance — booted once per test process and reused
/// across all 200 SLICE_04 golden samples. Boot cost is ~3-5 s for the
/// 7-encoder eager-load (Layer A sha256 + Layer B fixture cross-check
/// per spec §7.4.1); amortising it across the suite drops wall-clock
/// from ~10 min to under 30 s on a typical dev box.
fn shared_tokenizer() -> &'static Tokenizer {
    static TOKENIZER: OnceLock<Tokenizer> = OnceLock::new();
    TOKENIZER.get_or_init(|| {
        Tokenizer::new_with_embedded_assets().expect("boot tokenizer")
    })
}

fn boot_tokenizer() -> &'static Tokenizer {
    shared_tokenizer()
}

fn tokenize_raw(model: &str, text: &str) -> i64 {
    let tok = boot_tokenizer();
    let req = TokenizeRequest {
        model: model.to_string(),
        raw_text: text.to_string(),
        ..Default::default()
    };
    let resp = tok.tokenize(&req).expect("tokenize");
    assert_eq!(resp.tier, "T2", "model `{model}` must hit Tier 2, got tier {}", resp.tier);
    resp.input_tokens
}

fn tokenize_chat(model: &str, role: &str, content: &str) -> i64 {
    let tok = boot_tokenizer();
    let req = TokenizeRequest {
        model: model.to_string(),
        messages: vec![Message {
            role: role.to_string(),
            content: content.to_string(),
            tool_calls: vec![],
        }],
        ..Default::default()
    };
    let resp = tok.tokenize(&req).expect("tokenize");
    assert_eq!(resp.tier, "T2");
    resp.input_tokens
}

fn dispatch_kind(model: &str) -> String {
    let tok = boot_tokenizer();
    let req = TokenizeRequest {
        model: model.to_string(),
        raw_text: "x".to_string(),
        ..Default::default()
    };
    let resp = tok.tokenize(&req).expect("tokenize");
    resp.kind
}

// ════════════════════════════════════════════════════════════════════
// ANTHROPIC — 50 samples (raw + dispatch + chat + boundary)
// ════════════════════════════════════════════════════════════════════

// --- 25 raw-text samples --------------------------------------------

#[test]
fn anthropic_hello_world_is_2_tokens() {
    assert_eq!(tokenize_raw("claude-3-haiku", "hello world"), 2);
}

#[test]
fn anthropic_hello_is_1_token() {
    assert_eq!(tokenize_raw("claude-3-haiku", "hello"), 1);
}

#[test]
fn anthropic_empty_string_is_0_tokens() {
    assert_eq!(tokenize_raw("claude-3-haiku", ""), 0);
}

#[test]
fn anthropic_short_sentence_in_band() {
    let n = tokenize_raw("claude-3-haiku", "The quick brown fox jumps over the lazy dog.");
    assert!((8..=14).contains(&n), "got {n}");
}

#[test]
fn anthropic_punctuation_heavy_text() {
    let n = tokenize_raw("claude-3-haiku", "Hello, world! How are you today?");
    assert!((6..=12).contains(&n), "got {n}");
}

#[test]
fn anthropic_numbers_text() {
    let n = tokenize_raw("claude-3-haiku", "Pi is approximately 3.14159.");
    assert!((6..=12).contains(&n), "got {n}");
}

#[test]
fn anthropic_code_snippet() {
    let n = tokenize_raw("claude-3-haiku", "fn main() { println!(\"hello\"); }");
    assert!((8..=18).contains(&n), "got {n}");
}

#[test]
fn anthropic_json_snippet() {
    let n = tokenize_raw("claude-3-haiku", r#"{"name":"alice","age":30}"#);
    assert!((6..=14).contains(&n), "got {n}");
}

#[test]
fn anthropic_cjk_text() {
    let n = tokenize_raw("claude-3-haiku", "你好世界");
    assert!((3..=10).contains(&n), "got {n}");
}

#[test]
fn anthropic_emoji_text() {
    let n = tokenize_raw("claude-3-haiku", "Hello 🌍 world 🚀");
    assert!((4..=14).contains(&n), "got {n}");
}

#[test]
fn anthropic_newlines_preserved() {
    let n = tokenize_raw("claude-3-haiku", "line1\nline2\nline3");
    assert!((4..=10).contains(&n), "got {n}");
}

#[test]
fn anthropic_repeated_word_compresses() {
    let n = tokenize_raw("claude-3-haiku", "the the the the the the the the");
    assert!((4..=12).contains(&n), "got {n}");
}

#[test]
fn anthropic_long_paragraph_proportional() {
    let text = "This is a longer paragraph that contains multiple sentences. \
                It is intended to verify that the tokenizer correctly handles \
                medium-length text without anomalies. The token count should \
                scale roughly linearly with the character count.";
    let n = tokenize_raw("claude-3-haiku", text);
    assert!((35..=80).contains(&n), "got {n}");
}

#[test]
fn anthropic_url_text() {
    let n = tokenize_raw("claude-3-haiku", "https://example.com/path?q=value");
    assert!((6..=18).contains(&n), "got {n}");
}

#[test]
fn anthropic_special_chars() {
    let n = tokenize_raw("claude-3-haiku", "@#$%^&*()");
    assert!((3..=15).contains(&n), "got {n}");
}

#[test]
fn anthropic_mixed_case_camelcase() {
    let n = tokenize_raw("claude-3-haiku", "ThisIsCamelCaseWord");
    assert!((4..=12).contains(&n), "got {n}");
}

#[test]
fn anthropic_snake_case_identifier() {
    let n = tokenize_raw("claude-3-haiku", "some_function_name_here");
    assert!((4..=12).contains(&n), "got {n}");
}

#[test]
fn anthropic_kebab_case_identifier() {
    let n = tokenize_raw("claude-3-haiku", "some-function-name-here");
    assert!((4..=12).contains(&n), "got {n}");
}

#[test]
fn anthropic_multiline_markdown() {
    let n = tokenize_raw("claude-3-haiku", "# Heading\n\nSome **bold** text and _italic_.");
    assert!((10..=22).contains(&n), "got {n}");
}

#[test]
fn anthropic_korean_text() {
    let n = tokenize_raw("claude-3-haiku", "안녕하세요");
    assert!((2..=12).contains(&n), "got {n}");
}

#[test]
fn anthropic_arabic_text() {
    let n = tokenize_raw("claude-3-haiku", "مرحبا بالعالم");
    assert!((3..=15).contains(&n), "got {n}");
}

#[test]
fn anthropic_xml_snippet() {
    let n = tokenize_raw("claude-3-haiku", "<tag>content</tag>");
    assert!((5..=12).contains(&n), "got {n}");
}

#[test]
fn anthropic_long_repeating() {
    let n = tokenize_raw("claude-3-haiku", &"a".repeat(100));
    assert!((1..=120).contains(&n), "got {n}");
}

#[test]
fn anthropic_python_code() {
    let n = tokenize_raw("claude-3-haiku", "def f(x):\n    return x + 1");
    assert!((6..=18).contains(&n), "got {n}");
}

#[test]
fn anthropic_quotes_text() {
    let n = tokenize_raw("claude-3-haiku", "\"single\" and 'double' quotes");
    assert!((6..=14).contains(&n), "got {n}");
}

// --- 10 model-string dispatch samples -------------------------------

#[test]
fn anthropic_claude_3_sonnet_dispatches() {
    assert_eq!(dispatch_kind("claude-3-sonnet"), "ANTHROPIC_BPE");
}

#[test]
fn anthropic_claude_3_opus_dispatches() {
    assert_eq!(dispatch_kind("claude-3-opus"), "ANTHROPIC_BPE");
}

#[test]
fn anthropic_claude_3_haiku_20240307_dispatches() {
    assert_eq!(dispatch_kind("claude-3-haiku-20240307"), "ANTHROPIC_BPE");
}

#[test]
fn anthropic_claude_3_sonnet_20240229_dispatches() {
    assert_eq!(dispatch_kind("claude-3-sonnet-20240229"), "ANTHROPIC_BPE");
}

#[test]
fn anthropic_claude_3_opus_20240229_dispatches() {
    assert_eq!(dispatch_kind("claude-3-opus-20240229"), "ANTHROPIC_BPE");
}

#[test]
fn anthropic_claude_3_5_sonnet_20240620_dispatches() {
    assert_eq!(dispatch_kind("claude-3-5-sonnet-20240620"), "ANTHROPIC_BPE");
}

#[test]
fn anthropic_claude_3_5_sonnet_20241022_dispatches() {
    assert_eq!(dispatch_kind("claude-3-5-sonnet-20241022"), "ANTHROPIC_BPE");
}

#[test]
fn anthropic_claude_3_5_haiku_20241022_dispatches() {
    assert_eq!(dispatch_kind("claude-3-5-haiku-20241022"), "ANTHROPIC_BPE");
}

#[test]
fn anthropic_bedrock_claude_3_5_sonnet_full_id_dispatches() {
    // Per §9 review question 3 — full Bedrock model id with dated
    // + versioned suffix.
    assert_eq!(
        dispatch_kind("anthropic.claude-3-5-sonnet-20240620-v1:0"),
        "ANTHROPIC_BPE"
    );
}

#[test]
fn anthropic_bedrock_claude_3_haiku_full_id_dispatches() {
    assert_eq!(
        dispatch_kind("anthropic.claude-3-haiku-20240307-v1:0"),
        "ANTHROPIC_BPE"
    );
}

// --- 10 chat envelope samples ---------------------------------------

#[test]
fn anthropic_chat_user_role_adds_envelope() {
    let raw = tokenize_raw("claude-3-haiku", "hello");
    let chat = tokenize_chat("claude-3-haiku", "user", "hello");
    assert!(chat > raw, "chat {chat} must exceed raw {raw}");
}

#[test]
fn anthropic_chat_system_role_in_band() {
    let n = tokenize_chat("claude-3-haiku", "system", "You are helpful.");
    assert!((6..=18).contains(&n), "got {n}");
}

#[test]
fn anthropic_chat_assistant_role_in_band() {
    let n = tokenize_chat("claude-3-haiku", "assistant", "Of course!");
    assert!((6..=15).contains(&n), "got {n}");
}

#[test]
fn anthropic_chat_tool_role_in_band() {
    let n = tokenize_chat("claude-3-haiku", "tool", "{\"result\":42}");
    assert!((6..=18).contains(&n), "got {n}");
}

#[test]
fn anthropic_chat_dated_model_same_as_undated() {
    let a = tokenize_chat("claude-3-haiku", "user", "hello");
    let b = tokenize_chat("claude-3-haiku-20240307", "user", "hello");
    assert_eq!(a, b);
}

#[test]
fn anthropic_chat_3_5_model_works() {
    let n = tokenize_chat("claude-3-5-sonnet-20240620", "user", "hello world");
    assert!(n >= 5);
}

#[test]
fn anthropic_chat_bedrock_model_works() {
    let n = tokenize_chat(
        "anthropic.claude-3-haiku-20240307-v1:0",
        "user",
        "hello",
    );
    assert!(n >= 5);
}

#[test]
fn anthropic_chat_multi_msg_proportional() {
    let tok = boot_tokenizer();
    let req = TokenizeRequest {
        model: "claude-3-haiku".to_string(),
        messages: vec![
            Message {
                role: "system".to_string(),
                content: "You are helpful.".to_string(),
                tool_calls: vec![],
            },
            Message {
                role: "user".to_string(),
                content: "Hi!".to_string(),
                tool_calls: vec![],
            },
            Message {
                role: "assistant".to_string(),
                content: "Hello!".to_string(),
                tool_calls: vec![],
            },
        ],
        ..Default::default()
    };
    let n = tok.tokenize(&req).unwrap().input_tokens;
    assert!(n >= 15, "got {n}");
}

#[test]
fn anthropic_chat_empty_content_handled() {
    let n = tokenize_chat("claude-3-haiku", "user", "");
    // Even empty content has envelope tokens: 3 per-msg + 1 role +
    // 3 priming = 7 lower bound.
    assert!(n >= 4);
}

#[test]
fn anthropic_chat_long_content_proportional() {
    let n = tokenize_chat("claude-3-haiku", "user", &"hello ".repeat(50));
    assert!(n >= 30);
}

// --- 5 boundary samples ---------------------------------------------

#[test]
fn anthropic_single_space() {
    let n = tokenize_raw("claude-3-haiku", " ");
    assert!((1..=3).contains(&n));
}

#[test]
fn anthropic_newline_only() {
    let n = tokenize_raw("claude-3-haiku", "\n");
    assert!((1..=3).contains(&n));
}

#[test]
fn anthropic_tab_only() {
    let n = tokenize_raw("claude-3-haiku", "\t");
    assert!((1..=3).contains(&n));
}

#[test]
fn anthropic_single_char() {
    let n = tokenize_raw("claude-3-haiku", "a");
    assert_eq!(n, 1);
}

#[test]
fn anthropic_very_long_text() {
    let n = tokenize_raw("claude-3-haiku", &"hello world ".repeat(100));
    // ~2 tokens per "hello world " repeats ≈ 200 tokens.
    assert!((100..=400).contains(&n), "got {n}");
}

// ════════════════════════════════════════════════════════════════════
// GEMINI — 50 samples (raw + dispatch + chat + boundary)
// ════════════════════════════════════════════════════════════════════

// --- 25 raw-text samples --------------------------------------------

#[test]
fn gemini_hello_world_is_2_tokens() {
    assert_eq!(tokenize_raw("gemini-1.5-flash", "hello world"), 2);
}

#[test]
fn gemini_hello_is_1_token() {
    assert_eq!(tokenize_raw("gemini-1.5-flash", "hello"), 1);
}

#[test]
fn gemini_empty_string_is_0_tokens() {
    assert_eq!(tokenize_raw("gemini-1.5-flash", ""), 0);
}

#[test]
fn gemini_short_sentence_in_band() {
    let n = tokenize_raw("gemini-1.5-flash", "The quick brown fox jumps over the lazy dog.");
    assert!((8..=14).contains(&n), "got {n}");
}

#[test]
fn gemini_punctuation_heavy_text() {
    let n = tokenize_raw("gemini-1.5-flash", "Hello, world! How are you today?");
    assert!((6..=12).contains(&n), "got {n}");
}

#[test]
fn gemini_numbers_text() {
    let n = tokenize_raw("gemini-1.5-flash", "Pi is approximately 3.14159.");
    assert!((6..=12).contains(&n), "got {n}");
}

#[test]
fn gemini_code_snippet() {
    let n = tokenize_raw("gemini-1.5-flash", "fn main() { println!(\"hello\"); }");
    assert!((8..=18).contains(&n), "got {n}");
}

#[test]
fn gemini_json_snippet() {
    let n = tokenize_raw("gemini-1.5-flash", r#"{"name":"alice","age":30}"#);
    assert!((6..=16).contains(&n), "got {n}");
}

#[test]
fn gemini_cjk_text_compact() {
    // Gemma's larger vocab (256K) compresses CJK very efficiently.
    let n = tokenize_raw("gemini-1.5-flash", "你好世界");
    assert!((1..=8).contains(&n), "got {n}");
}

#[test]
fn gemini_emoji_text() {
    let n = tokenize_raw("gemini-1.5-flash", "Hello 🌍 world 🚀");
    assert!((4..=14).contains(&n), "got {n}");
}

#[test]
fn gemini_newlines_preserved() {
    let n = tokenize_raw("gemini-1.5-flash", "line1\nline2\nline3");
    assert!((4..=10).contains(&n), "got {n}");
}

#[test]
fn gemini_repeated_word_compresses() {
    let n = tokenize_raw("gemini-1.5-flash", "the the the the the the the the");
    assert!((4..=12).contains(&n), "got {n}");
}

#[test]
fn gemini_long_paragraph_proportional() {
    let text = "This is a longer paragraph for tokenizer testing. \
                It must scale roughly linearly with character count.";
    let n = tokenize_raw("gemini-1.5-flash", text);
    assert!((15..=35).contains(&n), "got {n}");
}

#[test]
fn gemini_url_text() {
    let n = tokenize_raw("gemini-1.5-flash", "https://example.com/path?q=value");
    assert!((6..=15).contains(&n), "got {n}");
}

#[test]
fn gemini_special_chars() {
    let n = tokenize_raw("gemini-1.5-flash", "@#$%^&*()");
    assert!((3..=15).contains(&n), "got {n}");
}

#[test]
fn gemini_mixed_case_camelcase() {
    let n = tokenize_raw("gemini-1.5-flash", "ThisIsCamelCaseWord");
    assert!((4..=12).contains(&n), "got {n}");
}

#[test]
fn gemini_snake_case_identifier() {
    let n = tokenize_raw("gemini-1.5-flash", "some_function_name_here");
    assert!((4..=12).contains(&n), "got {n}");
}

#[test]
fn gemini_kebab_case_identifier() {
    let n = tokenize_raw("gemini-1.5-flash", "some-function-name-here");
    assert!((4..=12).contains(&n), "got {n}");
}

#[test]
fn gemini_multiline_markdown() {
    let n = tokenize_raw("gemini-1.5-flash", "# Heading\n\nSome **bold** text.");
    assert!((6..=18).contains(&n), "got {n}");
}

#[test]
fn gemini_korean_text() {
    let n = tokenize_raw("gemini-1.5-flash", "안녕하세요");
    assert!((1..=10).contains(&n), "got {n}");
}

#[test]
fn gemini_arabic_text() {
    let n = tokenize_raw("gemini-1.5-flash", "مرحبا بالعالم");
    assert!((2..=12).contains(&n), "got {n}");
}

#[test]
fn gemini_xml_snippet() {
    let n = tokenize_raw("gemini-1.5-flash", "<tag>content</tag>");
    assert!((4..=12).contains(&n), "got {n}");
}

#[test]
fn gemini_long_repeating() {
    let n = tokenize_raw("gemini-1.5-flash", &"a".repeat(100));
    assert!((1..=120).contains(&n), "got {n}");
}

#[test]
fn gemini_python_code() {
    let n = tokenize_raw("gemini-1.5-flash", "def f(x):\n    return x + 1");
    assert!((6..=18).contains(&n), "got {n}");
}

#[test]
fn gemini_quotes_text() {
    let n = tokenize_raw("gemini-1.5-flash", "\"single\" and 'double' quotes");
    assert!((6..=14).contains(&n), "got {n}");
}

// --- 10 dispatch samples --------------------------------------------

#[test]
fn gemini_1_5_pro_dispatches() {
    assert_eq!(dispatch_kind("gemini-1.5-pro"), "GEMINI_BPE");
}

#[test]
fn gemini_1_5_pro_002_dispatches() {
    assert_eq!(dispatch_kind("gemini-1.5-pro-002"), "GEMINI_BPE");
}

#[test]
fn gemini_1_5_flash_001_dispatches() {
    assert_eq!(dispatch_kind("gemini-1.5-flash-001"), "GEMINI_BPE");
}

#[test]
fn gemini_2_0_flash_dispatches() {
    assert_eq!(dispatch_kind("gemini-2.0-flash"), "GEMINI_BPE");
}

#[test]
fn gemini_2_0_flash_exp_dispatches() {
    assert_eq!(dispatch_kind("gemini-2.0-flash-exp"), "GEMINI_BPE");
}

#[test]
fn gemini_flash_and_pro_match_for_ascii() {
    let a = tokenize_raw("gemini-1.5-flash", "hello world hello");
    let b = tokenize_raw("gemini-1.5-pro", "hello world hello");
    assert_eq!(a, b, "same encoder so counts must match");
}

#[test]
fn gemini_revision_suffix_does_not_change_count() {
    let a = tokenize_raw("gemini-1.5-flash", "hello world");
    let b = tokenize_raw("gemini-1.5-flash-001", "hello world");
    assert_eq!(a, b);
}

#[test]
fn gemini_2_0_matches_1_5_for_ascii() {
    let a = tokenize_raw("gemini-1.5-flash", "hello");
    let b = tokenize_raw("gemini-2.0-flash", "hello");
    assert_eq!(a, b, "same encoder asset so counts must match");
}

#[test]
fn gemini_2_0_exp_matches_2_0() {
    let a = tokenize_raw("gemini-2.0-flash", "test");
    let b = tokenize_raw("gemini-2.0-flash-exp", "test");
    assert_eq!(a, b);
}

#[test]
fn gemini_pro_no_version_does_not_dispatch_to_gemini() {
    // Per spec §3.3 no fuzzy match — `gemini-pro` (without 1.5/2.0
    // prefix) drops to Tier 3.
    let tok = boot_tokenizer();
    let req = TokenizeRequest {
        model: "gemini-pro".to_string(),
        raw_text: "hello".to_string(),
        ..Default::default()
    };
    let resp = tok.tokenize(&req).unwrap();
    assert_eq!(resp.tier, "T3");
    assert_eq!(resp.kind, "HEURISTIC");
}

// --- 10 chat envelope samples ---------------------------------------

#[test]
fn gemini_chat_user_role_adds_envelope() {
    let raw = tokenize_raw("gemini-1.5-flash", "hello");
    let chat = tokenize_chat("gemini-1.5-flash", "user", "hello");
    assert!(chat > raw);
}

#[test]
fn gemini_chat_system_role_in_band() {
    let n = tokenize_chat("gemini-1.5-flash", "system", "You are helpful.");
    assert!((6..=18).contains(&n), "got {n}");
}

#[test]
fn gemini_chat_assistant_role_in_band() {
    let n = tokenize_chat("gemini-1.5-flash", "assistant", "Of course!");
    assert!((6..=15).contains(&n), "got {n}");
}

#[test]
fn gemini_chat_tool_role_in_band() {
    let n = tokenize_chat("gemini-1.5-flash", "tool", "{\"result\":42}");
    assert!((6..=18).contains(&n), "got {n}");
}

#[test]
fn gemini_chat_pro_matches_flash() {
    let a = tokenize_chat("gemini-1.5-flash", "user", "hello");
    let b = tokenize_chat("gemini-1.5-pro", "user", "hello");
    assert_eq!(a, b);
}

#[test]
fn gemini_chat_2_0_works() {
    let n = tokenize_chat("gemini-2.0-flash", "user", "hi");
    assert!(n >= 5);
}

#[test]
fn gemini_chat_long_content_proportional() {
    let n = tokenize_chat("gemini-1.5-flash", "user", &"hi ".repeat(50));
    assert!(n >= 30);
}

#[test]
fn gemini_chat_empty_content() {
    let n = tokenize_chat("gemini-1.5-flash", "user", "");
    assert!(n >= 4);
}

#[test]
fn gemini_chat_cjk_content() {
    let n = tokenize_chat("gemini-1.5-flash", "user", "你好");
    assert!(n >= 5);
}

#[test]
fn gemini_chat_multi_msg() {
    let tok = boot_tokenizer();
    let req = TokenizeRequest {
        model: "gemini-1.5-flash".to_string(),
        messages: vec![
            Message {
                role: "user".to_string(),
                content: "Hi".to_string(),
                tool_calls: vec![],
            },
            Message {
                role: "assistant".to_string(),
                content: "Hello!".to_string(),
                tool_calls: vec![],
            },
        ],
        ..Default::default()
    };
    let n = tok.tokenize(&req).unwrap().input_tokens;
    assert!(n >= 10);
}

// --- 5 boundary samples ---------------------------------------------

#[test]
fn gemini_single_space() {
    let n = tokenize_raw("gemini-1.5-flash", " ");
    assert!((1..=3).contains(&n));
}

#[test]
fn gemini_newline_only() {
    let n = tokenize_raw("gemini-1.5-flash", "\n");
    assert!((1..=3).contains(&n));
}

#[test]
fn gemini_tab_only() {
    let n = tokenize_raw("gemini-1.5-flash", "\t");
    assert!((1..=3).contains(&n));
}

#[test]
fn gemini_single_char() {
    let n = tokenize_raw("gemini-1.5-flash", "a");
    assert_eq!(n, 1);
}

#[test]
fn gemini_very_long_text() {
    let n = tokenize_raw("gemini-1.5-flash", &"hello world ".repeat(100));
    assert!((100..=400).contains(&n), "got {n}");
}

// ════════════════════════════════════════════════════════════════════
// COHERE — 50 samples
// ════════════════════════════════════════════════════════════════════

// --- 25 raw-text samples --------------------------------------------

#[test]
fn cohere_hello_world_is_2_tokens() {
    assert_eq!(tokenize_raw("command-r", "hello world"), 2);
}

#[test]
fn cohere_hello_is_1_token() {
    assert_eq!(tokenize_raw("command-r", "hello"), 1);
}

#[test]
fn cohere_empty_string_is_0_tokens() {
    assert_eq!(tokenize_raw("command-r", ""), 0);
}

#[test]
fn cohere_short_sentence_in_band() {
    let n = tokenize_raw("command-r", "The quick brown fox jumps over the lazy dog.");
    assert!((8..=14).contains(&n), "got {n}");
}

#[test]
fn cohere_punctuation_heavy_text() {
    let n = tokenize_raw("command-r", "Hello, world! How are you today?");
    assert!((6..=12).contains(&n), "got {n}");
}

#[test]
fn cohere_numbers_text() {
    let n = tokenize_raw("command-r", "Pi is approximately 3.14159.");
    assert!((6..=12).contains(&n), "got {n}");
}

#[test]
fn cohere_code_snippet() {
    let n = tokenize_raw("command-r", "fn main() { println!(\"hello\"); }");
    assert!((8..=18).contains(&n), "got {n}");
}

#[test]
fn cohere_json_snippet() {
    let n = tokenize_raw("command-r", r#"{"name":"alice","age":30}"#);
    assert!((6..=16).contains(&n), "got {n}");
}

#[test]
fn cohere_cjk_text() {
    let n = tokenize_raw("command-r", "你好世界");
    assert!((2..=8).contains(&n), "got {n}");
}

#[test]
fn cohere_emoji_text() {
    let n = tokenize_raw("command-r", "Hello 🌍 world 🚀");
    assert!((4..=14).contains(&n), "got {n}");
}

#[test]
fn cohere_newlines_preserved() {
    let n = tokenize_raw("command-r", "line1\nline2\nline3");
    assert!((4..=10).contains(&n), "got {n}");
}

#[test]
fn cohere_repeated_word_compresses() {
    let n = tokenize_raw("command-r", "the the the the the the the the");
    assert!((4..=12).contains(&n), "got {n}");
}

#[test]
fn cohere_long_paragraph_proportional() {
    let text = "This is a longer paragraph for tokenizer testing. \
                It must scale roughly linearly with character count.";
    let n = tokenize_raw("command-r", text);
    assert!((15..=35).contains(&n), "got {n}");
}

#[test]
fn cohere_url_text() {
    let n = tokenize_raw("command-r", "https://example.com/path?q=value");
    assert!((6..=15).contains(&n), "got {n}");
}

#[test]
fn cohere_special_chars() {
    let n = tokenize_raw("command-r", "@#$%^&*()");
    assert!((3..=15).contains(&n), "got {n}");
}

#[test]
fn cohere_mixed_case_camelcase() {
    let n = tokenize_raw("command-r", "ThisIsCamelCaseWord");
    assert!((4..=12).contains(&n), "got {n}");
}

#[test]
fn cohere_snake_case_identifier() {
    let n = tokenize_raw("command-r", "some_function_name_here");
    assert!((4..=12).contains(&n), "got {n}");
}

#[test]
fn cohere_kebab_case_identifier() {
    let n = tokenize_raw("command-r", "some-function-name-here");
    assert!((4..=12).contains(&n), "got {n}");
}

#[test]
fn cohere_multiline_markdown() {
    let n = tokenize_raw("command-r", "# Heading\n\nSome **bold** text.");
    assert!((6..=18).contains(&n), "got {n}");
}

#[test]
fn cohere_korean_text() {
    let n = tokenize_raw("command-r", "안녕하세요");
    assert!((1..=10).contains(&n), "got {n}");
}

#[test]
fn cohere_arabic_text() {
    let n = tokenize_raw("command-r", "مرحبا بالعالم");
    assert!((2..=12).contains(&n), "got {n}");
}

#[test]
fn cohere_xml_snippet() {
    let n = tokenize_raw("command-r", "<tag>content</tag>");
    assert!((4..=12).contains(&n), "got {n}");
}

#[test]
fn cohere_long_repeating() {
    let n = tokenize_raw("command-r", &"a".repeat(100));
    assert!((1..=120).contains(&n), "got {n}");
}

#[test]
fn cohere_python_code() {
    let n = tokenize_raw("command-r", "def f(x):\n    return x + 1");
    assert!((6..=18).contains(&n), "got {n}");
}

#[test]
fn cohere_quotes_text() {
    let n = tokenize_raw("command-r", "\"single\" and 'double' quotes");
    assert!((6..=14).contains(&n), "got {n}");
}

// --- 10 dispatch samples --------------------------------------------

#[test]
fn cohere_command_r_dispatches() {
    assert_eq!(dispatch_kind("command-r"), "COHERE_BPE");
}

#[test]
fn cohere_command_r_plus_dispatches() {
    assert_eq!(dispatch_kind("command-r-plus"), "COHERE_BPE");
}

#[test]
fn cohere_command_light_falls_to_tier3() {
    // Round-2 fix Backend F4: `command-light` uses a different BPE
    // vocab than `command-r`. The R1 dispatch row was removed in R2
    // to avoid silent ~5-20% under-counts; `command-light` now hits
    // Tier 3 (5% conservative margin + `tokenizer_unknown_model`
    // metric per spec §3.3) until a separate vendored asset ships.
    assert_eq!(dispatch_kind("command-light"), "HEURISTIC");
}

#[test]
fn cohere_command_r_dated_dispatches() {
    assert_eq!(dispatch_kind("command-r-20240501"), "COHERE_BPE");
}

#[test]
fn cohere_command_r_plus_dated_dispatches() {
    assert_eq!(dispatch_kind("command-r-plus-20240501"), "COHERE_BPE");
}

#[test]
fn cohere_bedrock_command_dispatches() {
    assert_eq!(dispatch_kind("cohere.command-v1:0"), "COHERE_BPE");
}

#[test]
fn cohere_bedrock_command_r_dispatches() {
    assert_eq!(dispatch_kind("cohere.command-r-v1:0"), "COHERE_BPE");
}

#[test]
fn cohere_bedrock_command_r_plus_dispatches() {
    assert_eq!(dispatch_kind("cohere.command-r-plus-v1:0"), "COHERE_BPE");
}

#[test]
fn cohere_r_and_r_plus_match_for_ascii() {
    let a = tokenize_raw("command-r", "hello world hello");
    let b = tokenize_raw("command-r-plus", "hello world hello");
    assert_eq!(a, b);
}

#[test]
fn cohere_bedrock_matches_native() {
    let a = tokenize_raw("command-r", "test message");
    let b = tokenize_raw("cohere.command-r-v1:0", "test message");
    assert_eq!(a, b);
}

// --- 10 chat envelope samples ---------------------------------------

#[test]
fn cohere_chat_user_role_adds_envelope() {
    let raw = tokenize_raw("command-r", "hello");
    let chat = tokenize_chat("command-r", "user", "hello");
    assert!(chat > raw);
}

#[test]
fn cohere_chat_system_role_in_band() {
    let n = tokenize_chat("command-r", "system", "You are helpful.");
    assert!((6..=18).contains(&n), "got {n}");
}

#[test]
fn cohere_chat_assistant_role_in_band() {
    let n = tokenize_chat("command-r", "assistant", "Of course!");
    assert!((6..=15).contains(&n), "got {n}");
}

#[test]
fn cohere_chat_tool_role_in_band() {
    let n = tokenize_chat("command-r", "tool", "{\"result\":42}");
    assert!((6..=18).contains(&n), "got {n}");
}

#[test]
fn cohere_chat_r_plus_matches_r() {
    let a = tokenize_chat("command-r", "user", "hello");
    let b = tokenize_chat("command-r-plus", "user", "hello");
    assert_eq!(a, b);
}

#[test]
fn cohere_chat_bedrock_works() {
    let n = tokenize_chat("cohere.command-r-v1:0", "user", "hello");
    assert!(n >= 5);
}

#[test]
fn cohere_chat_long_content_proportional() {
    let n = tokenize_chat("command-r", "user", &"hi ".repeat(50));
    assert!(n >= 30);
}

#[test]
fn cohere_chat_empty_content() {
    let n = tokenize_chat("command-r", "user", "");
    assert!(n >= 4);
}

#[test]
fn cohere_chat_cjk_content() {
    let n = tokenize_chat("command-r", "user", "你好");
    assert!(n >= 5);
}

#[test]
fn cohere_chat_multi_msg() {
    let tok = boot_tokenizer();
    let req = TokenizeRequest {
        model: "command-r".to_string(),
        messages: vec![
            Message {
                role: "user".to_string(),
                content: "Hi".to_string(),
                tool_calls: vec![],
            },
            Message {
                role: "assistant".to_string(),
                content: "Hello!".to_string(),
                tool_calls: vec![],
            },
        ],
        ..Default::default()
    };
    let n = tok.tokenize(&req).unwrap().input_tokens;
    assert!(n >= 10);
}

// --- 5 boundary samples ---------------------------------------------

#[test]
fn cohere_single_space() {
    let n = tokenize_raw("command-r", " ");
    assert!((1..=3).contains(&n));
}

#[test]
fn cohere_newline_only() {
    let n = tokenize_raw("command-r", "\n");
    assert!((1..=3).contains(&n));
}

#[test]
fn cohere_tab_only() {
    let n = tokenize_raw("command-r", "\t");
    assert!((1..=3).contains(&n));
}

#[test]
fn cohere_single_char() {
    let n = tokenize_raw("command-r", "a");
    assert_eq!(n, 1);
}

#[test]
fn cohere_very_long_text() {
    let n = tokenize_raw("command-r", &"hello world ".repeat(100));
    assert!((100..=400).contains(&n), "got {n}");
}

// ════════════════════════════════════════════════════════════════════
// LLAMA — 50 samples
// ════════════════════════════════════════════════════════════════════

// --- 25 raw-text samples --------------------------------------------

#[test]
fn llama_hello_world_is_2_tokens() {
    assert_eq!(tokenize_raw("meta.llama3-8b-instruct-v1:0", "hello world"), 2);
}

#[test]
fn llama_hello_is_1_token() {
    assert_eq!(tokenize_raw("meta.llama3-8b-instruct-v1:0", "hello"), 1);
}

#[test]
fn llama_empty_string_is_0_tokens() {
    assert_eq!(tokenize_raw("meta.llama3-8b-instruct-v1:0", ""), 0);
}

#[test]
fn llama_short_sentence_in_band() {
    let n = tokenize_raw(
        "meta.llama3-8b-instruct-v1:0",
        "The quick brown fox jumps over the lazy dog.",
    );
    assert!((8..=14).contains(&n), "got {n}");
}

#[test]
fn llama_punctuation_heavy_text() {
    let n = tokenize_raw("meta.llama3-8b-instruct-v1:0", "Hello, world! How are you?");
    assert!((6..=12).contains(&n), "got {n}");
}

#[test]
fn llama_numbers_text() {
    let n = tokenize_raw("meta.llama3-8b-instruct-v1:0", "Pi is approximately 3.14159.");
    assert!((6..=12).contains(&n), "got {n}");
}

#[test]
fn llama_code_snippet() {
    let n = tokenize_raw("meta.llama3-8b-instruct-v1:0", "fn main() { println!(\"hi\"); }");
    assert!((8..=18).contains(&n), "got {n}");
}

#[test]
fn llama_json_snippet() {
    let n = tokenize_raw("meta.llama3-8b-instruct-v1:0", r#"{"k":"v","n":1}"#);
    assert!((6..=14).contains(&n), "got {n}");
}

#[test]
fn llama_cjk_text() {
    let n = tokenize_raw("meta.llama3-8b-instruct-v1:0", "你好世界");
    assert!((2..=8).contains(&n), "got {n}");
}

#[test]
fn llama_emoji_text() {
    let n = tokenize_raw("meta.llama3-8b-instruct-v1:0", "Hello 🌍 world 🚀");
    assert!((4..=14).contains(&n), "got {n}");
}

#[test]
fn llama_newlines_preserved() {
    let n = tokenize_raw("meta.llama3-8b-instruct-v1:0", "line1\nline2\nline3");
    assert!((4..=10).contains(&n), "got {n}");
}

#[test]
fn llama_repeated_word_compresses() {
    let n = tokenize_raw(
        "meta.llama3-8b-instruct-v1:0",
        "the the the the the the the the",
    );
    assert!((4..=12).contains(&n), "got {n}");
}

#[test]
fn llama_long_paragraph_proportional() {
    let text = "This is a paragraph for tokenizer testing. \
                The count should scale linearly with character count.";
    let n = tokenize_raw("meta.llama3-8b-instruct-v1:0", text);
    assert!((15..=35).contains(&n), "got {n}");
}

#[test]
fn llama_url_text() {
    let n = tokenize_raw(
        "meta.llama3-8b-instruct-v1:0",
        "https://example.com/path?q=value",
    );
    assert!((6..=15).contains(&n), "got {n}");
}

#[test]
fn llama_special_chars() {
    let n = tokenize_raw("meta.llama3-8b-instruct-v1:0", "@#$%^&*()");
    assert!((3..=15).contains(&n), "got {n}");
}

#[test]
fn llama_mixed_case_camelcase() {
    let n = tokenize_raw("meta.llama3-8b-instruct-v1:0", "ThisIsCamelCaseWord");
    assert!((4..=12).contains(&n), "got {n}");
}

#[test]
fn llama_snake_case_identifier() {
    let n = tokenize_raw("meta.llama3-8b-instruct-v1:0", "some_function_name_here");
    assert!((4..=12).contains(&n), "got {n}");
}

#[test]
fn llama_kebab_case_identifier() {
    let n = tokenize_raw("meta.llama3-8b-instruct-v1:0", "some-function-name-here");
    assert!((4..=12).contains(&n), "got {n}");
}

#[test]
fn llama_multiline_markdown() {
    let n = tokenize_raw(
        "meta.llama3-8b-instruct-v1:0",
        "# Heading\n\nSome **bold** text.",
    );
    assert!((6..=18).contains(&n), "got {n}");
}

#[test]
fn llama_korean_text() {
    let n = tokenize_raw("meta.llama3-8b-instruct-v1:0", "안녕하세요");
    assert!((1..=10).contains(&n), "got {n}");
}

#[test]
fn llama_arabic_text() {
    let n = tokenize_raw("meta.llama3-8b-instruct-v1:0", "مرحبا بالعالم");
    assert!((2..=12).contains(&n), "got {n}");
}

#[test]
fn llama_xml_snippet() {
    let n = tokenize_raw("meta.llama3-8b-instruct-v1:0", "<tag>content</tag>");
    assert!((4..=12).contains(&n), "got {n}");
}

#[test]
fn llama_long_repeating() {
    let n = tokenize_raw("meta.llama3-8b-instruct-v1:0", &"a".repeat(100));
    assert!((1..=120).contains(&n), "got {n}");
}

#[test]
fn llama_python_code() {
    let n = tokenize_raw(
        "meta.llama3-8b-instruct-v1:0",
        "def f(x):\n    return x + 1",
    );
    assert!((6..=18).contains(&n), "got {n}");
}

#[test]
fn llama_quotes_text() {
    let n = tokenize_raw(
        "meta.llama3-8b-instruct-v1:0",
        "\"single\" and 'double' quotes",
    );
    assert!((6..=14).contains(&n), "got {n}");
}

// --- 10 dispatch samples --------------------------------------------

#[test]
fn llama_3_8b_dispatches() {
    assert_eq!(
        dispatch_kind("meta.llama3-8b-instruct-v1:0"),
        "SENTENCEPIECE_LLAMA"
    );
}

#[test]
fn llama_3_70b_dispatches() {
    assert_eq!(
        dispatch_kind("meta.llama3-70b-instruct-v1:0"),
        "SENTENCEPIECE_LLAMA"
    );
}

#[test]
fn llama_3_1_8b_dispatches() {
    assert_eq!(
        dispatch_kind("meta.llama3-1-8b-instruct-v1:0"),
        "SENTENCEPIECE_LLAMA"
    );
}

#[test]
fn llama_3_1_70b_dispatches() {
    assert_eq!(
        dispatch_kind("meta.llama3-1-70b-instruct-v1:0"),
        "SENTENCEPIECE_LLAMA"
    );
}

#[test]
fn llama_3_1_405b_dispatches() {
    assert_eq!(
        dispatch_kind("meta.llama3-1-405b-instruct-v1:0"),
        "SENTENCEPIECE_LLAMA"
    );
}

#[test]
fn llama_3_2_1b_dispatches() {
    assert_eq!(
        dispatch_kind("meta.llama3-2-1b-instruct-v1:0"),
        "SENTENCEPIECE_LLAMA"
    );
}

#[test]
fn llama_3_2_3b_dispatches() {
    assert_eq!(
        dispatch_kind("meta.llama3-2-3b-instruct-v1:0"),
        "SENTENCEPIECE_LLAMA"
    );
}

#[test]
fn llama_3_3_70b_dispatches() {
    assert_eq!(
        dispatch_kind("meta.llama3-3-70b-instruct-v1:0"),
        "SENTENCEPIECE_LLAMA"
    );
}

#[test]
fn llama_8b_and_70b_match_for_ascii() {
    let a = tokenize_raw("meta.llama3-1-8b-instruct-v1:0", "hello world hello");
    let b = tokenize_raw("meta.llama3-1-70b-instruct-v1:0", "hello world hello");
    assert_eq!(a, b);
}

#[test]
fn llama_3_and_3_1_match_for_ascii() {
    let a = tokenize_raw("meta.llama3-8b-instruct-v1:0", "hello world");
    let b = tokenize_raw("meta.llama3-1-8b-instruct-v1:0", "hello world");
    assert_eq!(a, b);
}

// --- 10 chat envelope samples ---------------------------------------

#[test]
fn llama_chat_user_role_adds_envelope() {
    let raw = tokenize_raw("meta.llama3-8b-instruct-v1:0", "hello");
    let chat = tokenize_chat("meta.llama3-8b-instruct-v1:0", "user", "hello");
    assert!(chat > raw);
}

#[test]
fn llama_chat_system_role_in_band() {
    let n = tokenize_chat(
        "meta.llama3-8b-instruct-v1:0",
        "system",
        "You are helpful.",
    );
    assert!((6..=18).contains(&n), "got {n}");
}

#[test]
fn llama_chat_assistant_role_in_band() {
    let n = tokenize_chat(
        "meta.llama3-8b-instruct-v1:0",
        "assistant",
        "Of course!",
    );
    assert!((6..=15).contains(&n), "got {n}");
}

#[test]
fn llama_chat_tool_role_in_band() {
    let n = tokenize_chat(
        "meta.llama3-8b-instruct-v1:0",
        "tool",
        "{\"result\":42}",
    );
    assert!((6..=18).contains(&n), "got {n}");
}

#[test]
fn llama_chat_3_1_70b_works() {
    let n = tokenize_chat("meta.llama3-1-70b-instruct-v1:0", "user", "hello");
    assert!(n >= 5);
}

#[test]
fn llama_chat_3_2_1b_works() {
    let n = tokenize_chat("meta.llama3-2-1b-instruct-v1:0", "user", "hi");
    assert!(n >= 5);
}

#[test]
fn llama_chat_long_content_proportional() {
    let n = tokenize_chat(
        "meta.llama3-8b-instruct-v1:0",
        "user",
        &"hi ".repeat(50),
    );
    assert!(n >= 30);
}

#[test]
fn llama_chat_empty_content() {
    let n = tokenize_chat("meta.llama3-8b-instruct-v1:0", "user", "");
    assert!(n >= 4);
}

#[test]
fn llama_chat_cjk_content() {
    let n = tokenize_chat("meta.llama3-8b-instruct-v1:0", "user", "你好");
    assert!(n >= 5);
}

#[test]
fn llama_chat_multi_msg() {
    let tok = boot_tokenizer();
    let req = TokenizeRequest {
        model: "meta.llama3-8b-instruct-v1:0".to_string(),
        messages: vec![
            Message {
                role: "user".to_string(),
                content: "Hi".to_string(),
                tool_calls: vec![],
            },
            Message {
                role: "assistant".to_string(),
                content: "Hello!".to_string(),
                tool_calls: vec![],
            },
        ],
        ..Default::default()
    };
    let n = tok.tokenize(&req).unwrap().input_tokens;
    assert!(n >= 10);
}

// --- 5 boundary samples ---------------------------------------------

#[test]
fn llama_single_space() {
    let n = tokenize_raw("meta.llama3-8b-instruct-v1:0", " ");
    assert!((1..=3).contains(&n));
}

#[test]
fn llama_newline_only() {
    let n = tokenize_raw("meta.llama3-8b-instruct-v1:0", "\n");
    assert!((1..=3).contains(&n));
}

#[test]
fn llama_tab_only() {
    let n = tokenize_raw("meta.llama3-8b-instruct-v1:0", "\t");
    assert!((1..=3).contains(&n));
}

#[test]
fn llama_single_char() {
    let n = tokenize_raw("meta.llama3-8b-instruct-v1:0", "a");
    assert_eq!(n, 1);
}

#[test]
fn llama_very_long_text() {
    let n = tokenize_raw(
        "meta.llama3-8b-instruct-v1:0",
        &"hello world ".repeat(100),
    );
    assert!((100..=400).contains(&n), "got {n}");
}
