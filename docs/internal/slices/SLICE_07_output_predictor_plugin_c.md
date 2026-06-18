# Slice 07 — output_predictor plugin contract + delegated mode (Strategy C)

> **Branch**: `slice/SLICE_07_output_predictor_plugin_c`
> **Status**: draft
> **Spec ancestor(s)**: `output-predictor-plugin-contract-v1alpha1.md` (full); `output-predictor-service-spec-v1alpha1.md` §5
> **Depends on prior slices**: SLICE_06 (output_predictor skeleton + selector)
> **Blocks subsequent slices**: SLICE_13 (calibration-report reads C predictions), SLICE_14 (customer template)
> **Estimated PR size**: medium (proto + delegated C path + circuit breaker + control plane API + isolation tests; ~1200 LOC)

---

## §0. TL;DR

Customer-trained Strategy C plugin: gRPC contract proto + delegated call mode in output_predictor + per-tenant circuit breaker + control plane endpoint registration + multi-tenant isolation enforcement. **Critical invariant**: any plugin failure silently falls to Strategy B with `customer_predictor_error` metric; never blocks reservation.

---

## §1. Architectural context

per `output-predictor-plugin-contract-v1alpha1.md` (full). Serves Q1 (no-ML in SpendGuard; C is delegated to customer).

---

## §2. Scope (must-do)

- `proto/spendguard/output_predictor_plugin/v1/plugin.proto` (Predict + HealthCheck)
- Strategy C delegated mode in `services/output_predictor/src/strategy_c.rs`
- Per-(tenant) plugin endpoint cache; mTLS client cert injection
- 50ms hard cap on Predict RPC + validation of response (per spec §5.1)
- Per-(tenant) circuit breaker per spec §6
- Selector updated: when C available → `prediction_strategy_used = C`
- Control plane API for endpoint registration / update / delete + force-reset
- mTLS client cert issuance pipeline (per-tenant SVID subject)
- Per-tenant isolation enforcement (tenant_id check on every Predict response)
- `customer_predictor_*` metrics

---

## §3. Out of scope

| 項目 | 推給 |
|---|---|
| Customer reference template | SLICE_14 |
| Plugin training | customer responsibility (per Q1) |
| Audit `plugin_error_reason` enriched column | v1beta1 |

---

## §4. File-level change list

### 4.1 New files

- `proto/spendguard/output_predictor_plugin/v1/plugin.proto`
- `services/output_predictor/src/strategy_c.rs`
- `services/output_predictor/src/plugin_client.rs` (gRPC client + circuit breaker)
- `services/output_predictor/src/endpoint_cache.rs`
- `services/control_plane/src/handlers/predictor_plugins.rs`
- `services/control_plane/migrations/00XX_predictor_plugin_endpoints.sql`
- `services/cert_issuer/...` (or extend existing cert pipeline)

### 4.2 Modified files

- `services/output_predictor/src/server.rs` — wire Strategy C parallel call
- `services/output_predictor/src/selector.rs` — incorporate C result
- `charts/spendguard/templates/control_plane.yaml` — add new API endpoint

---

## §5. Schema / proto changes

per `output-predictor-plugin-contract-v1alpha1.md` §2.1 (Predict + HealthCheck); §8 (control plane API).

New table `predictor_plugin_endpoints` (control plane side):
```sql
CREATE TABLE predictor_plugin_endpoints (
    plugin_endpoint_id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL UNIQUE,  -- one endpoint per tenant
    endpoint_url TEXT NOT NULL,
    server_cert_fingerprint TEXT NOT NULL,
    client_cert_id TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    registered_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    last_health_check_at TIMESTAMPTZ,
    current_health_status TEXT
);
```

---

## §6. Audit-chain impact

- `predicted_c_tokens` column populated when C path succeeds
- `prediction_strategy_used = 'C'` when C available + policy allows
- Plugin error fall to B: `predicted_c_tokens = NULL`, `prediction_strategy_used = 'B'`
- CloudEvents `spendguard.plugin.registered / updated / deleted / force_reset` emitted

