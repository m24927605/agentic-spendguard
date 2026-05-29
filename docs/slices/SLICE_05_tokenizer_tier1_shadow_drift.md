# Slice 05 — Tokenizer Tier 1 shadow + drift alert

> **Branch**: `slice/SLICE_05_tokenizer_tier1_shadow_drift`
> **Status**: draft
> **Spec ancestor(s)**: `tokenizer-service-spec-v1alpha1.md` §4, §4.5; `stats-aggregator-spec-v1alpha1.md` (drift_alert event semantics)
> **Depends on prior slices**: SLICE_04 (Anthropic / Gemini Tier 2 needed for shadow comparison)
> **Blocks subsequent slices**: none directly, but informs calibration-report (SLICE_13) and ops dashboard
> **Estimated PR size**: medium (shadow worker + provider clients + circuit breaker + t1_samples table; ~1000 LOC)

---

## §0. TL;DR

Background async shadow worker that samples (default 1%) of tokenize calls and verifies against provider count_tokens API (Anthropic / Gemini). Detects drift > per-kind threshold; emits `tokenizer_drift_alert` CloudEvent; raises sample rate to 100% for 1h cool-down. Circuit breaker on Tier 1 endpoint. New `tokenizer_t1_samples` table (NOT in audit chain).

---

## §1. Architectural context

per `tokenizer-service-spec-v1alpha1.md` §4 (Tier 1 shadow), §4.5 (circuit breaker). Serves Q2 (Tier 1 strictly off hot path; verification only).

---

## §2. Scope (must-do)

- New `services/tokenizer/src/shadow_worker.rs` async loop
- Provider clients: Anthropic `POST /v1/messages/count_tokens`, Gemini `POST /v1/models/{model}:countTokens`
- Per-(tenant, model) sampling rate state in `services/tokenizer/src/sample_rate_state.rs` (loaded from control plane)
- Drift detection per `tokenizer-service-spec-v1alpha1.md` §4.1-§4.2
- 100% cool-down window mechanism per §4.3
- `tokenizer_t1_samples` table per §4.4 (separate from audit chain)
- Circuit breaker per §4.5
- ShadowVerify gRPC handler implementation
- Control plane API `POST /tokenizer/sampling-rate` for per-tenant override

---

## §3. Out of scope

| 項目 | 推給 |
|---|---|
| Provider HTTP client retries (use existing http client crate) | use existing |
| Audit chain entries for T1 samples | Intentionally NOT (per §4.4) |
| Calibration-report integration of T1 samples | SLICE_13 |

---

## §4. File-level change list

### 4.1 New files

- `services/tokenizer/src/shadow_worker.rs`
- `services/tokenizer/src/provider_clients/anthropic.rs`
- `services/tokenizer/src/provider_clients/gemini.rs`
- `services/tokenizer/src/sample_rate_state.rs`
- `services/tokenizer/src/circuit_breaker.rs`
- `services/tokenizer/migrations/00XX_tokenizer_t1_samples.sql`

### 4.2 Modified files

- `services/tokenizer/src/server.rs` — wire shadow worker as background task
- `services/tokenizer/src/main.rs` — spawn shadow_worker on startup
- `services/control_plane/src/handlers/tokenizer.rs` — add sampling rate API
- `proto/spendguard/tokenizer/v1/tokenizer.proto` — implement ShadowVerify

### 4.3 Helm / config

- `charts/spendguard/templates/tokenizer.yaml` — add Anthropic / Gemini provider endpoint env vars
- Tokenizer service requires Anthropic / Gemini API tokens (mTLS or API key) for shadow path

---

## §5. Schema / proto changes

- `tokenizer_t1_samples` table per `tokenizer-service-spec-v1alpha1.md` §4.4
- No proto wire changes (ShadowVerify proto already defined SLICE_03)

---

## §6. Audit-chain impact

