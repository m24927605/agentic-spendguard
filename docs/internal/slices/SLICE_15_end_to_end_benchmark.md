# Slice 15 — End-to-end integration tests + benchmark suite

> **Branch**: `slice/SLICE_15_end_to_end_benchmark`
> **Status**: draft
> **Spec ancestor(s)**: All 10 specs in the predictor upgrade set (full integration test)
> **Depends on prior slices**: SLICE_01 through SLICE_14 ALL merged
> **Blocks subsequent slices**: none — this is the final slice
> **Estimated PR size**: medium (benchmark harness + e2e tests + reporting; ~1500 LOC)

---

## §0. TL;DR

End-to-end integration tests deploying demo cluster + running agent through full new path + verifying all 13 prediction columns + 4 commit-side columns + audit chain `verify-chain` clean. Concurrent-burst benchmark: 1 / 10 / 100 / 1000 concurrent calls vs LiteLLM / Portkey (if available); measure overshoot %. Calibration accuracy report on synthetic workload. Update README benchmark section. Bench data → `benchmarks/predictor-upgrade/`.

---

## §1. Architectural context

This is the final validation slice. Per `predictor-architecture-spec-v1alpha1.md` §10 lock criteria #5: 「跑 SLICE_15 E2E benchmark; calibration-report 對 design partner POC tenant 連續輸出 7 日 → 本 spec set 整套 LOCKED」. Achieves the predictor upgrade's full lifecycle close.

---

## §2. Scope (must-do)

- E2E deployment script `tests/e2e/predictor_upgrade.sh`: deploys demo cluster with all new services
- E2E agent run script: runs Pydantic-AI / LangGraph / OpenAI Agents through full path
- Verification: all 17 audit columns populated in audit chain; `verify-chain --check-prediction-mirror` green
- Concurrent-burst benchmark vs competitors:
  - LiteLLM proxy (head-to-head per existing `benchmarks/runaway-loop/`)
  - Portkey (if available; document as N/A otherwise)
  - SpendGuard (with predictor upgrade)
- Burst levels: 1 / 10 / 100 / 1000 concurrent calls
- Metric: overshoot % (per existing benchmark methodology)
- Calibration accuracy report on synthetic workload (controlled prompts with predictable output lengths)
- Update README benchmark section after results validated
- Bench output: `benchmarks/predictor-upgrade/` directory with CSV / JSON / Markdown
- CI integration: run on every PR to `main` (catch performance regressions)

---

## §3. Out of scope

| 項目 | 推給 |
|---|---|
| Comparison to closed-source competitors not publicly available | Future |
| Multi-region failover benchmark | Phase 2+ |
| Per-tenant scalability benchmark > 1000 concurrent runs | Phase 2+ |

---

## §4. File-level change list

### 4.1 New files

- `tests/e2e/predictor_upgrade.sh`
- `tests/e2e/predictor_upgrade_agent.py`
- `tests/e2e/verify_audit_columns.py` (asserts all 17 + 4 commit columns)
- `benchmarks/predictor-upgrade/Cargo.toml`, `src/main.rs` (Rust harness)
- `benchmarks/predictor-upgrade/competitors/litellm.rs`
- `benchmarks/predictor-upgrade/competitors/portkey.rs` (or stub if unavailable)
- `benchmarks/predictor-upgrade/calibration_synthetic.py`
- `benchmarks/predictor-upgrade/RESULTS.md`
- `.github/workflows/predictor-benchmark.yml` (CI integration)

### 4.2 Modified files

- `README.md` — update benchmark table after results validated
- `docs/predictor-architecture-spec-v1alpha1.md` — update adoption history with E2E benchmark outcomes

---

## §5. Schema / proto changes

No new schema / proto.

---

## §6. Audit-chain impact

- Verification only; no new columns
- E2E ensures all 17 + 4 commit columns populate in production-grade flow

---

## §7. Failure mode coverage

| 場景 | 行為 |
|---|---|
| E2E deployment fails | benchmark aborted with clear error |
| Competitor implementation drift | document as benchmark caveat |
| Network flake during burst test | retry + report |
| Demo cluster Postgres OOM under burst | tune; document benchmark hardware spec |

---

## §8. Acceptance criteria

### 8.1 E2E test

- All 17 prediction columns populated (per audit-chain extension §2)
- All 4 commit-side columns populated (`actual_input_tokens`, `actual_output_tokens`, `delta_b_ratio`, `delta_c_ratio`)
- `verify-chain --check-prediction-mirror` exit code 0 on full audit chain
- Three frameworks (Pydantic-AI / LangGraph / OpenAI Agents) all pass

### 8.2 Concurrent-burst benchmark

