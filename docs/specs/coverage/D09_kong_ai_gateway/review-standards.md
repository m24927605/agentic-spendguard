# D09 — Kong AI Gateway Plugin — Review Standards

**Companion to:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`acceptance.md`](acceptance.md)
**Reviewer:** `superpowers:code-reviewer` (R1-R5), Staff+ panel on R5 failure per [`framework-coverage-build-plan-2026-06.md`](../../../strategy/framework-coverage-build-plan-2026-06.md) §1.

This document is the canonical R1-R5 checklist for **every slice of D09**. The reviewer runs through every applicable section against the slice diff. A finding is any "BLOCK" or "FIX-BEFORE-MERGE" item below; "NIT" findings are tracked as residual GH issues, not blockers.

---

## §1. Architectural invariants (BLOCK — applies to every slice)

| # | Invariant | How to verify |
|---|-----------|---------------|
| 1.1 | Plugin is a *translation layer*, not a re-implementation. No new decision engine, ledger, tokenizer, or audit chain logic appears anywhere in `plugins/kong/` or `services/sidecar/src/server/http_companion.rs`. | `grep -rE "(audit_outbox\|reservation_id_seq\|ledger\|tokenizer::encode_impl)" plugins/kong/ services/sidecar/src/server/http_companion.rs` — every hit must be a *call* into existing code, never a re-implementation. |
| 1.2 | All audit writes go through `decision::transaction::run` / `run_commit_estimated`. No direct `INSERT INTO audit_outbox` from the HTTP companion or the Kong plugin. | `git grep "INSERT INTO audit_outbox" plugins/kong/ services/sidecar/src/server/http_companion.rs` must be empty. |
| 1.3 | Adapter v1alpha1 proto unchanged. | `git diff main -- 'services/sidecar/proto/**'` must be empty across every D09 slice. |
| 1.4 | Sidecar HTTP companion is mTLS-only; loopback bind by default. | inspect `http_companion.rs::bind_listener`: no plain HTTP path, `0.0.0.0` bind requires explicit `--allow-pod-network`. |
| 1.5 | Provider routing reused, not forked. Body-shape detection in `provider_route.go` calls `spendguard-provider-routing` via the HTTP companion's `/v1/tokenize` `provider` field, never re-implements `resolve_model_id` in Go. | `grep -E "model_id|provider_kind" plugins/kong/spendguard-go/` shows only string passing, no resolution logic. |
| 1.6 | Fail-closed default. `Config{FailOpen: false}` is the constructor default; flipping the flag emits a startup log warning. | inspect `config.go::New()` and the startup log. |
| 1.7 | No `kong.response.exit` skipping reservation. Every code path that hits an upstream LLM endpoint must first have a successful `client.Decision(...)` ALLOW response. | walk `access.go` execution graph; verify no early-return ALLOW path bypassing sidecar. |

## §2. Slice-1 specific (sidecar HTTP companion)

| # | Item | Severity |
|---|------|----------|
| 2.1 | Handler bodies are *thin wrappers* — < 50 LOC each — delegating to the same internal handlers used by the gRPC adapter. No copy-pasted business logic. | BLOCK |
| 2.2 | mTLS cert chain validation uses the same `rustls::ServerConfig` + custom `ClientCertVerifier` as the gRPC adapter (SLICE_07 pattern from HARDEN_08). No custom verifier. | BLOCK |
| 2.3 | SVID SAN URI matched against the configured tenant before any business logic runs. Mismatch → 403, no audit row. | BLOCK |
| 2.4 | Body-size cap (4 MiB) enforced at axum extractor level, not deep in handler. | BLOCK |
| 2.5 | `POST /v1/tokenize` returns the same token count `spendguard_tokenizer::encode` produces in-process; differential test required. | BLOCK |
| 2.6 | `POST /v1/decision` is idempotent on `request_fingerprint`; replays return the same `reservation_id` (mirrors POST_GA_01 fingerprint-cache semantics). | BLOCK |
| 2.7 | Metrics surface: `spendguard_http_companion_requests_total{handler,outcome}` + `spendguard_http_companion_latency_seconds_bucket{handler}`. | FIX-BEFORE-MERGE |
| 2.8 | Loopback-only by default: default config binds `127.0.0.1`, `0.0.0.0` requires explicit flag + emits a "pod-network exposure enabled" startup log. | BLOCK |

## §3. Slice-2 specific (Go plugin scaffold)

