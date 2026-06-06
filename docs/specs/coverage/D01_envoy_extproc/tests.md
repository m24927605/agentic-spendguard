# D01 — Envoy AI Gateway ExtProc Sidecar — Tests

**Companion to:** [`design.md`](design.md) + [`implementation.md`](implementation.md)
**Test runner:** `cargo test -p spendguard-envoy-extproc -p spendguard-provider-routing` plus `make demo-up DEMO_MODE=envoy_extproc`

---

## §1. Unit tests

### §1.1 `crates/spendguard-provider-routing/src/lib.rs` (extracted in SLICE 1)

All existing tests at [`services/egress_proxy/src/routing.rs:351+`](../../../../services/egress_proxy/src/routing.rs) move with the code. The shared crate must keep them green byte-identically:

| Test | Intent |
|------|--------|
| `routes_openai_chat_completions` | OpenAI Chat Completions path resolves with `EncoderKind::OpenAi` |
| `routes_openai_responses` | OpenAI Responses API path resolves |
| `routes_anthropic_messages` | Anthropic Messages API path resolves with `EncoderKind::Anthropic` |
| `routes_bedrock_invoke` | Bedrock InvokeModel URL captures model id into `{0}` |
| `routes_vertex_generate_content` | Vertex captures project + location + model |
| `routes_azure_openai_chat_completions` | Azure deployment-id captured into `{0}` |
| `unknown_path_returns_none` | Unknown inbound path returns `None` (404 in caller) |
| `resolve_model_id_bedrock_from_url` | Bedrock model resolved from path, not body |
| `resolve_tokenizer_kind_bedrock_dispatches_per_vendor` | Bedrock anthropic / cohere / llama dispatched to right encoder |

### §1.2 `services/envoy_extproc/src/translate/request_phase.rs` (SLICE 2)

| Test | Intent |
|------|--------|
| `parse_openai_chat_completions_extracts_input_tokens` | OpenAI chat body → `input_tokens > 0` via Tier 2 encoder |
| `parse_anthropic_messages_extracts_input_tokens` | Anthropic body → `input_tokens > 0` via Anthropic encoder |
| `parse_bedrock_invoke_uses_model_from_path` | Bedrock model id resolved from URL path, not body |
| `parse_unknown_path_returns_error` | Unknown path returns `anyhow::Error`, caller maps to 400 |
| `parse_malformed_json_returns_error` | Invalid JSON returns `anyhow::Error`, caller maps to 400 |
| `parse_t3_fallback_emits_zero_tokens` | Unknown Bedrock model → `tokenizer_kind = None`, `input_tokens = 0`, `tokenizer_tier = "T3"` |
| `build_claim_estimate_sets_strategy_a_only` | Strategy B/C = 0; reserved_strategy = "A"; policy = "STRICT_CEILING" |
| `build_claim_estimate_estimates_a_as_2x_input` | `predicted_a_tokens == input_tokens * 2` |

### §1.3 `services/envoy_extproc/src/translate/decision_map.rs` (SLICE 3)

| Test | Intent |
|------|--------|
| `decision_continue_maps_to_extproc_continue` | `Decision::Continue` → ProcessingResponse with CommonResponse, no immediate_response |
| `decision_stop_maps_to_immediate_429` | `Decision::Stop` → HTTP 429 with reason_codes in body |
| `decision_stop_run_projection_maps_to_immediate_429` | `STOP_RUN_PROJECTION` → 429 with `run_code_triggered` surfaced |
| `decision_require_approval_maps_to_immediate_403` | `RequireApproval` → 403 with approval_request_id header |
| `decision_degrade_emits_mutation_patch` | `Degrade` → headers + body mutation per RFC 6902 patch |
| `decision_unspecified_fails_closed_with_503` | Sentinel unspecified → 503 (defense in depth) |

### §1.4 `services/envoy_extproc/src/translate/response_phase.rs` (SLICE 4)

| Test | Intent |
|------|--------|
| `response_openai_extracts_usage_from_body` | OpenAI response → `input_tokens + output_tokens > 0` |
| `response_anthropic_extracts_usage_from_body` | Anthropic response → usage including `cache_creation_input_tokens` |
| `response_bedrock_extracts_usage_from_body` | Bedrock per-vendor response shapes parsed |
| `response_truncated_body_returns_none` | Partial JSON → `None`, caller emits `RUN_ABORTED` |
| `response_5xx_status_emits_run_aborted` | Upstream 5xx → `LLM_CALL_POST.RUN_ABORTED` event |
| `response_missing_usage_falls_to_estimated` | Response without usage block → uses cached input × 2 estimate per HARDEN_03 pattern |

