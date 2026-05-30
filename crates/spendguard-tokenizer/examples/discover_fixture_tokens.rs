use spendguard_tokenizer::asset_sha256;
use tiktoken_rs::{cl100k_base_singleton, o200k_base_singleton, p50k_base_singleton};

fn main() {
    println!("asset_sha256::CL100K_BASE = {}", asset_sha256::CL100K_BASE);
    println!("asset_sha256::O200K_BASE = {}", asset_sha256::O200K_BASE);
    println!("asset_sha256::P50K_BASE = {}", asset_sha256::P50K_BASE);

    let fixture = "spendguard-cross-check-fixture-v1alpha1";

    let cl = cl100k_base_singleton();
    let toks = cl.encode_with_special_tokens(fixture);
    println!("cl100k tokens for `{}`: {:?}", fixture, toks);

    let o = o200k_base_singleton();
    let toks2 = o.encode_with_special_tokens(fixture);
    println!("o200k tokens for `{}`: {:?}", fixture, toks2);

    let p = p50k_base_singleton();
    let toks3 = p.encode_with_special_tokens(fixture);
    println!("p50k tokens for `{}`: {:?}", fixture, toks3);
}
