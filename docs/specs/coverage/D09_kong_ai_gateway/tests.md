# D09 — Kong AI Gateway Plugin — Tests

**Companion to:** [`design.md`](design.md), [`implementation.md`](implementation.md)

---

## §1. Test taxonomy

| Layer | Framework | Where | Slice |
|-------|-----------|-------|-------|
| Sidecar HTTP companion unit | `cargo test` (axum::body + tower::ServiceExt) | `services/sidecar/src/server/http_companion.rs` `#[cfg(test)]` | SLICE 1 |
| Sidecar HTTP companion integration | `cargo test` + `reqwest` against bound port | `services/sidecar/tests/http_companion_integration.rs` | SLICE 1 |
| Go plugin unit | `go test ./...` | `plugins/kong/spendguard-go/*_test.go` | SLICE 2-4 |
| Go plugin integration | `kong-pongo` (Kong's official plugin test harness, docker-compose) | `plugins/kong/spendguard-go/spec/` | SLICE 3, 4 |
| Lua plugin integration | `kong-pongo` | `plugins/kong/spendguard-lua/spec/` | SLICE 5 |
| Helm chart unit | `helm template` + `yq` assertions | `charts/spendguard/tests/kong_plugin_test.sh` | SLICE 6 |
| Helm chart kind smoke | `kind create cluster` + `kubectl apply` | `charts/spendguard/tests/kong_plugin_kind.sh` | SLICE 6 |
| Demo-mode E2E | `deploy/demo/Makefile` + Postgres SQL verifier | `deploy/demo/verify_step_kong_gateway_real.sql` | SLICE 7 |
| Audit chain regression | Existing `services/sidecar/tests/audit_chain_invariants.rs` extended | (existing file) | SLICE 7 |

## §2. Per-slice test plan

### SLICE 1 — sidecar HTTP companion

**Unit tests** (`services/sidecar/src/server/http_companion.rs`):

- `tokenize_handler_returns_input_tokens_for_openai_chat` — POSTs `{provider:"openai_chat", body:"<json>"}`, asserts `input_tokens` matches `spendguard_tokenizer::encode("openai_chat", body).len()`.
- `tokenize_handler_4mib_cap` — POSTs 5 MiB body, expects 413.
- `decision_handler_allow_path` — wires a `SidecarState` with budget headroom, expects `{"decision":"ALLOW", "reservation_id":<uuid>}`.
- `decision_handler_deny_path` — exhausts budget; expects `{"decision":"DENY"}`.
- `decision_handler_degrade_path` — kills ledger DB pool, expects `{"decision":"DEGRADE"}`.
- `trace_handler_emits_single_event` — POSTs `LLM_CALL_POST.SUCCESS` with `reservation_id`, asserts `audit_outbox` row written via shared decision::transaction path.
- `mtls_required` — non-mTLS plain HTTPS connection rejected at TLS handshake.
- `svid_san_validates_tenant` — cert with mismatched SAN URI rejected at TLS handshake.

**Integration test** (`services/sidecar/tests/http_companion_integration.rs`):

- Boots the HTTP companion on an ephemeral port with a test SVID, fires 100 concurrent `/v1/decision` requests via `reqwest::Client`, asserts no audit-chain violation, asserts each `reservation_id` is unique, asserts /metrics increments correctly.

**Negative regression**:

- `loopback_only_by_default` — without `--allow-pod-network` flag, binding 0.0.0.0 should refuse to start.

### SLICE 2 — Go plugin scaffold

**Unit tests** (`plugins/kong/spendguard-go/main_test.go`):

- `TestConfigDefaults_TimeoutMS_500` — `New()` returns `Config{TimeoutMS: 500}`.
- `TestPluginRegistersAccessAndBodyFilter` — uses `go-pdk` test harness to confirm both phase functions are registered.
- `TestBuildProducesSO_ELF` — CI gate that `go build ./...` produces a Linux ELF binary executable by Kong's plugin-server.

### SLICE 3 — access reserve flow

**Unit tests** (`plugins/kong/spendguard-go/access_test.go`):

- `TestAccess_OpenAI_AllowPath` — fake sidecar returns ALLOW; assert `kong.Ctx.SetShared("spendguard_reservation_id", _)` was called; assert no `Response.Exit`.
- `TestAccess_OpenAI_DenyPath` — fake sidecar returns DENY; assert `kong.Response.Exit(429, ...)` was called with `SPENDGUARD_DENY` body.
- `TestAccess_DegradeFailClosed` — fake sidecar returns DEGRADE, `fail_open=false`; assert `Response.Exit(503)`.
- `TestAccess_DegradeFailOpen` — DEGRADE + `fail_open=true`; assert `Response.Exit` NOT called; assert log warning emitted.
- `TestAccess_AnthropicMessagesShape` — request body matches `/v1/messages` shape; provider detected as `anthropic_messages`; tokenize called with that key.
- `TestAccess_UnknownProviderShape` — body is neither OpenAI nor Anthropic; `fail_open=false` → 400; `fail_open=true` → log and continue.
- `TestAccess_SidecarUnreachable_FailClosed` — sidecar HTTP client returns connection refused; assert 503.
- `TestAccess_TimeoutEnforced` — sidecar stub sleeps 1000ms with `TimeoutMS=500`; assert deadline triggers DEGRADE path.

**Integration test (kong-pongo)** (`plugins/kong/spendguard-go/spec/access_spec.lua` — pongo wraps Go plugins too):

- `[D09-ACCESS-01] real Kong + fake sidecar returns 429 on DENY` — boots Kong with declarative config, fires `curl POST /v1/chat/completions`, fake sidecar configured to DENY, asserts curl sees 429 + `SPENDGUARD_DENY`.
- `[D09-ACCESS-02] real Kong + fake sidecar returns 200 on ALLOW` — same but ALLOW; asserts upstream stub got the request.

### SLICE 4 — body_filter commit flow

**Unit tests** (`plugins/kong/spendguard-go/body_filter_test.go`):

- `TestBodyFilter_OpenAI_UsageParsed` — feeds `{"usage":{"prompt_tokens":42,"completion_tokens":17,"total_tokens":59}}` final chunk; assert `client.Trace(reservationID, "LLM_CALL_POST.SUCCESS", &{42,17})`.
- `TestBodyFilter_Anthropic_UsageParsed` — feeds `{"usage":{"input_tokens":42,"output_tokens":17}}`; same trace shape.
- `TestBodyFilter_MalformedJSON_EmitsRunAborted` — final chunk is truncated JSON; assert `Trace("RUN_ABORTED", nil)`.
- `TestBodyFilter_ChunkedAccumulation` — feeds 3 partial chunks then final; assert single `Trace` call at end-of-body.
- `TestBodyFilter_NoReservationIdSkips` — `kong.Ctx.GetSharedString("spendguard_reservation_id")` returns empty; assert no sidecar call.
- `TestBodyFilter_IdempotentOnDuplicate` — calling twice with same `reservation_id`; assert sidecar `Trace` only fired once (plugin-side dedup via ctx flag).

**Integration test (kong-pongo)**:

- `[D09-COMMIT-01] ALLOW → upstream 200 → commit fires` — boots full topology with stub upstream returning canned OpenAI usage block; verifies sidecar receives `LLM_CALL_POST.SUCCESS` with matching `input_tokens`/`output_tokens`.
- `[D09-COMMIT-02] ALLOW → upstream 500 → RUN_ABORTED` — stub upstream returns 500; verifies sidecar receives `RUN_ABORTED`.

### SLICE 5 — Lua fallback

**Integration test (kong-pongo)** (`plugins/kong/spendguard-lua/spec/`):

- `[D09-LUA-01] access ALLOW path` — same as `[D09-ACCESS-02]` but with Lua plugin loaded instead of Go.
- `[D09-LUA-02] access DENY path` — same as `[D09-ACCESS-01]`.
- `[D09-LUA-03] body_filter commit` — same as `[D09-COMMIT-01]`.
- `[D09-LUA-04] schema validation` — `kong.conf` with `sidecar_url` missing rejected.

**Anti-regression**: Lua plugin tests are gated behind `KONG_LUA_TESTS=1` CI flag; the docs page declares Lua "experimental" so the test surface is intentionally smaller than the Go path.

### SLICE 6 — Helm chart

**Helm template tests** (`charts/spendguard/tests/kong_plugin_test.sh`):

- `helm template . --set kongPlugin.enabled=true` succeeds; output contains `kind: Deployment` named `*-kong-companion`.
- ServiceMonitor rendered when `kongPlugin.monitoring.serviceMonitor.enabled=true`.
- NetworkPolicy rendered with `podSelector: app.kubernetes.io/name=kong`.
- SVID volume mount present when `kongPlugin.svidIssuer` is set; rejected when unset (render-time fail-closed gate).
- `--set kongPlugin.enabled=false` produces no kong-companion resources.

**Kind smoke test** (`charts/spendguard/tests/kong_plugin_kind.sh`):

- `kind create cluster --name d09-test`.
- `helm install spendguard ./charts/spendguard --set kongPlugin.enabled=true --set image.tag=dev`.
- Apply Kong Ingress Controller manifests + reference `KongPlugin` CRD.
- Wait for `kong-companion` pod Ready; assert `/v1/tokenize` reachable from a debug pod with the right SVID.
- Tear down.

### SLICE 7 — Demo + audit chain

**Demo gate** (`deploy/demo/verify_step_kong_gateway_real.sql`):

```sql
-- ASSERTIONS for DEMO_MODE=kong_gateway_real
\set ON_ERROR_STOP 1

-- ALLOW path: exactly one PRE_LLM_CALL.RESERVE then one LLM_CALL_POST.SUCCESS with matching reservation_id
SELECT count(*) = 1 AS allow_reserve_present
FROM audit_outbox
WHERE event_type = 'PRE_LLM_CALL.RESERVE'
  AND request_metadata->>'demo_mode' = 'kong_gateway_real'
  AND request_metadata->>'phase' = 'allow';

SELECT count(*) = 1 AS allow_commit_present
FROM audit_outbox
WHERE event_type = 'LLM_CALL_POST.SUCCESS'
  AND request_metadata->>'demo_mode' = 'kong_gateway_real'
  AND request_metadata->>'phase' = 'allow';

-- DENY path: exactly one PRE_LLM_CALL.RESERVE with decision=DENY, no LLM_CALL_POST.* follow-up
SELECT count(*) = 1 AS deny_reserve_present
FROM audit_outbox
WHERE event_type = 'PRE_LLM_CALL.RESERVE'
  AND request_metadata->>'demo_mode' = 'kong_gateway_real'
  AND request_metadata->>'phase' = 'deny'
  AND request_metadata->>'decision' = 'DENY';

SELECT count(*) = 0 AS deny_has_no_commit
FROM audit_outbox
WHERE event_type LIKE 'LLM_CALL_POST.%'
  AND request_metadata->>'demo_mode' = 'kong_gateway_real'
  AND request_metadata->>'phase' = 'deny';

-- Audit chain hash continuity
SELECT spendguard_verify_chain('kong_gateway_real') AS chain_intact;
```

**Audit-chain regression** (`services/sidecar/tests/audit_chain_invariants.rs`): extend the existing invariants suite with a `kong_companion_via_http` fixture that exercises `/v1/decision` and `/v1/trace` and confirms the resulting audit row hashes match the canonical chain.

## §3. Coverage targets

| Area | Target |
|------|--------|
| Go plugin line coverage | ≥ 85% (excluding `main.go` and generated proto stubs) |
| Sidecar HTTP companion line coverage | ≥ 90% |
| Lua plugin line coverage | not gated (experimental tier) |
| kong-pongo integration scenarios | 4 Go + 4 Lua, all named `[D09-*]` |
| Demo-mode SQL verifier | all 5 assertions PASS |

## §4. CI wiring

```yaml
# .github/workflows/kong-plugin.yml
name: kong-plugin
on:
  pull_request:
    paths: ["plugins/kong/**", "services/sidecar/src/server/http_companion.rs",
            "charts/spendguard/templates/kong_plugin_*", "deploy/demo/compose.kong.yaml"]
jobs:
  go-plugin-unit:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-go@v5
        with: { go-version: '1.22' }
      - run: cd plugins/kong/spendguard-go && go test -race -cover ./...
  kong-pongo-go:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cd plugins/kong/spendguard-go && pongo run
  kong-pongo-lua:
    runs-on: ubuntu-latest
    if: ${{ contains(github.event.pull_request.labels.*.name, 'kong-lua') }}
    steps:
      - uses: actions/checkout@v4
      - run: cd plugins/kong/spendguard-lua && pongo run
  sidecar-http-companion:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo test -p spendguard-sidecar http_companion
  helm-chart:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: bash charts/spendguard/tests/kong_plugin_test.sh
```

## §5. Manual verification

Documented in `docs/site/docs/integrations/kong-ai-gateway.md` (SLICE 7): the 10-line `curl` recipe that any reviewer can copy-paste against a freshly booted `make demo-up DEMO_MODE=kong_gateway_real` topology to see ALLOW + DENY + COMMIT lifecycle without reading SQL.