### §1.5 `services/envoy_extproc/src/stream.rs` (SLICE 3 + 4)

| Test | Intent |
|------|--------|
| `stream_state_threads_session_through_phases` | Headers → Body → ResponseHeaders → ResponseBody preserves session_id |
| `stream_state_remembers_reservation_id` | `bind_decision` stores reservation_id for response-phase commit |
| `stream_state_session_lost_on_drop_emits_run_aborted` | Stream drop before ResponseBody → background task emits `LLM_CALL_POST.RUN_ABORTED` |
| `stream_state_handles_out_of_order_phases` | Body before Headers → returns error, ExtProc spec requires ordered phases |

### §1.6 `services/envoy_extproc/src/sidecar_client.rs` (SLICE 1 + 3)

| Test | Intent |
|------|--------|
| `client_handshake_succeeds_against_loopback_server` | Loopback gRPC server returns valid `HandshakeResponse` |
| `client_handshake_fails_on_tenant_mismatch` | Mismatched `tenant_id` → `PermissionDenied`, process exits 1 |
| `client_request_decision_propagates_timeout` | `request_timeout_ms` enforced; sidecar slow → caller deny_503 |
| `client_reconnects_on_transport_error` | Sidecar restart → client re-establishes mTLS without losing in-flight streams |

### §1.7 `services/envoy_extproc/src/tls.rs` (SLICE 1)

| Test | Intent |
|------|--------|
| `tls_loads_svid_pem_files` | Reads `tls.crt` + `tls.key` + `ca.crt` from configured paths |
| `tls_rejects_missing_cert` | Missing cert path → typed error at startup, no insecure fallback |
| `tls_pins_sidecar_spiffe_uri` | Sidecar cert SPIFFE URI SAN matches expected `spiffe://*/sidecar` pattern (mirrors HARDEN_08) |

## §2. Integration tests

### §2.1 `tests/conformance/` (SLICE 5)

| Test | Intent |
|------|--------|
| `conformance_envoy_v06_token_counting_yaml` | Replays the Envoy AI Gateway v0.6 `token_counting.yaml` reference manifest's ExtProc stream against MockSidecar; byte-equals golden response |
| `conformance_envoy_v06_budget_yaml` | Same for `budget.yaml` reference manifest; verifies CONTINUE / 429 / 403 mapping matches Envoy AI Gateway docs |
| `conformance_envoy_v06_chat_completions_streaming` | Streaming chunk handling (chunks arrive as separate `ProcessingRequest::ResponseBody` frames); end-of-stream emits a single `LLM_CALL_POST` per spec §3.5 |
| `conformance_envoy_v06_messages_anthropic` | Anthropic Messages golden fixture |

### §2.2 `tests/translate_request_phase.rs` (SLICE 2 + 3)

| Test | Intent |
|------|--------|
| `integration_extproc_request_to_sidecar_request_decision` | Full ExtProc Request-Body phase invokes `MockSidecar::request_decision` exactly once with correct ClaimEstimate |
| `integration_unknown_path_emits_400` | Unknown path → ExtProc immediate_response 400; sidecar never called |
| `integration_sidecar_unreachable_emits_503_with_retry_after` | Sidecar mTLS handshake failure → 503 + Retry-After: 1s |

### §2.3 `tests/translate_response_phase.rs` (SLICE 4)

| Test | Intent |
|------|--------|
| `integration_response_body_emits_llm_call_post_success` | Full upstream 200 + usage → `EmitTraceEvents` stream contains exactly one `LLM_CALL_POST.SUCCESS` |
| `integration_response_5xx_emits_run_aborted_release` | Upstream 500 → `LLM_CALL_POST.RUN_ABORTED` → sidecar invokes release lane (verified via mock counter) |
| `integration_stream_drop_emits_run_aborted` | Client disconnects mid-stream → `RUN_ABORTED` emitted within 100ms |

### §2.4 `tests/stream_lifecycle.rs` (SLICE 3)

| Test | Intent |
|------|--------|
| `lifecycle_drain_signal_stops_new_streams` | When sidecar advertises drain phase, ExtProc returns 503 on new ProcessingRequest streams (mirrors `services/sidecar/src/server/adapter_uds.rs:135-144`) |
| `lifecycle_metrics_increment_per_handler` | `/metrics` exposes `envoy_extproc_handler_total{handler,outcome}` per phase |
| `lifecycle_session_id_propagated_to_audit` | Generated session_id appears in MockSidecar's recorded audit event |

