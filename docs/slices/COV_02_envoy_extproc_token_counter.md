# COV_02 — D01 Envoy ExtProc: token counter

> **Deliverable**: D01 Envoy AI Gateway ExtProc sidecar
> **Slice**: 2 of 7 (S)
> **Spec set**: [`docs/specs/coverage/D01_envoy_extproc/`](../specs/coverage/D01_envoy_extproc/)

## Scope

Wire `spendguard-tokenizer` library + `provider-routing::resolve_tokenizer_kind` into the ExtProc Request-Body phase. On Request-Body callback, parse the OpenAI / Anthropic / Bedrock request shape, extract `messages` + `model`, dispatch to the correct tokenizer, and emit an internal `ClaimEstimate { input_tokens, model_id, provider }` carried through subsequent SLICE 3 budget decision and SLICE 4 audit emit.

Concretely:
- `services/envoy_extproc/Cargo.toml` — add deps:
  - `spendguard-tokenizer = { path = "../../crates/spendguard-tokenizer" }`
  - `spendguard-provider-routing = { path = "../../crates/spendguard-provider-routing" }` (already present)
  - `bytes = "1"` (likely already present)
- `services/envoy_extproc/src/parse.rs` — new module:
  - `pub fn parse_request_body(path: &str, body: &bytes::Bytes) -> Result<ParsedRequest, ParseError>`
  - `ParsedRequest { provider: ProviderKind, model_id: String, messages: Vec<Message> }`
  - Dispatches on `path` via `provider-routing::route(path)` → provider; then per-provider JSON parsing (OpenAI shape, Anthropic shape, Bedrock shape).
- `services/envoy_extproc/src/tokenize.rs` — new module:
  - `pub fn estimate_tokens(parsed: &ParsedRequest) -> Result<ClaimEstimate, TokenizeError>`
  - Looks up tokenizer via `provider-routing::resolve_tokenizer_kind(parsed.provider, &parsed.model_id)` → tokenizer kind; runs `spendguard-tokenizer` API on messages.
- `services/envoy_extproc/src/server.rs` — extend the `Process` stream handler:
  - On `ProcessingRequest::RequestBody`: call `parse_request_body` + `estimate_tokens`. Stash the `ClaimEstimate` in per-stream state keyed by the stream ID. On error: log + fall through with no estimate (don't fail the request — SLICE 3 will fail-closed if estimate missing).
- `services/envoy_extproc/src/state.rs` — new module: per-stream state map (`DashMap<StreamId, StreamState>` or `tokio::sync::Mutex<HashMap<...>>`).

## Files touched

| File | Why |
|------|-----|
| `services/envoy_extproc/Cargo.toml` | Add spendguard-tokenizer dep |
| `services/envoy_extproc/src/parse.rs` | OpenAI/Anthropic/Bedrock request body parser |
| `services/envoy_extproc/src/tokenize.rs` | Token estimate via dispatch |
| `services/envoy_extproc/src/state.rs` | Per-stream state map |
| `services/envoy_extproc/src/server.rs` | Wire Request-Body phase |
| `services/envoy_extproc/src/lib.rs` | Module declarations |

## Test/verification plan

1. `cargo build --manifest-path services/envoy_extproc/Cargo.toml` clean.
2. `cargo test --manifest-path services/envoy_extproc/Cargo.toml` — new tests:
   - `parse::tests::parses_openai_chat_completions_body`
   - `parse::tests::parses_anthropic_messages_body`
   - `parse::tests::parses_bedrock_anthropic_body`
   - `parse::tests::rejects_unknown_provider_path` (returns ParseError, no panic)
   - `tokenize::tests::estimates_tokens_for_openai_model`
   - `tokenize::tests::estimates_tokens_for_anthropic_model`
   - Integration test extending `handshake_smoke.rs`: send a Request-Headers + Request-Body sequence with a real OpenAI chat completion body; assert the stream state map contains a ClaimEstimate with `input_tokens > 0` for that stream ID.
3. SLICE 1's existing 8 unit + 1 integration test STILL pass.
4. `cargo fmt --check` clean.

## Anti-scope

- No budget decision translation (`RequestDecision` RPC to sidecar) — SLICE 3.
- No audit emission — SLICE 4.
- No Helm — SLICE 6.
- No new demo mode — SLICE 7.

## Backlinks

- Spec set: [`design.md`](../specs/coverage/D01_envoy_extproc/design.md) §4 slice 2 row
- SLICE 1: [`COV_01_envoy_extproc_skeleton.md`](COV_01_envoy_extproc_skeleton.md)
