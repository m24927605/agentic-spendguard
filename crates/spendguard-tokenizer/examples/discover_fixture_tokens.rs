//! Cross-check fixture vector discovery helper.
//!
//! Spec ref `tokenizer-service-spec-v1alpha1.md` §7.4.1.
//!
//! Run this example after bumping `tiktoken-rs` (SLICE_03 encoders)
//! or any of the vendored `tokenizer.json` files (SLICE_04 encoders)
//! to refresh the `EXPECTED_*` token vectors that the Layer B
//! cross-check guard pins.
//!
//! Usage:
//!   cargo run --release --example discover_fixture_tokens
//!
//! Output is sorted in the same order as the per-encoder
//! `EXPECTED_*` constants live in the source tree so a copy-paste
//! into the `encoders/<vendor>.rs` files is mechanical.

use spendguard_tokenizer::asset_sha256;
use tiktoken_rs::{cl100k_base_singleton, o200k_base_singleton, p50k_base_singleton};
use tokenizers::Tokenizer;

const FIXTURE: &str = "spendguard-cross-check-fixture-v1alpha1";

fn print_tiktoken_block() {
    println!("// ── SLICE_03 — OpenAI tiktoken-rs ─────────────────────");
    println!("// asset_sha256::CL100K_BASE = {}", asset_sha256::CL100K_BASE);
    println!("// asset_sha256::O200K_BASE = {}", asset_sha256::O200K_BASE);
    println!("// asset_sha256::P50K_BASE = {}", asset_sha256::P50K_BASE);
    println!();

    let cl = cl100k_base_singleton();
    println!(
        "EXPECTED_CL100K_FIXTURE = {:?}",
        cl.encode_with_special_tokens(FIXTURE)
    );

    let o = o200k_base_singleton();
    println!(
        "EXPECTED_O200K_FIXTURE = {:?}",
        o.encode_with_special_tokens(FIXTURE)
    );

    let p = p50k_base_singleton();
    println!(
        "EXPECTED_P50K_FIXTURE = {:?}",
        p.encode_with_special_tokens(FIXTURE)
    );
    println!();
}

fn print_tier2_block(label: &str, asset_path: &str, sha256_const: &str) {
    println!("// ── SLICE_04 — {} ──────────────────────", label);
    println!("// asset_sha256 = {}", sha256_const);
    let bytes = std::fs::read(asset_path).expect("read vendored tokenizer.json");
    let tok = Tokenizer::from_bytes(&bytes).expect("parse tokenizer.json");
    let enc = tok.encode(FIXTURE, false).expect("encode fixture");
    println!("EXPECTED_{}_FIXTURE = {:?}", label, enc.get_ids());
    println!();
}

fn main() {
    println!("// Cross-check fixture token vector discovery");
    println!("// Fixture: {:?}", FIXTURE);
    println!();

    print_tiktoken_block();

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    print_tier2_block(
        "ANTHROPIC",
        &format!("{manifest_dir}/data/anthropic-claude3/tokenizer.json"),
        asset_sha256::ANTHROPIC_CLAUDE3,
    );
    print_tier2_block(
        "GEMINI",
        &format!("{manifest_dir}/data/gemini-1.5/tokenizer.json"),
        asset_sha256::GEMINI_15,
    );
    #[cfg(feature = "cohere")]
    print_tier2_block(
        "COHERE",
        &format!("{manifest_dir}/data/cohere-command-r/tokenizer.json"),
        asset_sha256::COHERE_COMMAND_R,
    );
    #[cfg(not(feature = "cohere"))]
    println!("// ── SLICE_04 — COHERE skipped (feature `cohere` disabled) ──");
    print_tier2_block(
        "LLAMA",
        &format!("{manifest_dir}/data/llama-3.1/tokenizer.json"),
        asset_sha256::LLAMA_31,
    );
}
