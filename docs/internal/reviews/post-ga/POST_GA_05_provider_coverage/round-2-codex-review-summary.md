# POST_GA_05 Round 2 Codex Review Summary

Date: 2026-06-02
Reviewer: codex CLI direct review fallback after AIT rejected `--review-mode`.

## Findings

- P2: Llama Tier 1 used Bedrock Converse CountTokens while Tier 2 Llama raw_text counts the raw Bedrock InvokeModel prompt plus BOS. This would create false drift for normal raw_text samples.
- P2: Cohere Tier 1 used native `/v1/tokenize` count without the Bedrock BOS token that Tier 2 raw_text adds for non-empty Cohere text. Short prompts could exceed the 1.5% drift threshold.

## Resolution

- `services/tokenizer/src/shadow/provider_clients/llama.rs` now sends CountTokens with the InvokeModel body shape and the HTTP-compatible mock verifies the same base64 body envelope.
- `services/tokenizer/src/shadow/provider_clients/cohere.rs` now adds the locked Bedrock BOS count for non-empty raw text and explicitly does not add it for empty raw text.
- `services/tokenizer/src/shadow/worker.rs` test expectations now assert the Cohere persisted T1 count includes BOS.

## Acceptance Rerun

- `cargo fmt --manifest-path services/tokenizer/Cargo.toml`
- `cargo test --manifest-path services/tokenizer/Cargo.toml`: PASS, lib 96, main 13, golden 51, slice04 golden 203, slice05 chaos 3
- `cargo build --manifest-path services/tokenizer/Cargo.toml`: PASS
- `helm template charts/spendguard --set chart.profile=demo`: PASS, 1443 lines
- `helm template charts/spendguard -f charts/spendguard/values-production.example.yaml --set chart.profile=production`: PASS, 2157 lines
- Hot-path grep: PASS, no tokenizer provider clients in sidecar/egress_proxy
- `make demo-down` then `ANTHROPIC_API_KEY= GEMINI_API_KEY= COHERE_API_KEY= LLAMA_BEDROCK_REGION= LLAMA_COUNT_TOKENS_BASE_URL= LLAMA_API_KEY= make demo-up DEMO_MODE=default`: PASS, Step 8 assertions, outbox total=7 forwarded=7, canonical_events count=5
