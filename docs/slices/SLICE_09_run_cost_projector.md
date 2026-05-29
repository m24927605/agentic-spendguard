# Slice 09 — run_cost_projector service + RUN_* decision codes

> **Branch**: `slice/SLICE_09_run_cost_projector`
> **Status**: draft
> **Spec ancestor(s)**: `run-cost-projector-spec-v1alpha1.md` (full); `contract-dsl-spec-v1alpha2.md` §3 §5; `audit-chain-prediction-extension-v1alpha1.md` §2.2
> **Depends on prior slices**: SLICE_02 (RUN_* codes + DSL evaluator pass-through); SLICE_06 (output_predictor Strategy B feed); SLICE_06 (stats_aggregator run-length distribution)
> **Blocks subsequent slices**: SLICE_10 (egress_proxy integrates projector)
> **Estimated PR size**: medium-large (new service + state cache + Signal 1/2/3 + audit columns wire; ~1800 LOC)

---

## §0. TL;DR

New `services/run_cost_projector/` gRPC service. Signal 1/2/3 layered projection. Per-(run_id) in-memory state cache with 30-min TTL + LRU evict + recovery from audit chain. RUN_BUDGET_PROJECTION_EXCEEDED / RUN_DRIFT_DETECTED / RUN_STEPS_EXCEEDED emission per code precedence. Sidecar integration wires projector before reserve. Project p99 ≤ 5ms.

---

## §1. Architectural context

per `run-cost-projector-spec-v1alpha1.md` (full); activates RUN_* codes that SLICE_02 added as pass-through. Serves Q3 (per-run projection moat).

---

## §2. Scope (must-do)

- New `services/run_cost_projector/` crate
- `proto/spendguard/run_cost_projector/v1/projector.proto` (Project + TerminateRun)
- Signal 1 implementation (induced from history; reads stats_aggregator run_length_distribution_cache)
- Signal 2 implementation (per-step re-projection + drift detection per spec §4.2)
- Signal 3 implementation (optional planned_steps_hint override)
- Signal layering algorithm per spec §6
- In-memory RunState cache per spec §7
- LRU eviction + memory pressure cap
- Recovery from audit chain on cache miss (per Sidecar §11 recovery pattern)
- TerminateRun RPC (called from SDK on run.end signal)
- DSL evaluator full integration (replaces SLICE_02 pass-through)
- Audit row population for run_projection_at_decision_atomic / run_predicted_remaining_steps / run_steps_completed
- Sidecar wires projector call before reserve stage

---

## §3. Out of scope

| 項目 | 推給 |
|---|---|
| SDK with_run_plan decorator | SLICE_12 |
| Web dashboard run projection surface | Separate frontend slice |
| Multi-replica horizontal scale (sharded by run_id) | Post-launch Phase 2 |

---

## §4. File-level change list

### 4.1 New files

- `services/run_cost_projector/Cargo.toml`, `src/main.rs`, `src/server.rs`, `src/signal_1.rs`, `src/signal_2.rs`, `src/signal_3.rs`, `src/layering.rs`, `src/state_cache.rs`, `src/recovery.rs`
- `proto/spendguard/run_cost_projector/v1/projector.proto`
- `charts/spendguard/templates/run_cost_projector.yaml`

### 4.2 Modified files

- `services/sidecar/src/decision.rs` — wire projector call after output_predictor; before reserve
- `services/sidecar/src/contract/evaluate.rs` — activate handle_run_code real logic (drops pass-through)
- `proto/spendguard/sidecar_adapter/v1/adapter.proto` — DecisionRequest gains planned_steps_hint passthrough field (additive)

---

## §5. Schema / proto changes

per `run-cost-projector-spec-v1alpha1.md` §2.1 (Project / TerminateRun proto). DecisionRequest extension for Signal 3 hint propagation.

---

## §6. Audit-chain impact

- `run_projection_at_decision_atomic`, `run_predicted_remaining_steps`, `run_steps_completed_so_far` columns populated per decision
- RUN_* code emission → `reason_codes` array + `prediction_strategy_used` unchanged
- `STOP_RUN_PROJECTION` enum value used when applicable per `contract-dsl-spec-v1alpha2.md` §6.1

---

## §7. Failure mode coverage

