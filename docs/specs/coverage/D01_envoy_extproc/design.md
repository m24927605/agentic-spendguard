# D01 — Envoy AI Gateway ExtProc Sidecar — Design

**Status:** Draft for R1 review
**Owner sub-agent (impl):** Backend Architect
**Parent strategy:** [`docs/strategy/framework-coverage-2026-06.md`](../../../strategy/framework-coverage-2026-06.md) §"Should integrate, not compete" + Tier 1 row 1
**Build plan:** [`docs/strategy/framework-coverage-build-plan-2026-06.md`](../../../strategy/framework-coverage-build-plan-2026-06.md) §2.1 row D01
**Companion specs touched:** sidecar adapter v1alpha1 (no proto changes); egress_proxy routing v0.5 (read-only re-use)

---

## §1. What we're building

A new service `services/envoy_extproc`: a single binary plus a Helm sub-chart. It speaks the Envoy ExternalProcessor gRPC API on one side and the existing SpendGuard sidecar adapter contract on the other. Envoy AI Gateway v0.6 (CNCF, GA 2026-05) calls out via gRPC ExtProc for every LLM-bound request; our service translates each `ProcessingRequest` into `RequestDecision` / `EmitTraceEvents` and returns the corresponding `ProcessingResponse` (continue / deny / mutate). The Helm sub-chart slots into [`charts/spendguard/templates/`](../../../../charts/spendguard/templates/) following the `sidecar.yaml` template shape.

## §2. Why this slot, why now

- **Architectural fit is 1:1.** Envoy AI Gateway asks for a sidecar over gRPC; SpendGuard already has one ([`services/sidecar/src/server/adapter_uds.rs`](../../../../services/sidecar/src/server/adapter_uds.rs)). The ExtProc service is a *protocol shim*, not a new decision engine — no new ledger, tokenizer, or audit chain. We re-use the SLICE_11 routing table at [`services/egress_proxy/src/routing.rs:181-279`](../../../../services/egress_proxy/src/routing.rs).
- **CNCF distribution.** Envoy AI Gateway is the only major LLM gateway under open CNCF governance. Kong AI Gateway and Apigee Extension Processor are commercial; Envoy lights up every Istio / service-mesh shop.
- **First-mover window.** Cloudflare AI Gateway Spend Limits GA'd 2026-06-05; ExtProc callouts are the next likely extension. We do not depend on any upstream PR; customers opt in via their own Envoy `ExternalProcessor` config — feasibility is 100% in-repo.

## §3. Key architectural decisions

### 3.1 ExtProc is a translation layer, not a re-implementation

`envoy_extproc` is a *client* of the sidecar adapter, identical to how `egress_proxy` is a client. The decision engine, ledger, tokenizer, and audit chain stay where they are; audit-chain invariants stay in one place.

### 3.2 Token counting reuses egress_proxy routing

The service depends on `spendguard-tokenizer` (existing workspace member) plus `resolve_model_id` / `resolve_tokenizer_kind` from `services/egress_proxy/src/routing.rs`. Slice 1 extracts those into a shared crate `crates/spendguard-provider-routing` so both consumers share one source of truth.

### 3.3 mTLS over TCP (not UDS)

ExtProc runs in a different pod from the sidecar (the sidecar is per-app-pod; Envoy AI Gateway is its own deployment). SO_PEERCRED is unavailable. We use mTLS over TCP, mirroring [`charts/spendguard/templates/output_predictor_plugin_svid.yaml`](../../../../charts/spendguard/templates/output_predictor_plugin_svid.yaml); HARDEN_08's per-tenant SVID minting carries over.

### 3.4 Deny-on-fail-closed default

If the sidecar is unreachable, ExtProc returns `immediate_response` HTTP 503 + `RetryAfter`. We never silently pass traffic — matches the egress_proxy fall-closed posture (SLICE_03 spec §8).

### 3.5 Wire-format scope

v1 covers the Request Headers + Request Body and Response Body phases for `chat/completions` and `messages`. No `TRAILERS` phase (Envoy AI Gateway v0.6 reference manifests do not require it for budget gating). Streaming SSE chunk-by-chunk enforcement is out of scope for v1; the commit lane runs at end-of-response-body.

## §4. Slice plan (7 slices)

| # | Name | Size | Scope |
|---|------|------|-------|
| 1 | `COV_01_envoy_extproc_skeleton` | M | Cargo crate scaffold; `envoy.service.ext_proc.v3` proto wired via `tonic-build`; extract `spendguard-provider-routing` shared crate; `Handshake` → sidecar UDS proven against `decision` demo mode |
| 2 | `COV_02_envoy_extproc_token_counter` | S | Wire `spendguard-tokenizer` library + `provider-routing::resolve_tokenizer_kind` into ExtProc Request-Body phase; emit `input_tokens` into a `ClaimEstimate` |
| 3 | `COV_03_envoy_extproc_budget_query` | M | Translate ExtProc Request → `RequestDecision`; map `DecisionResponse.decision` to ExtProc `CommonResponse` (CONTINUE / immediate_response 429 / immediate_response 403); reservation_id stored in per-stream state |
| 4 | `COV_04_envoy_extproc_audit_emit` | M | Wire ExtProc Response-Body phase → `EmitTraceEvents` with `LLM_CALL_POST.SUCCESS` (provider-reported usage from response body) or `RUN_ABORTED` on upstream 5xx; idempotent via reservation_id |
| 5 | `COV_05_envoy_extproc_conformance` | M | Conformance fixtures against Envoy AI Gateway v0.6 reference manifest examples (`token_counting.yaml`, `budget.yaml`); golden-file tests |
| 6 | `COV_06_envoy_extproc_helm` | S | `charts/spendguard/templates/envoy_extproc.yaml` + values; SVID mount; NetworkPolicy ingress from `app.kubernetes.io/name: envoy-ai-gateway`; ServiceMonitor |
| 7 | `COV_07_envoy_extproc_demo` | S | `DEMO_MODE=envoy_extproc` in `deploy/demo/Makefile`; `verify_step_envoy_extproc.sql`; one-pager `docs/site/docs/integrations/envoy-ai-gateway.md` |

## §5. Anti-scope

- No Envoy AI Gateway control-plane CRDs; customers configure `ExternalProcessor` themselves (we ship a reference snippet).
- No upstream PR to envoyproxy/ai-gateway. The contract is public; we conform.
- No ExtProc `TRAILERS` / metadata-only phases.
- No support for the older non-AI Envoy Gateway distribution.
- No streaming SSE budget enforcement in v1.
- No customer plugin contract integration (output_predictor plugin C); Strategy A reservation is sufficient for v1.

## §6. Out-of-band coordination

None. The ExtProc contract is stable in Envoy AI Gateway v0.6; we ship against the public proto.

---

*Locked decisions: §3.1, §3.2, §3.3, §3.4, §3.5. Slice plan: §4 (7 slices). Anti-scope: §5.*