### §2.5 `tests/tls_loopback.rs` (SLICE 1)

| Test | Intent |
|------|--------|
| `tls_full_handshake_e2e` | Real rustls server + client; mTLS completes; handshake RPC returns |
| `tls_cert_pinning_rejects_wrong_spiffe_id` | Sidecar presents a cert with SPIFFE URI `spiffe://other-tenant/sidecar` → client refuses (matches HARDEN_08 invariant) |

## §3. Demo regression tests

### §3.1 `make demo-up DEMO_MODE=envoy_extproc` (SLICE 7)

| Check | Intent |
|-------|--------|
| All containers reach Ready | postgres / sidecar / envoy-extproc / envoy-proxy / mock-upstream all healthy within 60s |
| Demo client emits 5 chat requests | Mock client sends 5 OpenAI chat completions through Envoy |
| `verify_step_envoy_extproc.sql` passes | Exactly 5 `audit_decision` rows + 5 matching `audit_outcome` rows, all with `runtime_kind = 'envoy-ai-gateway'` |
| `verify-chain` passes | Audit chain signature verification across all 10 new rows |
| Pod logs contain no error level lines | Clean run; `envoy_extproc` emits structured INFO only |

### §3.2 Existing demo modes still pass (regression)

| Demo mode | Intent |
|-----------|--------|
| `DEMO_MODE=decision` | Sidecar UDS adapter path unaffected by SLICE 1 routing-table extraction |
| `DEMO_MODE=proxy` | egress_proxy still routes correctly after `routing.rs` becomes a re-export from the shared crate |
| `DEMO_MODE=multi_provider_usd` | SLICE_11 multi-provider routing-table tests still pass (no behavior change) |
| `DEMO_MODE=approval` | REQUIRE_APPROVAL path still resumes; ExtProc deliverable does not touch the resume flow |

## §4. Benchmark + load tests

### §4.1 `benchmarks/envoy_extproc/Cargo.toml` (SLICE 5)

| Benchmark | Target |
|-----------|--------|
| `bench_token_counting_openai_p99` | < 1ms p99 for OpenAI chat body of 4 KB (Tier 2 budget per `predictor-review-checklist.md` §1.2) |
| `bench_token_counting_anthropic_p99` | < 1ms p99 for Anthropic body of 4 KB |
| `bench_full_extproc_roundtrip_p99` | < 50ms p99 for full Request-Headers → Request-Body → Decision → Response-Body → Audit roundtrip including mocked sidecar (Contract §14 budget) |

### §4.2 Load harness (SLICE 7)

| Load profile | Intent |
|--------------|--------|
| 100 RPS for 60s | No 5xx; p95 < 100ms; reservation_id never reused |
| 1000 concurrent streams | No OOM; in-memory stream state map bounded; old streams expire after 60s |

## §5. Negative / chaos tests

| Test | Intent |
|------|--------|
| `chaos_sidecar_killed_mid_stream` | SIGKILL sidecar → `envoy_extproc` returns 503 with Retry-After; no audit half-write |
| `chaos_envoy_drops_stream_mid_body` | Envoy disconnects between Request-Body and Response-Body → background task emits `RUN_ABORTED` within 100ms |
| `chaos_malformed_protobuf` | Garbage bytes on the wire → tonic rejects, sidecar never invoked |
| `chaos_oversized_request_body` | Body > 4 MiB cap → 413; matches POST_GA_03 ingress hardening pattern |
| `chaos_clock_skew` | Server clock 5min ahead → SVID still validates (rustls 5min tolerance); no spurious denies |

## §6. Coverage matrix

| Layer | Unit | Integration | Demo | Bench |
|-------|------|-------------|------|-------|
| `provider-routing` | ✅ §1.1 | ✅ §2.2 | ✅ §3.1 | — |
| `request_phase` | ✅ §1.2 | ✅ §2.2 | ✅ §3.1 | ✅ §4.1 |
| `decision_map` | ✅ §1.3 | ✅ §2.2 | ✅ §3.1 | — |
| `response_phase` | ✅ §1.4 | ✅ §2.3 | ✅ §3.1 | ✅ §4.1 |
| `stream` | ✅ §1.5 | ✅ §2.4 | ✅ §3.1 | ✅ §4.2 |
| `sidecar_client` | ✅ §1.6 | ✅ §2.5 | ✅ §3.1 | — |
| `tls` | ✅ §1.7 | ✅ §2.5 | ✅ §3.1 | — |
| Helm chart | — | — | ✅ §3.1 | — |
