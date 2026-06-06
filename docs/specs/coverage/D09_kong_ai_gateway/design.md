# D09 — Kong AI Gateway Plugin — Design

**Status:** Draft for R1 review
**Owner sub-agent (impl):** Backend Architect
**Parent strategy:** [`docs/strategy/framework-coverage-2026-06.md`](../../../strategy/framework-coverage-2026-06.md) §"Should integrate, not compete" + Tier 2 row D09
**Build plan:** [`docs/strategy/framework-coverage-build-plan-2026-06.md`](../../../strategy/framework-coverage-build-plan-2026-06.md) §2.2 D09
**Siblings:** [`D01`](../D01_envoy_extproc/design.md), [`D11`](../D11_litellm_proxy_plugin/design.md)
**Touches:** sidecar adapter v1alpha1 (no proto changes); egress_proxy routing v0.5 (read-only via `spendguard-provider-routing`).

---

## §1. What we're building

`plugins/kong/`: a Kong Gateway plugin in **Go** (`go-pdk`, `.so` plugin-server binary), a Helm sub-chart running a SpendGuard sidecar with a new HTTP companion listener, a reference `KongPlugin` CRD, a Lua-PDK fallback, and a `DEMO_MODE=kong_gateway_real` topology proving a live OpenAI call through Kong lights up the reserve → call → commit lifecycle. Plugin runs in Kong's `access` phase, calls sidecar over **HTTP+mTLS** (not UDS — Kong workers live in a separate pod), short-circuits via `kong.response.exit(429)` on DENY.

## §2. Why this slot, why now

- **Commercial counterweight to Envoy AI Gateway.** Kong Enterprise + OSS Kong 3.6+ ship the `ai-proxy` / `ai-prompt-guard` / `ai-rate-limiting-advanced` plugin family. SpendGuard absence is the same discoverability gap D01/D11 close on adjacent surfaces.
- **Go-PDK over Lua-PDK.** `go-pdk` v0.11 is stable; Go is faster than LuaJIT FFI, the dep surface is healthier (Go modules vs LuaRocks), and the binary distribution model matches our existing Rust/Helm packaging. Lua ships as fallback, labeled experimental.
- **Translation layer, not re-implementation.** Identical to D01: plugin is a *client* of the sidecar adapter. No new decision engine, ledger, or tokenizer. Token counting reuses `spendguard-tokenizer`; because Kong runs out-of-process from the Rust sidecar, the Go plugin calls a sidecar **HTTP companion** endpoint (`/v1/tokenize`, `/v1/decision`, `/v1/trace`) added as SLICE 1 — thin wrappers over existing handlers, reusable by D31 (Coze) and D32 (Botpress).

## §3. Key architectural decisions

### 3.1 HTTP+mTLS transport, not UDS

Kong DataPlane pods own a separate network namespace; SO_PEERCRED is unavailable. Mirror D01 §3.3: mTLS over TCP, sidecar on loopback `127.0.0.1:8443` when sharing the Kong pod's net-ns, or SVID-mTLS pod-network port when sidecar is a sibling pod. HARDEN_08 per-tenant SVID minting carries over; the Kong plugin container holds the workload SVID for its tenant.

### 3.2 Go plugin primary, Lua fallback

`plugins/kong/spendguard-go/` is the supported distribution. `plugins/kong/spendguard-lua/` covers `access` + `body_filter` only via `lua-resty-http` against the same companion endpoints. Lua does not get the conformance-test guarantee.

### 3.3 Reserve in `access`, commit in `body_filter`

`access` runs after Kong buffers the body (we require `request_buffering: true`): Go plugin parses body → `/v1/tokenize` → `/v1/decision` → `kong.response.exit(429)` on DENY, store `reservation_id` in `kong.ctx.shared` on ALLOW. `body_filter` accumulates upstream response, then calls `/v1/trace` with `LLM_CALL_POST.SUCCESS` (provider-reported usage) or `RUN_ABORTED` on upstream 5xx. No streaming SSE budget enforcement in v1; commit at end-of-body, matching D01 §3.5.

### 3.4 Deny-on-fail-closed default

Sidecar unreachable → `kong.response.exit(503)` + `Retry-After`. Operators flip explicit `fail_open: true` plugin-config flag (audit-logged on startup) to degrade-to-allow. Default closed.

### 3.5 Provider scope

v1 covers OpenAI-shaped (`/v1/chat/completions`) and Anthropic-shaped (`/v1/messages`) payloads, matching `ai-proxy` providers `openai` + `anthropic`. Bedrock / Cohere / Mistral routing reuses `spendguard-provider-routing` from D01 SLICE 1; plugin maps `kong.request.get_path()` + `ai-proxy` `route_type` to a `ProviderKind`. No Bedrock SigV4 mutation in v1.

## §4. Slice plan (7 slices)

| # | Name | Size | Scope |
|---|------|------|-------|
| 1 | `COV_D09_01_sidecar_http_companion` | M | Sidecar HTTP/1.1+mTLS axum listener exposing `/v1/tokenize` + `/v1/decision` + `/v1/trace`; thin wrappers over existing handlers; loopback-only by default. |
| 2 | `COV_D09_02_kong_plugin_scaffold` | M | `plugins/kong/spendguard-go/` Go-PDK scaffold; `main.go` registers `spendguard` with empty `Access` + `BodyFilter`; `make build-kong-plugin` produces `.so`. |
| 3 | `COV_D09_03_kong_access_reserve` | M | `Access` parses body → tokenize → decision; DENY → 429; ALLOW → reservation_id in ctx; DEGRADE → fail-closed unless `fail_open=true`. |
| 4 | `COV_D09_04_kong_body_filter_commit` | M | `BodyFilter` accumulates response, parses provider usage, calls `/v1/trace` with `LLM_CALL_POST.SUCCESS` or `RUN_ABORTED`; idempotent on reservation_id. |
| 5 | `COV_D09_05_kong_lua_fallback` | S | `plugins/kong/spendguard-lua/` parity plugin against same HTTP companion contract; documented experimental. |
| 6 | `COV_D09_06_kong_helm_chart` | M | `charts/spendguard/templates/kong_plugin_sidecar.yaml`; reference `KongPlugin` CRDs in `examples/kong-gateway-composite/`; ServiceMonitor + NetworkPolicy. |
| 7 | `COV_D09_07_kong_demo` | M | `DEMO_MODE=kong_gateway_real`; `compose.kong.yaml` boots Kong + SpendGuard + real OpenAI; `verify_step_kong_gateway_real.sql`; `docs/site/docs/integrations/kong-ai-gateway.md`. |

## §5. Anti-scope

- No Kong control-plane plugin-registry distribution; install via `KongPlugin` CRD or `kong.conf`.
- No upstream PR to Kong/kong; plugin lives in our repo.
- No streaming SSE budget enforcement in v1.
- No co-install validation with `ai-rate-limiting-advanced`.
- No Kong Konnect (SaaS control plane) integration.
- No customer plugin contract Strategy C in v1; Strategy A reservation is sufficient.

## §6. Out-of-band coordination

D09 depends on D01 SLICE 1 extracting `crates/spendguard-provider-routing`. If D09 starts before that lands, D09 SLICE 1 absorbs the extraction. The HTTP companion is reusable by D31 + D32; landing under D09 SLICE 1, referenced from those specs.

---

*Locked decisions: §3.1, §3.2, §3.3, §3.4, §3.5. Slice plan: §4 (7 slices). Anti-scope: §5.*
