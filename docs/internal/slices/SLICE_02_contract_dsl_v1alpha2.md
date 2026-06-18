# Slice 02 — Contract DSL v1alpha2 additive changes

> **Branch**: `slice/SLICE_02_contract_dsl_v1alpha2`
> **Status**: draft
> **Spec ancestor(s)**: `contract-dsl-spec-v1alpha2.md` (primary), `predictor-architecture-spec-v1alpha1.md` (umbrella §5 policy matrix)
> **Depends on prior slices**: SLICE_01（needs audit columns for new code persistence）
> **Blocks subsequent slices**: SLICE_09 (run_cost_projector uses RUN_* codes), SLICE_10 (egress proxy updates DecisionResponse handling)
> **Estimated PR size**: small (proto bump + DSL evaluator extension + tests; ~400 LOC)

---

## §0. TL;DR

Bump Contract DSL `apiVersion` from `spendguard.ai/v1alpha1` to `spendguard.ai/v1alpha2`; add 3 RUN_* decision codes, `prediction_policy` enum, `run_projection_action` enum + allowed-pairs validation; new `DecisionResponse.Decision::STOP_RUN_PROJECTION` enum value; new field `run_code_triggered`; DSL evaluator pass-through implementation (logic in SLICE_09). Strictly additive; v1alpha1 contracts byte-identically evaluable.

---

## §1. Architectural context

per `contract-dsl-spec-v1alpha2.md` §1.1: additive-only patch over v1alpha1; per `predictor-architecture-spec-v1alpha1.md` §5 policy matrix this slice activates the `prediction_policy` enum that all downstream specs reference. Serves Q3 (per-run projection codes) and Q1 (no-ML default = STRICT_CEILING).

---

## §2. Scope (must-do)

