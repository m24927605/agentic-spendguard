# Slice 10 — egress_proxy `decision.rs` rewrite

> **Branch**: `slice/SLICE_10_egress_proxy_decision_rewrite`
> **Status**: draft
> **Spec ancestor(s)**: `predictor-architecture-spec-v1alpha1.md` §2.2 §4; `tokenizer-service-spec-v1alpha1.md`; `output-predictor-service-spec-v1alpha1.md`; `audit-chain-prediction-extension-v1alpha1.md`
> **Depends on prior slices**: SLICE_03 (tokenizer library), SLICE_06 (output_predictor service), SLICE_09 (run_cost_projector)
> **Blocks subsequent slices**: SLICE_15 (E2E benchmark)
> **Estimated PR size**: medium (rewrite `estimate_tokens` + integrate 3 services + populate 17 audit columns; ~1000 LOC + ~500 deleted)

---

## §0. TL;DR

Replace `services/egress_proxy/src/decision.rs::estimate_tokens` (the 17-line heuristic from HANDOFF §2.2) with calls to tokenizer service (Tier 2 hot path), output_predictor service (A+B+C), and run_cost_projector. New `estimate_call_cost()` function returns full 10 prediction columns + 3 run-level columns. Wire results through existing `DecisionRequest` flow. **All audit rows from this point on carry full prediction metadata**.

---

## §1. Architectural context

per `predictor-architecture-spec-v1alpha1.md` §2.2 (the 17-line heuristic this slice replaces). Serves Q2 + Q3 + Q4 in production hot path.

---

## §2. Scope (must-do)

- Remove `services/egress_proxy/src/decision.rs::estimate_tokens` (line 277-295)
- New function `estimate_call_cost(body, model, tenant_id, agent_id, ...) → ClaimEstimate`
- Call tokenizer service library (Tier 2 hot path) for `input_tokens` + `tokenizer_tier` + `tokenizer_version_id`
- Call output_predictor service for `predicted_a/b/c_tokens` + strategy + confidence + cold_start_layer
- Call run_cost_projector for `run_projection` + `predicted_remaining_steps` + `steps_completed`
- Build full `BudgetClaim` with rich metadata
- Update test suite: replace `estimate_tokens_*` tests with new tests covering real tiktoken-rs results
- Maintain Contract §14 50ms p99 sidecar latency budget
- Backward compatibility: SDK wrapper-mode still works (caller-supplied claim_estimator no longer required as default)

---

## §3. Out of scope

| 項目 | 推給 |
|---|---|
| Multi-provider routing (Anthropic / Bedrock / Vertex) | SLICE_11 |
| SDK default estimator | SLICE_12 |
| Calibration-report integration | SLICE_13 |

---

## §4. File-level change list

### 4.1 Modified files

- `services/egress_proxy/src/decision.rs` — remove `estimate_tokens`; add `estimate_call_cost`
- `services/egress_proxy/src/forward.rs` — propagate enriched DecisionRequest
- `services/egress_proxy/Cargo.toml` — add deps for tokenizer / output_predictor / run_cost_projector clients
- `services/sidecar/src/decision/transaction.rs` — `build_budget_claims` no longer requires `projected_claims` non-empty (caller may supply OR egress_proxy may build)
- `tests/egress_proxy/decision_tests.rs` — full rewrite

### 4.2 New files

- `services/egress_proxy/src/predictor_client.rs` — gRPC clients for output_predictor + run_cost_projector

---

## §5. Schema / proto changes

No new proto changes (uses existing tokenizer + output_predictor + run_cost_projector protos).

---

## §6. Audit-chain impact

- **First production write of all 17 new columns** (prior slices wrote them in test paths; this slice activates the hot path)
- `tokenizer_tier`, `tokenizer_version_id`, `predicted_a/b/c_tokens`, `reserved_strategy`, `prediction_strategy_used`, `prediction_policy_used`, `prediction_confidence`, `prediction_sample_size`, `cold_start_layer_used`, `run_projection_at_decision_atomic`, `run_predicted_remaining_steps`, `run_steps_completed_so_far` all populated
- Existing `commit_estimated` path (SLICE_extra) populates the 4 commit-side columns

---

## §7. Failure mode coverage