---

## §7. Failure mode coverage

per `output-predictor-plugin-contract-v1alpha1.md` §5.1 全 8 failure modes列出 + fall to B. Adversarial review checklist must verify each path.

---

## §8. Acceptance criteria

### 8.1 Unit tests

- 8 failure modes each tested: timeout / gRPC error / negative return / overflow / illegal confidence / deserialization / TLS error / NOT_SERVING
- Circuit breaker state machine: closed → 10 failures → open → 5min → half-open → success → closed
- Selector: C path with success vs fallback to B

### 8.2 Integration tests

- Real plugin endpoint mock: Predict success path → audit row with C populated
- Plugin endpoint timeout → fall to B; metric emitted
- Multi-tenant: tenant A endpoint cannot be called for tenant B's Predict

### 8.3 Property tests

- For 1000 simulated plugin responses (varied success/failure mix): reservation always = A (under STRICT_CEILING); never blocked

### 8.4 Audit invariant tests

- verify-chain on rows with C populated; verify-chain on rows where C fell to B (NULL c)

### 8.5 Demo-mode regression

- Add `make demo-up DEMO_MODE=plugin_c_synthetic` demo with mock plugin

---

## §9. Slice-specific adversarial review checklist

1. Plugin malicious return test: plugin returns `predicted_output_tokens = 10^15` → fall to B + circuit breaker count up?
2. Plugin malicious return: negative or > model_context_window → fall to B + metric.
3. mTLS cert mismatch: plugin cert subject != expected tenant → reject + metric.
4. Circuit breaker: 10 consecutive failures over what time window? Spec implies absolute count, no time-window.
5. Half-open probe: which call is the probe? Synthetic with `probe-` prefix per §6.4.
6. Per-tenant endpoint cache TTL: how often refresh from control plane?
7. mTLS rotation: 30-day cert rotation handled by control plane or plugin self-rotation?
8. Customer plugin invocation latency overhead on hot path: measured.
9. Selector + C: under STRICT_CEILING, reserved_strategy = 'A' even when C succeeds → verify code path.
10. Customer plugin endpoint registration validation: idempotent? Cleanup if endpoint unreachable?

---

## §10. Out-of-scope deferrals

| 項目 | 推給 |
|---|---|
| Customer template (Python reference) | SLICE_14 |
| Audit plugin_error_reason column | v1beta1 |
| Dashboard plugin health surface | Separate frontend slice |

---

## §11. Risk / rollback plan

- Risk: customer plugin path adds latency exceeding 50ms tail
- Mitigation: 50ms hard deadline at output_predictor side; circuit breaker
- Rollback: disable C via control plane API (`enabled = FALSE`); output_predictor falls back to B/A

---

## §12. Review Execution Notes

- Recommended reviewer profile: Backend Architect + `Security Engineer` (for mTLS / cert handling)
- Review depth: deep
- Expected rounds: 3-4 (security-sensitive)

---

## §13. Adoption history (filled during review)

| Round | Reviewer | 採納率 | 主要產出 |
|---|---|---|---|
| (placeholder) | (placeholder) | (placeholder) | (placeholder) |

---

## §14. Merge checklist

- [ ] §8 acceptance green
- [ ] §9 specific clear
- [ ] universal §1.8 (failure isolation: plugin fail → B) verified
- [ ] universal §1.9 (multi-tenant isolation) verified by adversarial test
- [ ] PR references `output-predictor-plugin-contract-v1alpha1.md`

---

*Slice version: SLICE_07_output_predictor_plugin_c v1alpha1 (draft) | Spec ancestor: output-predictor-plugin-contract-v1alpha1.md | Depends: SLICE_06 | Branch: `slice/SLICE_07_output_predictor_plugin_c`*
