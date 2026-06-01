# POST_GA_04 Evidence Summary

Generated on 2026-06-02 from branch
`post-ga/POST_GA_04_tokenizer_asset_performance`.

## Commands

- `bash crates/spendguard-tokenizer/data/recompute_sha256.sh`
- `du -h crates/spendguard-tokenizer/data/*.tiktoken crates/spendguard-tokenizer/data/*/tokenizer.json`
- `cargo tree --manifest-path services/sidecar/Cargo.toml | rg 'spendguard-tokenizer|tokenizers|tiktoken' || true`
- `cargo bench --manifest-path benchmarks/tokenizer/Cargo.toml --bench tier2_library -- --sample-size 10 --warm-up-time 1 --measurement-time 3`
- `cargo bench --manifest-path benchmarks/tokenizer/Cargo.toml --features cohere --bench tier2_library -- --sample-size 10 --warm-up-time 1 --measurement-time 3`
- `for i in $(seq 1 10); do cargo run --manifest-path benchmarks/tokenizer/Cargo.toml --release --bin tokenizer_cold_start_once --quiet; done`

## Findings

- #95: `sidecar-tokenizer-tree.txt` contains only the header line. The
  sidecar compile graph no longer includes `spendguard-tokenizer`,
  `tokenizers`, or `tiktoken-rs`.
- #102: dual-copy OpenAI assets remain intentional. Layer A verifies
  SpendGuard-owned mirrored `.tiktoken` bytes; Layer B cross-checks the
  actual tiktoken-rs singleton runtime path. The sidecar no longer pays
  this asset cost because it does not directly tokenize after SLICE_10.
- #122: eager-load remains the production default. Boot cost is now
  observable through `spendguard_tokenizer_encoder_boot_duration_ms`;
  fresh-process cold-start p99 was 1.370s from 10 process launches.
- #125: `checksums.txt` enumerates all 7 vendored assets.
- #130: dispatch uses `RegexSet`; routing tests and benchmarks passed.
- #140: Llama envelope remains `per_message=5`, `reply_priming=0`,
  `BOS=1` for Bedrock. POST_GA_04 pins tests and benchmark evidence;
  no tokenizer version ID changed because no asset was repacked.

## Percentiles

`benchmark-percentiles.tsv` is derived from Criterion `sample.json`
files using `times / iters` per sample. `cold-start-percentiles.tsv`
is derived from `cold-start-fresh-process.tsv`, where each row comes
from a separate `tokenizer_cold_start_once` process. With 10 samples,
p95 and p99 are the maximum observed sample.

Key warm-path p99 values:

- `raw_text_gpt_4o_chars/1000`: 591.048us
- `raw_text_gpt_4_1000_chars`: 679.801us
- `raw_text_claude_3_5_sonnet_1000_chars`: 265.772us
- `raw_text_gemini_1_5_pro_1000_chars`: 208.439us
- `raw_text_command_r_plus_1000_chars`: 262.950us
- `raw_text_llama_3_1_70b_1000_chars`: 228.750us
- Fresh-process cold start: 1.370s

10K stress p99 values:

- OpenAI `gpt-4o` pathological repeated char: 41.542ms
- Anthropic: 2.158ms
- Gemini: 2.540ms
- Cohere: 2.451ms
- Llama: 2.294ms
