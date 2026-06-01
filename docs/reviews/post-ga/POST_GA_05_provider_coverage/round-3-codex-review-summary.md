# POST_GA_05 Round 3 Codex Review Summary

Date: 2026-06-02
Reviewer: codex CLI direct review fallback after AIT rejected `--review-mode`.

## Findings

- P2: Cohere Bedrock model IDs with non-zero numeric revision suffixes, for example `cohere.command-r-v1:1`, were routed by the tokenizer dispatch table but not normalized before calling Cohere native `/v1/tokenize`.
- P2: `CohereClient` derived `Debug`, exposing the raw API key and any userinfo embedded in `base_url`.

## Resolution

- `services/tokenizer/src/shadow/provider_clients/cohere.rs` now strips any numeric Bedrock revision suffix before deriving Cohere native model names.
- `CohereClient` now has a custom `Debug` implementation that redacts the API key and displays only URL scheme/host/port, removing userinfo and path material from the displayed base URL.
- Added tests for non-zero Bedrock revision normalization and Cohere debug redaction.

## Acceptance Rerun

- `cargo fmt --manifest-path services/tokenizer/Cargo.toml`
- `cargo test --manifest-path services/tokenizer/Cargo.toml`: PASS, lib 98, main 13, golden 51, slice04 golden 203, slice05 chaos 3
- `cargo build --manifest-path services/tokenizer/Cargo.toml`: PASS
- `helm template charts/spendguard --set chart.profile=demo`: PASS, 1443 lines
- `helm template charts/spendguard -f charts/spendguard/values-production.example.yaml --set chart.profile=production`: PASS, 2157 lines
- Hot-path grep: PASS, no tokenizer provider clients in sidecar/egress_proxy
- `make demo-down` then `ANTHROPIC_API_KEY= GEMINI_API_KEY= COHERE_API_KEY= LLAMA_BEDROCK_REGION= LLAMA_COUNT_TOKENS_BASE_URL= LLAMA_API_KEY= make demo-up DEMO_MODE=default`: PASS, Step 8 assertions, outbox total=7 forwarded=7, canonical_events count=5