- **None directly**. Tier 1 samples land in `tokenizer_t1_samples` not audit_outbox (per spec §4.4)
- BUT: `tokenizer_drift_alert` CloudEvent emitted to canonical_ingest as signed event (audit chain entry; tied to stats_aggregator's drift_alert family; per `stats-aggregator-spec-v1alpha1.md` §7.2 schema)

---

## §7. Failure mode coverage

| 場景 | 行為 |
|---|---|
| Anthropic / Gemini endpoint timeout | circuit breaker open after 10 consecutive failures; shadow skip; hot path Tier 2 unaffected |
| Provider count_tokens returns different schema | emit `provider_count_tokens_schema_drift` + skip sample |
| sample_rate config corrupted | fall to default 1% |
| Drift alert false positive | operator review; correct via control plane override |
| Cool-down 100% sampling triggers rate limit | retry-after-backoff + reduce |

---

## §8. Acceptance criteria

### 8.1 Unit tests

- Sample rate logic correctly drops requests when below rate
- Drift detection per-kind thresholds correct
- Cool-down window 1h after first alert; rate reverts after no new alerts
- Circuit breaker state transitions per §4.5

### 8.2 Integration tests

- Mock Anthropic / Gemini endpoint; inject drift; verify alert emission
- Circuit breaker open after 10 failures; half-open probe success closes

### 8.3 Chaos tests

per `tokenizer-service-spec-v1alpha1.md` §11.1:
- `tier1_endpoint_outage`
- `tier1_endpoint_recovery`
- `drift_alert_cool_down`

### 8.4 Audit invariant tests

- `tokenizer_drift_alert` CloudEvents land in canonical_ingest with valid signature
- `verify-chain` over the drift alert subset passes

### 8.5 Demo-mode regression

- `make demo-up DEMO_MODE=agent_real_anthropic` shows shadow worker active; Tier 2 hot path unaffected by shadow latency

---

## §9. Slice-specific adversarial review checklist

1. Hot path latency invariant: Tier 1 NEVER called from sidecar / egress_proxy decision flow? Verify by source grep.
2. Sample rate persistence: in-memory cache + control plane sync. What's the refresh interval?
3. Drift threshold per-kind in code matches spec §4.2 table verbatim?
4. Cool-down window: what happens if 1 hour ends but drift still present? Spec says re-alert + maintain.
5. `tokenizer_t1_samples` table retention default 90 days? Cleanup job exists?
6. Provider API key rotation: how does shadow worker pick up new key? Hot-reload or restart?
7. Multi-tenant: sample_rate per (tenant, model). Cross-tenant rate leakage tested?
8. Circuit breaker isolation: per-(tenant, model) not global. Tested under cross-tenant fault?

---

## §10. Out-of-scope deferrals

| 項目 | 推給 |
|---|---|
| Real-time dashboard for drift trends | Separate dashboard slice |
| Provider count_tokens caching | Not needed at 1% sample rate |

---

## §11. Risk / rollback plan

- Risk: shadow worker overload provider API or causes 429 cascades on real workload
- Mitigation: 1% default rate + circuit breaker + retry-after handling
- Rollback: disable shadow worker via env var `TOKENIZER_SHADOW_ENABLED=false`; hot path unaffected

---

## §12. AIT execution notes

- Recommended `--agent Backend Architect`
- `--review-budget deep`
- Expected rounds: 3 (concurrency + provider integration)

---

## §13. Adoption history (filled during review)

| Round | Reviewer | 採納率 | 主要產出 |
|---|---|---|---|
| (placeholder) | (placeholder) | (placeholder) | (placeholder) |

---

## §14. Merge checklist

- [ ] §8 + §11.1 chaos green
- [ ] §9 specific clear
- [ ] Tier 1 strictly off hot path verified (universal §1.2)
- [ ] Drift alert CloudEvent verified by canonical_ingest
- [ ] PR references `tokenizer-service-spec-v1alpha1.md` §4

---

*Slice version: SLICE_05_tokenizer_tier1_shadow_drift v1alpha1 (draft) | Spec ancestor: tokenizer-service-spec-v1alpha1.md §4 | Depends: SLICE_04 | Branch: `slice/SLICE_05_tokenizer_tier1_shadow_drift`*
