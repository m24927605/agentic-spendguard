# POST_GA_05 Round 4 Codex Review Summary

Date: 2026-06-02
Reviewer: codex CLI direct review fallback after AIT rejected `--review-mode`.

## Findings

- P2: Documented Cohere dated model IDs such as `command-r-plus-08-2024` did not route to the Cohere encoder; only undated and `YYYYMMDD` test-style suffixes routed.
- P2: Bedrock Llama SDK service errors embedded provider-controlled `Debug` detail in `ProviderError`, which `worker::on_provider_error` can log.

## Resolution

- `crates/spendguard-tokenizer/src/dispatch.rs` now accepts Cohere native dated suffixes in both `YYYYMMDD` and `MM-YYYY` forms.
- `services/tokenizer/tests/slice04_golden_samples.rs` pins `command-r-08-2024` and `command-r-plus-08-2024` dispatch to Cohere.
- `services/tokenizer/src/shadow/provider_clients/llama.rs` maps Bedrock service errors to redacted static categories before constructing `ProviderError`.

## Acceptance Rerun

- `cargo fmt --manifest-path crates/spendguard-tokenizer/Cargo.toml`
- `cargo fmt --manifest-path services/tokenizer/Cargo.toml`
- `cargo test --manifest-path crates/spendguard-tokenizer/Cargo.toml --features cohere`: PASS, lib 135, cross-check schema 2, seed parity 15
- `cargo test --manifest-path services/tokenizer/Cargo.toml`: PASS, lib 99, main 13, golden 51, slice04 golden 205, slice05 chaos 3
- `cargo build --manifest-path services/tokenizer/Cargo.toml`: PASS
- `helm template charts/spendguard --set chart.profile=demo`: PASS, 1443 lines
- `helm template charts/spendguard -f charts/spendguard/values-production.example.yaml --set chart.profile=production`: PASS, 2157 lines
- Hot-path grep: PASS, no tokenizer provider clients in sidecar/egress_proxy
- `make demo-down` then `ANTHROPIC_API_KEY= GEMINI_API_KEY= COHERE_API_KEY= LLAMA_BEDROCK_REGION= LLAMA_COUNT_TOKENS_BASE_URL= LLAMA_API_KEY= make demo-up DEMO_MODE=default`: PASS, Step 8 assertions, outbox total=7 forwarded=7, canonical_events count=5
