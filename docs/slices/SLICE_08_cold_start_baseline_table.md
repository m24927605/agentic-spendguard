# Slice 08 — `model_default_distribution.toml` initial table + loader

> **Branch**: `slice/SLICE_08_cold_start_baseline_table`
> **Status**: draft
> **Spec ancestor(s)**: `cold-start-baseline-spec-v1alpha1.md` (primary §3 §4 §7); `output-predictor-service-spec-v1alpha1.md` §7
> **Depends on prior slices**: SLICE_06 (output_predictor + 7-class classifier)
> **Blocks subsequent slices**: SLICE_13 (calibration-report references cold_start_layer)
> **Estimated PR size**: medium (TOML curation + loader module + simulation validation; ~600 LOC + research time)

---

## §0. TL;DR

Hand-curated `model_default_distribution.toml` (70+ entries: 10+ models × 7 prompt classes). `docs/cold-start-baseline-sources.md` documents every source. Loader module activates L2 fallback path in output_predictor. Simulation validates 30-sample threshold yields ≤5% prediction variance.

---

## §1. Architectural context

per `cold-start-baseline-spec-v1alpha1.md` §4 (TOML schema), §7 (source curation flow). Serves Q4 (4-layer cold-start fallback).

---

## §2. Scope (must-do)

- Populate `services/output_predictor/data/model_default_distribution.toml` with 70+ entries per spec §4.2 schema
- For each entry: P50/P95/P99/sample_size/source/source_url/methodology_doc/confidence
- New file `docs/cold-start-baseline-sources.md` per spec §7.4 schema
- Loader module `services/output_predictor/src/cold_start_loader.rs`
- L2 fallback wired in `services/output_predictor/src/strategy_b.rs` (extends SLICE_06's L1-only path)
- Asset bundling (embedded into binary per same pattern as tokenizer assets)
- Simulation validation script under `tests/output_predictor/cold_start_simulation.rs`
- Helper: `agent dispatch general-purpose` or `trend-researcher` for benchmark research (per HANDOFF §7 SLICE 08 note)

---

## §3. Out of scope

| 項目 | 推給 |
|---|---|
| L3 federated aggregate implementation | Post-launch (per spec §5.6 trigger) |
| Quarterly refresh automation | Maintainer process; not slice-bound |
| Per-class override (HIGH_VARIANCE_CLASSES) | Future enhancement |

---

## §4. File-level change list

### 4.1 New files

- `services/output_predictor/data/model_default_distribution.toml` (the 70+ entries)
- `docs/cold-start-baseline-sources.md`
- `services/output_predictor/src/cold_start_loader.rs`
- `tests/output_predictor/cold_start_simulation.rs`

### 4.2 Modified files

- `services/output_predictor/src/strategy_b.rs` — extend cold-start chain to call L2
- `crates/spendguard-cold-start-loader/...` (or extend output_predictor with new submodule)

---

## §5. Schema / proto changes

No proto changes. TOML schema per spec §4.2.

---

## §6. Audit-chain impact

- `cold_start_layer_used = 'L2'` written when L4 + L3 unavailable and L2 entry present
- `cold_start_layer_used = 'L1'` written when L1 hard fallback (no TOML entry)
- `prediction_confidence` derived per `cold-start-baseline-spec-v1alpha1.md` §4.3 entry confidence × sample_size weighting

---

## §7. Failure mode coverage

| 場景 | 行為 |
|---|---|
| TOML asset corrupted / signature invalid | refuse-to-start |
| TOML schema_version unrecognized | refuse-to-start |
| Specific (model, class) entry missing | L2 lookup returns None; fall to L1 |
| TOML hot-reload triggered (rare) | reload with sanity check; revert if invalid |
| Classifier returns unknown class | sentinel + emit metric (rare per classifier rules) |

---

## §8. Acceptance criteria

### 8.1 Unit tests

- Loader correctly parses TOML; sanity checks (no duplicate keys, confidence in [0,1])
- L2 fallback returns correct entry when bucket samples < 30
- L1 fallback returns None when L2 entry missing

### 8.2 Integration tests

- Demo run with no historical data: cold-start L2 hit; audit row populated; verify-chain green

### 8.3 Simulation validation

- Inject 10 / 20 / 30 / 50 / 100 samples → P95 variance ≤ 5% at 30-sample threshold per spec §6.3

### 8.4 Demo-mode regression

- `make demo-up DEMO_MODE=cold_start_partial` (new demo mode for this slice)

---

## §9. Slice-specific adversarial review checklist

1. Source quality bar (sample_size ≥ 500; reproducible methodology; reviewer agreement) per spec §7.3 enforced in TOML curation?
2. Each TOML entry's source URL still active? Citation hygiene check.
3. 7 classes × 10 models = 70 entries; any gaps? List missing combinations.
4. Confidence values: distribution sensible? Not all 0.5; reflects source quality.
5. Class assignment in source curation: code_gen examples from HumanEval+MBPP; chat from MT-Bench; etc. Verify in `cold-start-baseline-sources.md`.
6. Loader sanity check on boot: O(N) over 70 entries; negligible startup overhead.
7. Asset bundling: total size?
8. Refresh playbook: documented per spec §7.2?
9. Simulation validate 30-sample threshold: shipped with PR?
10. L2 sentinel for unknown class: how does loader handle? Tested.

---

## §10. Out-of-scope deferrals

| 項目 | 推給 |
|---|---|
| L3 federated build | Post-launch (per spec §5.6) |
| Per-class threshold override | Future enhancement |
| Quarterly refresh CI automation | Post-launch |

---

## §11. Risk / rollback plan

- Risk: source data inaccuracy systematically biases L2 predictions
- Mitigation: confidence values force caution; calibration-report identifies systematic L2 drift
- Rollback: drop affected entries; revert to L1 for those buckets

---

## §12. AIT execution notes

- Recommended `--agent Trend Researcher` (per HANDOFF §10.1) for TOML population
- `--review-budget standard` (research-heavy; not high-risk code)
- Expected rounds: 2-3 (research / curation focused)

---

## §13. Adoption history (filled during review)

| Round | Reviewer | 採納率 | 主要產出 |
|---|---|---|---|
| (placeholder) | (placeholder) | (placeholder) | (placeholder) |

---

## §14. Merge checklist

- [ ] 70+ entries with sources
- [ ] `docs/cold-start-baseline-sources.md` complete
- [ ] Simulation validation green
- [ ] Loader tested
- [ ] universal §1.4 (cold-start fallback exhaustive) green
- [ ] PR references `cold-start-baseline-spec-v1alpha1.md`

---

*Slice version: SLICE_08_cold_start_baseline_table v1alpha1 (draft) | Spec ancestor: cold-start-baseline-spec-v1alpha1.md §4 §7 | Depends: SLICE_06 | Branch: `slice/SLICE_08_cold_start_baseline_table`*