| 場景 | 行為 |
|---|---|
| tokenizer service down | fail-closed (per Tier 2 panic invariant) |
| output_predictor service down | A safety net via local computation; B/C null; metric emitted |
| run_cost_projector down | pass-through (no RUN_* emitted); reservation uses A |
| Aggregate latency > 50ms p99 | metric `decision_latency_breach`; falls within Contract §14 budget |
| Backward compat: SDK with explicit projected_claims | accepted; egress_proxy doesn't override |

---

## §8. Acceptance criteria

### 8.1 Unit tests

- New tests cover real tiktoken-rs results for known fixture inputs
- Existing `estimate_tokens_*` tests removed (no longer applicable)
- 17 audit columns populated correctly for each strategy/policy combination

### 8.2 Integration tests

- End-to-end: real OpenAI request → tokenizer → predictor → projector → audit row with full metadata
- LiteLLM proxy mode: requests pass through with enrichment
- Backward compat with explicit `projected_claims`: SDK wrapper-mode still works

### 8.3 Benchmarks

- Aggregate decision latency p99 < 50ms (Contract §14 SLO)
- Tokenizer hot-path p99 < 1ms (library form)

### 8.4 Audit invariant tests

- verify-chain on new prediction rows green
- Mirror cross-check via `--check-prediction-mirror` green

### 8.5 Demo-mode regression

- All 8+ demos pass; audit rows now have full prediction metadata
- `make benchmark` (`benchmarks/runaway-loop/`) still shows SpendGuard better than competitors

### 8.6 Backwards compat

- Pre-slice SDK wrapper-mode with caller-supplied claim_estimator still works

---

## §9. Slice-specific adversarial review checklist

1. Is `estimate_tokens` actually deleted (not deprecated)? Show diff.
2. New `estimate_call_cost`: aggregate latency budget? Per-call profile.
3. tokenizer service library vs gRPC: which used in proxy? Library preferred.
4. output_predictor unreachable: fall back to local Strategy A? Show code path.
5. run_cost_projector unreachable: skip RUN_* emission; reservation = A. Confirmed.
6. Audit row 17 columns: ordering of writes in DecisionRequest pipeline?
7. SDK backward compat: how does egress_proxy know SDK supplied claim_estimator vs not?
8. LiteLLM mode integration: which path runs in proxy?
9. Existing 8+ demos: explicit regression matrix? Each demo's decision path verified.
10. m1_benchmark_runaway_loop precision improvement: target measurable?

---

## §10. Out-of-scope deferrals

| 項目 | 推給 |
|---|---|
| Multi-provider routing in forward.rs | SLICE_11 |
| SDK default estimator | SLICE_12 |
| Commit-side actual_input/output_tokens write | SLICE-extra (in commit_estimated handler) |

---

## §11. Risk / rollback plan

- **Highest-risk slice** — the heuristic is the 17 lines the maintainer explicitly highlighted in HANDOFF §2.2; full replacement
- Mitigation: 100 fixture test cases comparing old heuristic + new estimator over varied inputs
- Rollback: revert single PR; old heuristic path restored

---

## §12. AIT execution notes

- Recommended `--agent Backend Architect`
- `--review-budget deep`
- Expected rounds: 4-5 (high-stakes; many dependencies). Plan for Staff+ panel possibility.

---

## §13. Adoption history (filled during review)

| Round | Reviewer | 採納率 | 主要產出 |
|---|---|---|---|
| (placeholder) | (placeholder) | (placeholder) | (placeholder) |

---

## §14. Merge checklist

- [ ] §8 acceptance + benchmark green
- [ ] §9 specific clear; full audit row populated
- [ ] universal §1.1 (audit-chain coverage of all 17) verified
- [ ] universal §1.3 (Strategy A as reservation) verified
- [ ] universal §1.11 (backwards compat) preserved
- [ ] PR references multiple specs (architecture; tokenizer; output_predictor; run_cost_projector; audit-chain extension)

---

*Slice version: SLICE_10_egress_proxy_decision_rewrite v1alpha1 (draft) | Critical: this is where the 17-line heuristic dies and full prediction metadata starts on every audit row | Depends: SLICE_03 + SLICE_06 + SLICE_09 | Branch: `slice/SLICE_10_egress_proxy_decision_rewrite`*