| 場景 | 行為 |
|---|---|
| stats_aggregator cache unreachable | Signal 1 fall to cold-start default (10 steps); reservation safe |
| Projector unreachable from sidecar | conservative pass-through (no RUN_* emitted); reservation = A as default |
| State cache memory full | LRU evict; rebuild from audit chain on next call |
| Process restart | All in-memory states lost; live runs reconstruct from audit chain replay |
| TerminateRun RPC fail | state remains until TTL eviction; no data loss |
| Concurrent Project calls for same run_id | state cache atomicity needed (test required) |

---

## §8. Acceptance criteria

### 8.1 Unit tests

- Signal 1 cold-start default 10 when no aggregator data
- Signal 2 drift detection: 3 consecutive steps with > 2σ ratio shift → emit RUN_DRIFT_DETECTED
- Signal 3 override: when planned_steps_hint > 0, Signal 1 ignored
- Code precedence: BUDGET > STEPS > DRIFT when multiple match

### 8.2 Integration tests

- Stuck-loop simulation: agent calls 47 times in normal flow; with projector emit RUN_BUDGET_PROJECTION_EXCEEDED at step 11
- Drift simulation: gradually increasing per-call cost; detect within 3 steps
- Recovery: kill projector + restart; live run states rebuild from audit chain

### 8.3 Property tests

- For 1000 simulated runs (varied agents): reservation correctness invariant under STRICT_CEILING regardless of RUN_* code firing

### 8.4 Benchmarks

- Project p99 ≤ 5ms (cold cache miss < 10ms)
- 10K concurrent runs: state cache 72h endurance, no memory leak

### 8.5 Demo-mode regression

- `make demo-up DEMO_MODE=m1_benchmark_runaway_loop` shows RUN_BUDGET_PROJECTION_EXCEEDED firing
- All existing demos still green

---

## §9. Slice-specific adversarial review checklist

1. Concurrent `Project` for same run_id: state cache lock or per-run mutex? Show code.
2. Signal 1 cold-start default = 10. Configurable per-tenant?
3. Signal 2 drift detection: N=3 consecutive steps. Configurable? Default rationale.
4. Recovery from audit chain: how does it bound replay window? (Don't replay 30 days of decisions to rebuild RunState.)
5. State cache memory cap: how is "80% allocated memory" measured? OS RSS? Process limit?
6. TerminateRun semantics: idempotent? Multiple terminations for same run_id behavior.
7. Sidecar integration: Project call BEFORE or AFTER reserve? Per Contract §6 stage 4 = reserve; projector should be in stage 4-or-5 boundary. Specify.
8. RUN_BUDGET_PROJECTION_EXCEEDED precision: target ≥ 90%. Benchmark methodology defined?
9. Signal 3 from SDK: how does sidecar know? DecisionRequest.planned_steps_hint field added in this slice. Order of operations.
10. Audit chain populating: when projection unreachable, what value for run_projection_at_decision_atomic? 0 or sentinel? Spec says NO (NOT NULL via fallback path).

---

## §10. Out-of-scope deferrals

| 項目 | 推給 |
|---|---|
| SDK @with_run_plan decorator | SLICE_12 |
| Sharded multi-replica scale | Post-launch |
| Drift threshold per-tenant override | Post-launch via control plane API |

---

## §11. Risk / rollback plan

- Risk: false-positive RUN_BUDGET_PROJECTION_EXCEEDED blocks legitimate runs
- Mitigation: 90% precision target; calibration-report monitors false positive rate
- Rollback: disable projector via Helm; sidecar pass-through (no RUN_* emitted)

---

## §12. AIT execution notes

- Recommended `--agent Backend Architect`
- `--review-budget deep`
- Expected rounds: 3-5 (concurrent state cache + complex signal layering; possible Staff+ panel if precision concerns)

---

## §13. Adoption history (filled during review)

| Round | Reviewer | 採納率 | 主要產出 |
|---|---|---|---|
| (placeholder) | (placeholder) | (placeholder) | (placeholder) |

---

## §14. Merge checklist

- [ ] §8 acceptance green; benchmark p99 ≤ 5ms
- [ ] §9 specific clear
- [ ] universal §1.5 (Signal 1/2/3 layering per spec) verified
- [ ] universal §1.6 (Contract DSL additive) preserved
- [ ] m1_benchmark_runaway_loop demo precision ≥ 90%
- [ ] PR references `run-cost-projector-spec-v1alpha1.md`

---

*Slice version: SLICE_09_run_cost_projector v1alpha1 (draft) | Spec ancestor: run-cost-projector-spec-v1alpha1.md | Depends: SLICE_02, SLICE_06 | Branch: `slice/SLICE_09_run_cost_projector`*