| # | Item | Severity |
|---|------|----------|
| 3.1 | `go.mod` minimum version `1.22`. | FIX-BEFORE-MERGE |
| 3.2 | `go-pdk` pinned to a tagged release ≥ `v0.11.0`. No `master` floating ref. | BLOCK |
| 3.3 | Build artifact is a single static binary; no CGO unless documented. | BLOCK |
| 3.4 | `New()` returns sensible defaults; `TimeoutMS: 500` not zero. | FIX-BEFORE-MERGE |
| 3.5 | Plugin registers in both `Access` and `BodyFilter` phases. | BLOCK |
| 3.6 | `main.go` calls `server.StartServer(New, "1.0.0", 0)` with explicit version. | FIX-BEFORE-MERGE |

## §4. Slice-3 specific (access reserve)

| # | Item | Severity |
|---|------|----------|
| 4.1 | Request body parsed exactly once and the parsed token count threaded to `Decision`. No double-parse. | BLOCK |
| 4.2 | DENY response body is JSON, not plain text, and contains the literal string `SPENDGUARD_DENY` for grep-ability. | BLOCK |
| 4.3 | `reservation_id` stored in `kong.ctx.shared` with a stable key name `spendguard_reservation_id`. Documented in code comment. | BLOCK |
| 4.4 | DEGRADE path differentiates client-config (`fail_open`) from operator-config; logged with both. | FIX-BEFORE-MERGE |
| 4.5 | Sidecar HTTP client honors `TimeoutMS`; timeout treated as DEGRADE not Err. | BLOCK |
| 4.6 | Provider detection failure → 400 (client error), not 503 (server error). | FIX-BEFORE-MERGE |
| 4.7 | All exits use `kong.response.exit(status, body, headers)` — no direct `ngx.exit`. | BLOCK |
| 4.8 | `Idempotency-Key` header propagated to sidecar `Decision.request_fingerprint`. | BLOCK |

## §5. Slice-4 specific (body_filter commit)

| # | Item | Severity |
|---|------|----------|
| 5.1 | `body_filter` accumulates chunks; trace fires exactly once at end-of-body. Verified by `TestBodyFilter_ChunkedAccumulation`. | BLOCK |
| 5.2 | Plugin-side dedup flag prevents double-`Trace` on re-entry. | BLOCK |
| 5.3 | Missing `reservation_id` in `kong.ctx.shared` → skip silently (upstream short-circuited at access). No error log. | FIX-BEFORE-MERGE |
| 5.4 | Provider-specific usage parsing covered for OpenAI + Anthropic; unknown provider → `RUN_ABORTED`, not silent commit. | BLOCK |
| 5.5 | Malformed upstream JSON → `RUN_ABORTED`. | BLOCK |
| 5.6 | Commit call respects `TimeoutMS`; timeout logged but does not exit the request (upstream response already going to client). | FIX-BEFORE-MERGE |

## §6. Slice-5 specific (Lua fallback)

| # | Item | Severity |
|---|------|----------|
| 6.1 | Schema file declares all configurable fields including `sidecar_url`, `tenant_id`, `fail_open`. | BLOCK |
| 6.2 | `lua-resty-http` connection uses `ssl_verify=true` + client cert + key paths. No `ssl_verify=false`. | BLOCK |
| 6.3 | Lua plugin priority lower than `ai-proxy` so SpendGuard runs **before** upstream auth — Kong plugin priorities: `ai-proxy` = 770; SpendGuard must be > 770 (we use 950). | BLOCK |
| 6.4 | Docs and rockspec label plugin "experimental". | FIX-BEFORE-MERGE |
| 6.5 | Functional parity for `[D09-LUA-01..04]` proven via kong-pongo. | BLOCK |

## §7. Slice-6 specific (Helm chart)

| # | Item | Severity |
|---|------|----------|
| 7.1 | `kongPlugin.enabled=false` (default) renders zero kong-companion resources. | BLOCK |
| 7.2 | `kongPlugin.svidIssuer` unset with `enabled=true` → render-time fail-closed (`helm template` exits non-zero). | BLOCK |
| 7.3 | NetworkPolicy ingress allow-list = `app.kubernetes.io/name: kong` selector + namespace selector. No `0.0.0.0/0`. | BLOCK |
| 7.4 | Container image runs as non-root with read-only root filesystem (GA_09 pattern). | BLOCK |
| 7.5 | ServiceMonitor present when `kongPlugin.monitoring.serviceMonitor.enabled=true`. | FIX-BEFORE-MERGE |
| 7.6 | Reference `KongPlugin` CRD manifest in `examples/kong-gateway-composite/` uses placeholders (`<TENANT_ID>`, `<SIDECAR_URL>`), not hardcoded values. | FIX-BEFORE-MERGE |

