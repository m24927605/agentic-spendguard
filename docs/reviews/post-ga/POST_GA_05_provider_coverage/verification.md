# POST_GA_05 Provider Coverage Verification

Date: 2026-06-02
Branch: `post-ga/POST_GA_05_provider_coverage`

## Acceptance Gates

| Gate | Command | Result |
|---|---|---|
| Format + tests | `cargo fmt --manifest-path services/tokenizer/Cargo.toml && cargo test --manifest-path services/tokenizer/Cargo.toml` | PASS: lib 93, main 12, golden 51, slice04 golden 203, slice05 chaos 3 |
| Build | `cargo build --manifest-path services/tokenizer/Cargo.toml` | PASS |
| Helm demo render | `helm template charts/spendguard --set chart.profile=demo` | PASS, 1443 rendered lines |
| Helm production render | `helm template charts/spendguard --set chart.profile=production ...` | PASS, 1511 rendered lines; Llama Bedrock region path exercised without provider Secret |
| Hot-path invariant | `rg -n "provider_clients\|CohereClient\|LlamaClient\|COHERE\|LLAMA\|count_tokens\\(" services/sidecar services/egress_proxy` | PASS: no tokenizer provider clients in sidecar/egress_proxy; only existing Bedrock provider model classifier regexes match Cohere/Llama names |
| Demo | `make demo-down && ANTHROPIC_API_KEY= GEMINI_API_KEY= COHERE_API_KEY= LLAMA_BEDROCK_REGION= LLAMA_COUNT_TOKENS_BASE_URL= LLAMA_API_KEY= make demo-up DEMO_MODE=default` | PASS: Step 8 assertions, outbox closure, canonical_events receipt |

## Test Coverage Added

- Cohere client success, auth, rate-limit, and schema-drift cases.
- Llama Bedrock-compatible client success, auth, rate-limit, and schema-drift cases.
- Worker dispatch paths for Cohere and Llama.
- PII opt-in denial proves raw text is not sent to Cohere/Llama.
- CountTokens quota test proves per-tenant/per-provider isolation across Cohere and Llama.
- Config Debug masking covers Cohere/Llama API keys and Llama CountTokens base URL presence.

## Real Provider Tests

Optional real Cohere/Llama provider tests were not run because sanitized local verification did not include `COHERE_API_KEY` or `LLAMA_BEDROCK_REGION`/AWS credentials. Mocked wire-contract tests cover the request envelopes without sending prompt text outside the local machine.

## Staff+ Decisions Locked For Review

| Role | Decision |
|---|---|
| Software Architect | Keep expansion scoped to tokenizer shadow worker only; no sidecar/egress_proxy hot-path dependency. |
| Backend Architect | Llama Tier 1 uses Bedrock Runtime CountTokens because the locked tokenizer spec dispatches Llama via Bedrock model IDs. |
| Security Engineer | New provider clients redact provider error bodies and mask base URLs in Debug output. |
| Database Optimizer | Reuse existing tokenizer_t1_samples, PII opt-in, quota, and sample-rate state; no schema change. |
| Tokenizer Domain Expert | Cohere count is `tokens.len()` from `/v1/tokenize`; Llama count is Bedrock `inputTokens`. |