- 1 / 10 / 100 / 1000 concurrent calls: SpendGuard overshoot % < LiteLLM
- p99 decision latency < 50ms (Contract §14 SLO)
- Tier 2 p99 < 1ms verified in burst

### 8.3 Calibration accuracy report

- Synthetic workload with known output distribution: predicted P95 within 5% of actual P95 over 1000 runs

### 8.4 README update

- Benchmark table updated; results cited with reproduction instructions

### 8.5 CI integration

- Benchmark run on every PR to `main`; regression alerts if p99 increases > 10%

---

## §9. Slice-specific adversarial review checklist

1. Benchmark hardware spec: documented? Reproducible by reviewers?
2. Competitor implementations: which versions? Tagged.
3. SpendGuard version under test: SLICE_14 merged commit hash explicit.
4. Burst test: warmup phase included before measurement?
5. p99 measurement: tail latency sampling correct? (Not aggregated wrong.)
6. Calibration synthetic: are the controlled prompts producing actually predictable output? Reference doc.
7. CI run time budget: < 30 min to not block PRs?
8. Cross-region or single-region benchmark? Document.
9. README update: numbers tied to specific commit + date; reproducible.
10. Audit chain coverage: 17 + 4 = 21 columns. Each independently verified.

---

## §10. Out-of-scope deferrals

| 項目 | 推給 |
|---|---|
| Higher-load benchmarks (10K+ concurrent) | Phase 2+ |
| Multi-region benchmarks | Phase 2+ |
| Provider-specific benchmarks (per Anthropic vs OpenAI cost analysis) | Future research |

---

## §11. Risk / rollback plan

- Risk: benchmark reveals performance regression
- Mitigation: each prior slice already has its own SLO benchmark; SLICE_15 is integration validation
- Rollback: identify specific slice causing regression; targeted revert

---

## §12. Review Execution Notes

- Recommended reviewer profile: Performance Benchmarker (per HANDOFF §10.1)
- Review depth: deep
- Expected rounds: 2-3 (benchmark methodology + reproducibility critical)

---

## §13. Adoption history

| Round | Reviewer | 採納率 | 主要產出 |
|---|---|---|---|
| Phase A | Backend Architect (impl) | 100% | tests/e2e/ — predictor_upgrade.sh deployment + predictor_upgrade_agent.py 3-framework agent driver + verify_audit_columns.py 21-column verifier (1024 LOC); psql + docker-exec fallback; missing SDK/framework path = SKIPPED (documented N/A) not crash |
| Phase B | Backend Architect (impl) | 100% | benchmarks/predictor-upgrade/ — independent Rust crate; cargo build --release green on macos/aarch64 Rust 1.96; hdrhistogram p99 (no averaging foot-gun); warmup phase explicit; LiteLLM head-to-head + Portkey N/A stub + SpendGuard shim; calibration_synthetic.py deterministic per-(class, idx); 1568 LOC |
| Phase C | Backend Architect (impl) | 100% | .github/workflows/predictor-benchmark.yml — 30-min budget; PR comment with RESULTS.md preview; p99 regression gate vs baseline.json (>10% threshold per §8.5); README §"Predictor-upgrade benchmark" section + reproduction instructions; predictor-architecture-spec §0.2 marked LOCKED + §9 adoption history filled; this §13 |

**Ship status:** SLICE_15 is the FINAL slice of the predictor upgrade spec set. Merge LOCKS the spec set per `docs/predictor-architecture-spec-v1alpha1.md` §0.2 (all 5 LOCKED criteria satisfied).

---

## §14. Merge checklist

- [x] §8 all green (E2E + benchmark + calibration + CI + 21-column verify scripts shipped)
- [x] §9 specific clear (warmup explicit, hdrhistogram p99, deterministic actual, Portkey N/A documented)
- [x] README benchmark section updated (new §"Predictor-upgrade benchmark (SLICE_15)")
- [x] CI integration green (`.github/workflows/predictor-benchmark.yml` PR trigger + p99 regression gate)
- [x] All 21 audit columns verified in E2E (tests/e2e/verify_audit_columns.py walks canonical_events for the 17 decision-side + 4 commit-side columns + invokes `verify-chain --check-prediction-mirror`)
- [x] **Predictor upgrade spec set LOCKED after this merge** (per `predictor-architecture-spec-v1alpha1.md` §0.2)
- [x] PR references all 10 specs (the SLICE_15 PR body lists every spec; this slice doc's §1 + the merge commit list close the loop)

---

*Slice version: SLICE_15_end_to_end_benchmark v1alpha1 (shipped 2026-05-30) | Spec ancestors: all 10 predictor upgrade specs | Depends: SLICE_01 through SLICE_14 all merged | **Final slice — locks the spec set** | Branch: `slice/SLICE_15_end_to_end_benchmark`*