## §8. Slice-7 specific (demo + docs)

| # | Item | Severity |
|---|------|----------|
| 8.1 | `DEMO_MODE=kong_gateway_real` boots cleanly on a workstation with only `OPENAI_API_KEY` exported. No other env required. | BLOCK |
| 8.2 | Demo uses **real** OpenAI upstream, not a stub (acceptance gate D09-E-03 — "real" in the demo name is load-bearing). | BLOCK |
| 8.3 | `verify_step_kong_gateway_real.sql` asserts ALLOW + DENY + commit lifecycle + chain continuity. All 5 assertions return `t`. | BLOCK |
| 8.4 | Audit chain `spendguard_verify_chain('kong_gateway_real')` returns `t`. | BLOCK |
| 8.5 | Docs page contains a working 10-line `curl` recipe. | FIX-BEFORE-MERGE |
| 8.6 | Docs page covers both Go and Lua install paths with a clear "production = Go, experimental = Lua" decision matrix. | FIX-BEFORE-MERGE |
| 8.7 | `README.md` adapter integrations table updated with Kong AI Gateway row. | FIX-BEFORE-MERGE |
| 8.8 | Starlight build green: `cd docs/site && npm run build` exit 0. | BLOCK |

## §9. Security review checklist (every slice)

| # | Item | Severity |
|---|------|----------|
| 9.1 | No plaintext credentials in `compose.kong.yaml`, `values.yaml`, or reference manifests. `OPENAI_API_KEY` always read from env. | BLOCK |
| 9.2 | mTLS required on every sidecar HTTP companion call. No `--no-verify` paths. | BLOCK |
| 9.3 | SVID-based tenant isolation: each tenant's plugin holds a distinct SVID; no shared cert (HARDEN_08 invariant). | BLOCK |
| 9.4 | No PII (request body content) logged at info level. Debug-level only with explicit opt-in flag. | BLOCK |
| 9.5 | NetworkPolicy default-deny on the SpendGuard namespace remains intact; the Kong allow-rule is additive. | BLOCK |
| 9.6 | Plugin distribution artifact (`.so`) signed when published to a customer-facing image (GA_09 cosign pattern). For dev images, signature optional but document the gap. | FIX-BEFORE-MERGE |
| 9.7 | No path traversal possible via plugin-config `client_cert_pem` / `client_key_pem` paths. Paths normalized + restricted to a configured prefix. | BLOCK |

## §10. Performance / scale guardrails

| # | Item | Severity |
|---|------|----------|
| 10.1 | Sidecar HTTP companion latency p99 < 25 ms under 100 concurrent requests against a 50ms-budget ledger (matches GA_08 projector SLO). | FIX-BEFORE-MERGE |
| 10.2 | Go plugin per-request allocation < 4 KiB excluding the request body buffer. Verified by `go test -bench` heap profile. | NIT |
| 10.3 | mTLS handshake reused across requests via `keep-alive`. No per-request handshake. | BLOCK |
| 10.4 | Sidecar HTTP companion `/v1/decision` connection pool sized to Kong's `nginx_worker_processes * 8` rule-of-thumb. | FIX-BEFORE-MERGE |

## §11. What we explicitly do not gate

- Lua plugin line coverage (experimental tier).
- Streaming SSE chunk-level enforcement (anti-scope §5 of design).
- Bedrock SigV4 mutation (Kong's `ai-proxy` handles upstream auth).
- Multi-cluster federation (deploy/demo target is single-pod).
- Kong Konnect (SaaS control plane) onboarding.

## §12. R5 panel arbitration triggers

If R5 produces > 0 BLOCK findings AND the implementer disputes, escalate to the Staff+ panel per `framework-coverage-build-plan-2026-06.md` §1.3 with these specific framing prompts:

1. Software Architect: "Is the HTTP companion the right Wire-format boundary, or should we have used a sidecar-per-Kong-pod with UDS instead?"
2. Backend Architect: "Is the body_filter buffering strategy safe under Kong's worker-process model, or do we need shared-memory state instead of `kong.ctx.shared`?"
3. AI Engineer: "Does the provider routing reuse hold for Kong's `ai-proxy` `route_type: preserve` mode (passthrough where Kong does not parse the request)?"
4. Security Engineer: "Is the mTLS + SVID model defensible when Kong is the multi-tenant entry point (a single Kong DataPlane serving N tenants vs N Kong DataPlanes)?"
5. Senior Developer: "Is the Go + Lua dual-implementation justified or is it 2x maintenance for 1.1x coverage?"

Summarizer (Software Architect by default) reconciles into merge-with-residuals / block / rework ruling.