- Bump `proto/spendguard/sidecar_adapter/v1/adapter.proto` `DecisionResponse.Decision` enum with `STOP_RUN_PROJECTION = 6`
- Add `DecisionResponse.run_code_triggered = 16` field
- Add YAML schema fields `prediction_policy` + `run_projection_action` to contract bundle loader
- Add CEL helpers `run_projection.*` and `prediction.*` per spec §6.3
- Add `prediction_policy × run_projection_action` allowed-pairs validation at bundle load time
- DSL evaluator pass-through for the 3 RUN_* codes (no triggering logic; that's SLICE_09)
- v1alpha1 → v1alpha2 evaluator default fill behavior (per spec §6.4)
- Apply audit column `prediction_policy_used` write logic in DSL evaluator side

---

## §3. Out of scope

| 項目 | 為何 | 推給 |
|---|---|---|
| RUN_* triggering logic | Requires run_cost_projector | SLICE_09 |
| Actual run_projection_action execution | Requires Signal 1/2/3 | SLICE_09 |
| Migration tooling for v1alpha1 → v1alpha2 contracts | Customer-facing tool | Post-launch |

---

## §4. File-level change list

### 4.1 New / modified files

- `proto/spendguard/sidecar_adapter/v1/adapter.proto` — add enum value + field
- `services/sidecar/src/contract/evaluate.rs` — extend evaluator for new enum dispatch
- `services/sidecar/src/contract/bundle.rs` — recognize `prediction_policy` + `run_projection_action` YAML; validate pairs
- `services/sidecar/src/contract/cel_helpers.rs` — new CEL functions
- `tests/sidecar/contract/v1alpha2_regression_test.rs` — byte-identical regression vs v1alpha1
- `examples/contracts/quickstart-v1alpha2.yaml` — sample with new policy

### 4.2 Helm / config

- No new env vars in SLICE_02. Per spec §4.2 prediction_policy is set
  by the contract bundle (declarative source-of-truth) and runtime
  override is FORBIDDEN — operators must redeploy a new bundle to
  change policy. The originally-planned
  `SPENDGUARD_DEFAULT_PREDICTION_POLICY` env var was removed in
  round-1 fix M2 because it had no consumer in sidecar code and the
  spec invariant precludes ever wiring one (an operator-side runtime
  override would bypass the bundle redeploy + signature flow that
  §4.2 mandates). YAGNI prevails; the Helm chart surface stays
  honest about what the sidecar actually consumes.

---

## §5. Schema / proto changes

per `contract-dsl-spec-v1alpha2.md` §6.1 (enum addition), §6.2 (field addition), §6.3 (YAML schema additions). Full diffs in those sections.

---

## §6. Audit-chain impact

- `prediction_policy_used` column populated per decision (SLICE_01 already added schema)
- For RUN_* code emission events: `cloudevent_payload.reason_codes` array contains the code string
- No new audit columns beyond SLICE_01

---

## §7. Failure mode coverage

| 依賴 | 失敗情境 | 預期行為 |
|---|---|---|
| v1alpha2 contract bundle loaded on v1alpha1 sidecar | apiVersion mismatch | refuse_to_load + bundle_validation_failed event (per `sidecar-architecture-spec-v1alpha1.md` §3.3) |
| v1alpha1 contract bundle loaded on v1alpha2 sidecar | normal additive evolution | default fill `prediction_policy=STRICT_CEILING` + `run_projection_action=BLOCK_NEXT_CALL` (per spec §6.4) |
| `STRICT_CEILING + ALERT_ONLY` pair in v1alpha2 contract | invalid combo per §5.3 | bundle_validation_failed + refuse_to_load |
| Proto codegen breaks downstream | binary incompat | refuse-to-deploy |
| CEL helper invoked but RUN_* code not yet triggered (SLICE_09 pending) | normal | evaluation returns default; runtime continues |

---

## §8. Acceptance criteria

### 8.1 Unit tests

- Bundle loader accepts v1alpha2 schema with all 4 policies + 3 actions × allowed pairs
- Bundle loader rejects `STRICT_CEILING + ALERT_ONLY` and `SHADOW_ONLY + BLOCK_NEXT_CALL` etc.
- CEL helpers evaluate correctly against synthetic predictor metadata
- DSL evaluator dispatches all 3 RUN_* codes to `handle_run_code()` (pass-through)

### 8.2 Integration tests

- v1alpha2 sidecar + v1alpha1 contract: byte-identical audit row output for 100 decisions vs v1alpha1 sidecar baseline
- v1alpha2 sidecar + v1alpha2 contract: new policies + audit `prediction_policy_used` column populated correctly

### 8.3 Property tests

- For all 4 × 3 policy×action combinations: matches §5.3 allowed-pairs table
- For 100 v1alpha1 contracts: v1alpha2 evaluator produces same decision_id, reason_codes, mutation_patch_json

### 8.4 Audit invariant tests

- `verify-chain` regression on v1alpha2 audit row stream

### 8.5 Demo-mode regression

All 8+ demo modes still pass under v1alpha2 evaluator with default policy fill.

---

## §9. Slice-specific adversarial review checklist

1. Are the 4 enum values for `prediction_policy` documented in a single source of truth (`contract-dsl-spec-v1alpha2.md` §4)? Confirm no drift to `predictor-architecture-spec-v1alpha1.md` §5.
2. STOP_RUN_PROJECTION (tag 6): does adding this break old DecisionResponse consumers? Test against v1alpha1 client deserializing v1alpha2 response.
3. `run_code_triggered` field: empty string vs absent semantics? Per `proto3 string default = ""`, ensure CLI / dashboard handle both.
4. Allowed-pairs validation: tested at bundle load AND tested at runtime fallback for malformed contracts?
5. v1alpha1 contract default fill: where in code does this happen? File path required.
6. CEL helpers documentation: where do customers see them? Updated `docs/contract-dsl-spec-v1alpha2.md` §6.3 table?
7. Rolling upgrade order (per spec §8.3): is this enforced in deployment scripts or relied on operator?
8. Conformance test corpus for v1alpha1 → v1alpha2 byte-identical: location?

---

## §10. Out-of-scope deferrals

| 項目 | 理由 | 推給 |
|---|---|---|
| Customer migration tooling | Not blocking | Post-launch |
| Web dashboard new decision filter | Frontend slice | Separate dashboard slice |
| Schema bundle id rotation script | Sidecar-managed | SLICE_03 |

---

## §11. Risk / rollback plan

- Worst case: v1alpha2 evaluator unintentionally changes v1alpha1 contract behavior
- Mitigation: byte-identical regression test in CI; 100 baseline contracts compared output exactly
- Rollback: revert sidecar to v1alpha1 binary; v1alpha2 contracts auto-refused (per §3.3)

---

## §12. Review Execution Notes

- Recommended reviewer profile: Backend Architect or `Software Architect`
- Review depth: deep
- Expected rounds: 2-3
- Risk factor: if byte-identical regression fails on any of 8+ demo modes → blocker

---

## §13. Adoption history (filled during review)

| Round | Reviewer | 採納率 | 主要產出 |
|---|---|---|---|
| (placeholder) | (placeholder) | (placeholder) | (placeholder) |

---

## §14. Merge checklist

- [ ] All §8 acceptance green
- [ ] §9 slice-specific full clear
- [ ] universal `predictor-review-checklist.md` §1.6 (Contract DSL additive) green
- [ ] v1alpha1 byte-identical regression green
- [ ] Demo modes green
- [ ] PR description references `contract-dsl-spec-v1alpha2.md`

---

*Slice version: SLICE_02_contract_dsl_v1alpha2 v1alpha1 (draft) | Spec ancestor: contract-dsl-spec-v1alpha2.md | Depends: SLICE_01 | Branch: `slice/SLICE_02_contract_dsl_v1alpha2`*
