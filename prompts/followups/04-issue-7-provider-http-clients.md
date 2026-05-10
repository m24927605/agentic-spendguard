# Followup #7 — Real OpenAI + Anthropic HTTP clients (S11/S12)

GitHub issue: https://github.com/m24927605/agentic-flow-cost-evaluation/issues/7

## Goal

Replace the stub `OpenAiClient` and `AnthropicClient` in
`services/usage_poller/src/lib.rs` with real `reqwest`-backed HTTP
implementations. Today both stubs return typed errors; only `MockProviderClient`
works, so the demo's S11+S12 path is shaped but not exercising real
billing-API integration.

## Files to read first

- `services/usage_poller/src/lib.rs` — full file:
  - `ProviderClient` trait
  - `MockProviderClient` (reference: deterministic fixtures)
  - `OpenAiClient` stub
  - `AnthropicClient` stub
  - `PollWindow`, `ProviderUsageRecord`, the canonical idempotency hash
- `services/usage_poller/src/main.rs` — cursor + safety_lag (round 5 fixed
  cold-start, round 10 made provider_kind explicit)
- `services/ledger/migrations/0025_provider_usage_records.sql` — schema for
  what the poll inserts; use `idempotency_key` exactly as documented in the
  migration comment
- Documentation links inside the stubs for the actual API endpoints

## Acceptance criteria

- `reqwest = { version = "0.12", features = ["json", "rustls-tls"] }` and
  `wiremock = "0.6"` (dev) added to `services/usage_poller/Cargo.toml`
- `OpenAiClient::fetch_usage_window`:
  - `GET https://api.openai.com/v1/organization/usage/{kind}` with
    `Authorization: Bearer ${api_key}`,
    `OpenAI-Organization: ${org_id}`,
    `OpenAI-Project: ${project_id}` (when set)
  - Query params: `start_time`, `end_time`, `bucket_width=1m`, `limit=200`
  - Cursor pagination via `page` token in response, follow until exhausted
  - Translate each row into `ProviderUsageRecord` with
    `provider="openai"`, populated `provider_account` (org/project),
    `provider_request_id`, `model_id`, `prompt_tokens`, `completion_tokens`,
    `cost_micros_usd` (convert from $-decimal if needed)
  - `idempotency_key = sha256(provider || ':' || provider_request_id || ':' || provider_account || ':' || event_kind)`
- `AnthropicClient::fetch_usage_window`:
  - `GET https://api.anthropic.com/v1/organizations/{workspace_id}/usage_report`
    with `x-api-key`, `anthropic-version: 2023-06-01`
  - Same translation contract; `provider="anthropic"`
- Rate limits (both):
  - On 429 + `Retry-After` header: sleep, retry once, then surface
    `LowLevelError::RateLimited` so the poller's outer loop retries on
    next cycle
  - On 5xx: similar one-shot retry with bounded backoff
  - On 401/403: surface immediately as `Authentication` (no retry)
- Round-10 explicit-enum match arm in main.rs continues to bail on unknown
  `provider_kind`; make sure it still returns the correct error variant
  for typos
- 8+ integration tests using `wiremock`:
  - 200-OK happy path with 1 page
  - 200-OK happy path with 2-page cursor
  - 429-with-Retry-After triggers backoff + retry
  - 401 returns Authentication immediately
  - Empty body
  - Malformed JSON
  - Provider-account mismatch (defensive: skip + log)
  - Idempotency-key matches the migration's documented hash byte-for-byte
- `cargo check --tests` clean for spendguard-usage-poller (uses the round-1
  rand 0.8 dev-dep already added in PR #2)

## Pattern references

- `MockProviderClient` is the trait's smallest correct implementation —
  copy its shape, replace the deterministic fixture with HTTP
- `services/canonical_ingest/src/handlers/append_events.rs` for how to
  use `reqwest` + structured error mapping if needed (pre-existing in
  the codebase)
- The canonical idempotency hash format is documented at the top of
  `services/ledger/migrations/0025_provider_usage_records.sql` —
  must match byte-for-byte so dedupe works against the matcher SP

## Verification

```bash
cargo check --tests
cargo test -p spendguard-usage-poller
```

Plus a manual smoke against a test API key:

```bash
SPENDGUARD_USAGE_POLLER_PROVIDER_KIND=openai \
SPENDGUARD_USAGE_POLLER_OPENAI_API_KEY=$THROWAWAY_KEY \
SPENDGUARD_USAGE_POLLER_OPENAI_ORG_ID=$ORG \
cargo run --bin spendguard-usage-poller -- --window 1h
# expect log lines showing real fetched= count > 0 from openai
```

No live HTTP calls in CI; all tests use wiremock fixtures.

## Commit + close

```
feat(s11/s12): real OpenAI + Anthropic HTTP poller backends (followup #7)

Replaces the stub `OpenAiClient` and `AnthropicClient` with real
reqwest-backed implementations. Cursor pagination, Retry-After
backoff, typed auth/rate-limit errors. Idempotency hash matches
migration 0025's documented format byte-for-byte so dedupe works
against the existing matcher SP.

Tests: 8 wiremock integration tests + 4 unit tests for hash format.
Manual smoke against throwaway OpenAI org confirmed real records
land.
```

After merge: `gh issue close 7 --comment "Shipped in <commit-sha>"`.
