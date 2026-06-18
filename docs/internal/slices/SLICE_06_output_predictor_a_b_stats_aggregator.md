# Slice 06 — output_predictor service (Strategy A + B) + stats_aggregator

> **Branch**: `slice/SLICE_06_output_predictor_a_b_stats_aggregator`
> **Status**: draft
> **Spec ancestor(s)**: `output-predictor-service-spec-v1alpha1.md` §3 §4 §6; `stats-aggregator-spec-v1alpha1.md` (full); `cold-start-baseline-spec-v1alpha1.md` §2.5 (partial L1/L2/L4)
> **Depends on prior slices**: SLICE_01 (audit columns); SLICE_03 (tokenizer for input_tokens)
> **Blocks subsequent slices**: SLICE_07 (plugin contract), SLICE_08 (TOML loader for L2 full), SLICE_09 (run_cost_projector needs B per-call), SLICE_10 (egress proxy uses output_predictor)
> **Estimated PR size**: large (two new services + classifier + cache schema; ~2500 LOC)

---

## §0. TL;DR

Two new services land together: `output_predictor` (gRPC `Predict` with parallel A + B computation + 7-class classifier) and `stats_aggregator` (pure SQL hourly aggregation + RLS + drift detection). Cold-start partial: L4 (cache) + L1 (hard fallback) supported; L2 returns null until SLICE_08 populates TOML; L3 disabled.

---

## §1. Architectural context

per `output-predictor-service-spec-v1alpha1.md` §3-§4 §6; `stats-aggregator-spec-v1alpha1.md`. Serves Q1 (no-ML; pure SQL stats), partial Q3 (B per-call needed for projector Signal 1 cost).

---

## §2. Scope (must-do)

**output_predictor**:
- New `services/output_predictor/` crate
- `proto/spendguard/output_predictor/v1/predictor.proto`
- Strategy A implementation (max_tokens-based; instant)
- Strategy B implementation (cache lookup + L4 promotion check)
- Cold-start partial: L4 + L1 only (L2/L3 return None)
- Strategy selector skeleton (selector returns A under all policies until SLICE_07 brings C)
- 7-class classifier per spec §8.1
- prompt_class_fingerprint per §8.2
- `model_context_window.toml` initial file
- In-memory cache with 5min TTL

**stats_aggregator**:
- New `services/stats_aggregator/` crate (daemon + scheduler)
- `output_distribution_cache` table schema + RLS policy
- `run_length_distribution_cache` table schema + RLS policy
- Aggregation cycle SQL per spec §4.1 + §6
- Drift detection per §7.1 (event emission)
- Postgres advisory lock for singleton
- Hourly cron with per-tenant override stub (full API in SLICE_extra)

---

## §3. Out of scope

| 項目 | 推給 |
|---|---|
| Strategy C delegated mode | SLICE_07 |
| L2 model_default_distribution.toml | SLICE_08 |
| L3 federated aggregate | post-launch (per spec §5.6 trigger) |
| Per-tenant cadence override API in control plane | SLICE-extra |
| Run-length distribution consumer | SLICE_09 |

---

## §4. File-level change list

### 4.1 New files

- `services/output_predictor/Cargo.toml`, `src/main.rs`, `src/server.rs`, `src/strategy_a.rs`, `src/strategy_b.rs`, `src/classifier.rs`, `src/fingerprint.rs`, `src/selector.rs`, `src/cache.rs`
- `services/output_predictor/data/model_context_window.toml`
- `services/stats_aggregator/Cargo.toml`, `src/main.rs`, `src/scheduler.rs`, `src/aggregation.rs`, `src/drift_detector.rs`, `src/run_length.rs`
- `proto/spendguard/output_predictor/v1/predictor.proto`
- `services/canonical_ingest/migrations/00XX_output_distribution_cache.sql`
- `services/canonical_ingest/migrations/00XX_run_length_distribution_cache.sql`
- `charts/spendguard/templates/output_predictor.yaml`
- `charts/spendguard/templates/stats_aggregator.yaml`

### 4.2 Modified files

- `services/sidecar/src/decision.rs` — placeholder integration (full SLICE_10)
- `deploy/demo/compose.yaml` — add both services

---

## §5. Schema / proto changes

per `output-predictor-service-spec-v1alpha1.md` §2.1 (proto); `stats-aggregator-spec-v1alpha1.md` §5 + §6 (cache tables with RLS).

---

## §6. Audit-chain impact

