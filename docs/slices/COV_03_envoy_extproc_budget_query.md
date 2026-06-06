# COV_03 — D01 Envoy ExtProc: budget query (RequestDecision wire)

> **Deliverable**: D01 Envoy AI Gateway ExtProc sidecar
> **Slice**: 3 of 7 (M)
> **Spec set**: [`docs/specs/coverage/D01_envoy_extproc/`](../specs/coverage/D01_envoy_extproc/)

## Scope

Wire the per-stream sidecar `RequestDecision` RPC: translate the ExtProc Request-Headers + Request-Body state stashed by SLICE 2 into a sidecar adapter `RequestDecision` call, map `DecisionResponse.decision` (ALLOW / DENY / DEGRADE) to ExtProc `CommonResponse` (CONTINUE / immediate_response 429 / immediate_response 403), stash `reservation_id` in the per-stream state so SLICE 4's audit emit can reference it.

Concretely:
- `services/envoy_extproc/Cargo.toml` — add deps:
  - `spendguard-sidecar-adapter-proto` (or equivalent tonic-built sidecar client) — re-use existing proto path
  - `tonic` + `tonic-rustls` already present
- `services/envoy_extproc/src/sidecar_client.rs` — NEW:
  - `pub struct SidecarClient { inner: SidecarAdapterServiceClient<...> }`
  - `pub async fn request_decision(&self, req: RequestDecisionRequest) -> Result<DecisionResponse, ClientError>`
  - UDS+mTLS connection per design §3.3 SLICES 1-5 UDS carve-out
- `services/envoy_extproc/src/decision.rs` — NEW:
  - `pub fn build_request_decision(stream_state: &StreamState, tenant_id: &str) -> RequestDecisionRequest`
  - Maps ClaimEstimate (from SLICE 2 state.rs) → RequestDecisionRequest fields (tenant_id, claim_amount, prompt_class, model_class, etc.)
- `services/envoy_extproc/src/response.rs` — NEW:
  - `pub fn build_extproc_response(decision: DecisionResponse) -> ProcessingResponse`
  - ALLOW → CommonResponse with continue
  - DENY → ImmediateResponse 429 with `x-spendguard-decision: deny` + reason_codes
  - DEGRADE → ImmediateResponse 403 (per fail-closed v1 contract; SLICE 4 might widen to data mutation per spec)
- `services/envoy_extproc/src/server.rs` — extend Request-Body phase:
  - After estimate_tokens_or_warn succeeds: call sidecar_client.request_decision(...), build response, store reservation_id in stream state
  - On failure: fail-closed (return ImmediateResponse 503 + log error)
- `services/envoy_extproc/src/state.rs` — extend StreamState:
  - Add `reservation_id: Option<String>` field
  - Add `decision_outcome: Option<DecisionOutcome>` field (for SLICE 4 audit ref)

## Files touched

| File | Why |
|------|-----|
| `services/envoy_extproc/Cargo.toml` | Add tonic sidecar client deps |
| `services/envoy_extproc/src/sidecar_client.rs` | UDS+mTLS sidecar RPC client |
| `services/envoy_extproc/src/decision.rs` | StreamState → RequestDecisionRequest builder |
| `services/envoy_extproc/src/response.rs` | DecisionResponse → ProcessingResponse mapping |
| `services/envoy_extproc/src/server.rs` | Wire Request-Body → request_decision → response |
| `services/envoy_extproc/src/state.rs` | Extend StreamState with reservation_id |
| `services/envoy_extproc/src/lib.rs` | Module registration |

## Test/verification plan

1. `cargo build --manifest-path services/envoy_extproc/Cargo.toml` clean.
2. `cargo test --manifest-path services/envoy_extproc/Cargo.toml` — new tests:
   - `decision::tests::builds_request_decision_from_claim_estimate`
   - `decision::tests::missing_estimate_fails_closed`
   - `response::tests::allow_maps_to_common_response_continue`
   - `response::tests::deny_maps_to_immediate_response_429_with_reason_codes`
   - `response::tests::degrade_maps_to_immediate_response_403_fail_closed`
   - Integration test: mock sidecar over UDS, real ExtProc Request-Headers + Request-Body flow ending in CONTINUE OR 429
3. SLICE 1 + SLICE 2 regression: all existing tests pass.
4. `cargo fmt --check` clean.

## Anti-scope

- No audit emission — SLICE 4.
- No conformance vs Envoy AI Gateway v0.6 reference fixtures — SLICE 5.
- No Helm — SLICE 6.
- No new demo mode — SLICE 7.

## Backlinks

- Spec set: [`design.md`](../specs/coverage/D01_envoy_extproc/design.md) §4 slice 3 row
- SLICE 1: [`COV_01_envoy_extproc_skeleton.md`](COV_01_envoy_extproc_skeleton.md)
- SLICE 2: [`COV_02_envoy_extproc_token_counter.md`](COV_02_envoy_extproc_token_counter.md)
