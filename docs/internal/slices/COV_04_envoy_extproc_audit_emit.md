# COV_04 — D01 Envoy ExtProc: audit emit (Response-Body phase)

> **Deliverable**: D01 Envoy AI Gateway ExtProc sidecar
> **Slice**: 4 of 7 (M)
> **Spec set**: [`docs/specs/coverage/D01_envoy_extproc/`](../../specs/coverage/D01_envoy_extproc/)

## Scope

Wire the ExtProc Response-Body phase to emit a single `LLM_CALL_POST` event over the sidecar adapter's `EmitTraceEvents` bidi RPC. Two outcome paths:
- **Upstream success (2xx)**: emit `LLM_CALL_POST.SUCCESS` with provider-reported usage extracted from the response body
- **Upstream failure (5xx / sidecar-rejected request)**: emit `LLM_CALL_POST` with `outcome=RUN_ABORTED` and the failure code

Idempotency: keyed by the `reservation_id` SLICE 3 stashed on `StreamState`. Replay-safe: if the same reservation_id is emitted twice, the sidecar's POST_GA_01 dedup catches it.

Carry-over from SLICE 3 R1 deferrals: implement the 60s TTL sweep on `StreamStateMap` (M2 from R1). The audit emit needs to remove the entry after EmitTraceEvents acks; for streams that error out before Response-Body lands, the 60s sweep is the only reaper.

Concretely:
- `services/envoy_extproc/src/audit.rs` — NEW:
  - `pub fn build_llm_call_post(state: &StreamState, response_meta: &ResponseMeta) -> AppendEventsRequest`
  - Maps `StreamState.decision_outcome` (SLICE 3) + `ResponseMeta { http_status, provider_usage }` → AppendEventsRequest with single TraceEvent of kind LLM_CALL_POST
  - For successful streams: outcome = SUCCESS, populate `actual_input_tokens` + `actual_output_tokens` from provider-reported usage
  - For Rejected (DegradeFailClosed/Deny/RequireApproval per SLICE 3 R2 enum) + SidecarError + MissingClaimEstimate outcomes: outcome = RUN_ABORTED with the matching audit_code
- `services/envoy_extproc/src/response_parse.rs` — NEW:
  - `pub fn extract_provider_usage(body: &[u8], provider: ProviderHint) -> Result<ProviderUsage, ParseError>`
  - Provider-agnostic for v1: handles OpenAI / Anthropic JSON shapes (`usage.prompt_tokens` / `usage.completion_tokens`; `usage.input_tokens` / `usage.output_tokens`)
  - Unknown provider → returns ProviderUsage with `tokens_unknown: true` flag (SLICE 4 still emits SUCCESS but lets calibration audit notice)
- `services/envoy_extproc/src/server.rs` — extend Response-Headers + Response-Body phases:
  - Response-Headers: stash http_status on `StreamState`
  - Response-Body: extract provider usage, build LLM_CALL_POST, call sidecar EmitTraceEvents bidi stream, drain ack, remove StreamState entry on success
- `services/envoy_extproc/src/sidecar_client.rs` — extend:
  - `pub async fn emit_trace_events(&self, req: AppendEventsRequest) -> Result<EmitTraceEventsResponse, ClientError>`
  - Same UDS+mTLS connection used by request_decision; reuse the SidecarClient connection.
- `services/envoy_extproc/src/state.rs` — extend `StreamStateMap`:
  - Add `tokio::spawn` background sweep loop (or lazy-check in `get`/`upsert`) that removes entries where `created_at.elapsed() > Duration::from_secs(60)` (closes M2 from SLICE 3 R1)
  - Add `pub fn remove(&self, stream_id: &str) -> Option<StreamState>` callsite for SLICE 4 cleanup (already `#[allow(dead_code)]` in SLICE 3 R1).

## Files touched

| File | Why |
|------|-----|
| `services/envoy_extproc/src/audit.rs` | NEW — StreamState → AppendEventsRequest builder |
| `services/envoy_extproc/src/response_parse.rs` | NEW — provider usage extractor |
| `services/envoy_extproc/src/server.rs` | Response-Headers + Response-Body phase wiring |
| `services/envoy_extproc/src/sidecar_client.rs` | emit_trace_events RPC |
| `services/envoy_extproc/src/state.rs` | 60s TTL sweep + remove() callsite (closes SLICE 3 R1 M2) |
| `services/envoy_extproc/src/lib.rs` | Module registration |
| `services/envoy_extproc/tests/handshake_smoke.rs` | Extend with Response-Body integration tests |

## Test/verification plan

1. `cargo build --manifest-path services/envoy_extproc/Cargo.toml` clean.
2. `cargo test --manifest-path services/envoy_extproc/Cargo.toml` — new tests:
   - `audit::tests::builds_llm_call_post_success_from_allow_outcome`
   - `audit::tests::builds_llm_call_post_run_aborted_from_rejected_outcome`
   - `audit::tests::builds_llm_call_post_run_aborted_from_sidecar_error_outcome`
   - `response_parse::tests::extracts_openai_usage_shape`
   - `response_parse::tests::extracts_anthropic_usage_shape`
   - `response_parse::tests::unknown_provider_returns_tokens_unknown`
   - `state::tests::sweep_removes_entries_older_than_60s`
   - `state::tests::sweep_preserves_entries_under_60s`
   - Integration test: full Request-Headers → Request-Body → Response-Headers → Response-Body → EmitTraceEvents over UDS mock sidecar, asserting the AppendEventsRequest carries reservation_id + LLM_CALL_POST.SUCCESS
   - Integration test: 5xx upstream → LLM_CALL_POST.RUN_ABORTED emit
3. SLICE 1+2+3 regression: all existing tests pass.
4. `cargo fmt --check` + `cargo clippy -D warnings` clean.

## Anti-scope

- No conformance vs Envoy AI Gateway v0.6 reference fixtures — SLICE 5.
- No Helm — SLICE 6.
- No new demo mode — SLICE 7.
- No provider-side calibration loop — out of D01 scope entirely.

## Backlinks

- Spec set: [`design.md`](../../specs/coverage/D01_envoy_extproc/design.md) §4 slice 4 row
- SLICE 3 R1 carry-over: M2 (60s TTL sweep), m3 (Rejected enum variant — already shipped in SLICE 3 R2)
- SLICE 3: [`COV_03_envoy_extproc_budget_query.md`](COV_03_envoy_extproc_budget_query.md)