- `predicted_a_tokens`, `predicted_b_tokens`, `prediction_strategy_used`, `prediction_confidence`, `prediction_sample_size`, `cold_start_layer_used` (NULL/L1/L4) columns populated
- `prediction_drift_alert` CloudEvents emitted to canonical_ingest (signed; immutable)
- Per stats_aggregator §3.2 RLS prevents cross-tenant reads

---

## §7. Failure mode coverage

| 場景 | 行為 |
|---|---|
| stats_aggregator cache unreachable | B fall to cold-start L1; A always succeeds |
| Cache stale > 2h | falls to cold-start (per spec §4.2) |
| Classifier mis-classify | reservation safe; calibration-report flags later |
| RLS bypass attempt | adversarial test catches; query returns 0 rows |
| Advisory lock held by stale instance | new aggregator process skips cycle + emits event |
| canonical_events DB outage | aggregation fail; cache stale; predictor still serves via cold-start |

---

## §8. Acceptance criteria

### 8.1 Unit tests

- Strategy A formula correct for all edge cases (max_tokens=0, context_window override, etc.)
- Strategy B cache lookup honors staleness gate (2h)
- 7-class classifier correctly labels 100 hand-labeled golden samples ≥ 90%
- Fingerprint determinism: same template → same hash
- Selector returns A under STRICT_CEILING regardless of B value

### 8.2 Integration tests

- End-to-end: tokenizer → output_predictor → audit row populated with B + L4 layer
- stats_aggregator cycle: insert 100 synthetic events → cache populated within 1 cycle
- RLS: caller with tenant_b cannot read tenant_a cache rows (cross-tenant injection test)

### 8.3 Property tests

- Reservation ≥ actual for 1000 simulated decisions under STRICT_CEILING

### 8.4 Audit invariant tests

- verify-chain on new prediction column rows clean
- Drift alert event signed + replicated

### 8.5 Demo-mode regression

`make demo-up` shows output_predictor + stats_aggregator in service map; default mode still works

---

## §9. Slice-specific adversarial review checklist

1. Classifier 7-class rules implemented in priority order per `output-predictor-service-spec-v1alpha1.md` §8.1?
2. RLS policy on cache tables: `app.current_tenant_id` set via connection-level statement before query?
3. Advisory lock semantics: what happens if held by long-dead process? PostgreSQL automatic timeout?
4. Aggregation query performance on 1M-row demo: < 15 min per cycle?
5. Cold-start partial: how does code path differ between L1 (return None) and L4 (return Some)? Show code branches.
6. Selector under STRICT_CEILING: explicitly returns A even when B = Some? Tested.
7. In-memory cache TTL: 5min. What's the metric for cache hit rate?
8. Drift detection 2σ: explicit MIN_SAMPLES_FOR_ALERT = 100 enforced?
9. Concurrent Predict calls for same bucket: cache update race tested?
10. stats_aggregator emitting CloudEvent vs writing to log: which channel?

---

## §10. Out-of-scope deferrals

| 項目 | 推給 |
|---|---|
| Strategy C plugin path | SLICE_07 |
| L2 TOML loader full | SLICE_08 |
| L3 federated | post-launch |
| Per-tenant aggregation cadence override | SLICE-extra |

---

## §11. Risk / rollback plan

- Risk: stats_aggregator cycle long-running blocks Postgres
- Mitigation: per-tenant transaction; rollback on failure
- Rollback: disable output_predictor via Helm; sidecar falls to legacy estimator

---

## §12. Review Execution Notes

- Recommended reviewer profile: Data Engineer for stats_aggregator portion + `Backend Architect` for output_predictor
- Review depth: deep
- Expected rounds: 3-4 (two services + cache schema + classifier)

---

## §13. Adoption history (filled during review)

| Round | Reviewer | 採納率 | 主要產出 |
|---|---|---|---|
| (placeholder) | (placeholder) | (placeholder) | (placeholder) |

---

## §14. Merge checklist

- [ ] §8 acceptance green
- [ ] §9 specific clear
- [ ] universal §1.1 (audit-chain) + §1.4 (cold-start partial) + §1.9 (multi-tenant isolation) green
- [ ] PR references both `output-predictor-service-spec-v1alpha1.md` + `stats-aggregator-spec-v1alpha1.md`

---

*Slice version: SLICE_06_output_predictor_a_b_stats_aggregator v1alpha1 (draft) | Spec ancestors: output-predictor-service-spec §3 §4 §6 + stats-aggregator-spec (full) + cold-start-baseline-spec §2.5 partial | Depends: SLICE_01, SLICE_03 | Branch: `slice/SLICE_06_output_predictor_a_b_stats_aggregator`*
