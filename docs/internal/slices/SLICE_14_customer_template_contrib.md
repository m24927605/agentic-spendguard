# Slice 14 — `contrib/output_predictor_template/`

> **Branch**: `slice/SLICE_14_customer_template_contrib`
> **Status**: draft
> **Spec ancestor(s)**: `output-predictor-plugin-contract-v1alpha1.md` §10
> **Depends on prior slices**: SLICE_07 (plugin contract + delegated mode)
> **Blocks subsequent slices**: none
> **Estimated PR size**: small-medium (Python reference + Dockerfile + tests; ~600 LOC + docs)

---

## §0. TL;DR

Reference Python implementation of a Strategy C plugin server under `contrib/output_predictor_template/`. Customer template only — SpendGuard does not operate it. Includes gRPC server skeleton, feature extractor, backtest harness, Dockerfile, mTLS setup walkthrough. Conformance test corpus included.

---

## §1. Architectural context

per `output-predictor-plugin-contract-v1alpha1.md` §10. Reference implementation for customers who choose to build Strategy C.

---

## §2. Scope (must-do)

- `contrib/output_predictor_template/predictor_server.py` — gRPC server skeleton per `output-predictor-plugin-contract-v1alpha1.md` proto §2
- `contrib/output_predictor_template/feature_extractor.py` — convert PredictRequest features to model input vector
- `contrib/output_predictor_template/model_predictor_stub.py` — sklearn-style stub returning constant; customer replaces
- `contrib/output_predictor_template/backtest_harness.py` — offline validation against historical actual_output_tokens data
- `contrib/output_predictor_template/Dockerfile` — containerized run env
- `contrib/output_predictor_template/mtls_setup.md` — cert setup walkthrough
- `contrib/output_predictor_template/conformance_test.py` — exercises plugin against SpendGuard's conformance corpus
- `contrib/output_predictor_template/README.md` — overview + quickstart

---

## §3. Out of scope

| 項目 | 推給 |
|---|---|
| Production ML model | Customer's responsibility (per Q1 reasoning) |
| Multi-language (Go / Rust / TS) ports | Post-launch |
| Customer onboarding playbook | Documentation team / post-launch |

---

## §4. File-level change list

### 4.1 New files (all under `contrib/output_predictor_template/`)

- `predictor_server.py`, `feature_extractor.py`, `model_predictor_stub.py`, `backtest_harness.py`, `Dockerfile`, `mtls_setup.md`, `conformance_test.py`, `README.md`, `pyproject.toml`, `requirements.txt`

### 4.2 Modified files

- `proto/spendguard/output_predictor_plugin/v1/plugin.proto` — codegen to Python via `python -m grpc_tools.protoc` (script in `contrib/output_predictor_template/`)
- `ci/` — add conformance test runner for the template in CI sandbox

---

## §5. Schema / proto changes

Uses existing `plugin.proto` from SLICE_07. Python bindings generated.

---

## §6. Audit-chain impact

None (template is customer-side; SpendGuard's audit chain not directly affected by template implementation).

---

## §7. Failure mode coverage

| 場景 | 行為 |
|---|---|
| Plugin server crash | SpendGuard side handles (per SLICE_07 circuit breaker) |
| Plugin returns invalid response | SpendGuard side handles (per SLICE_07 §5.1 validation) |
| Template Dockerfile fails to build | CI catches |
| Conformance corpus mismatch | Template version skew; customer fix |

---

## §8. Acceptance criteria

### 8.1 Unit tests

- Template predictor_server runs locally
- Conformance test 100% pass on stub model
- Backtest harness produces validation report

### 8.2 Integration tests

- Docker sandbox: build + run + Predict via SpendGuard mock client; round-trip success
- mTLS setup: walkthrough produces working cert chain that SpendGuard accepts

### 8.3 Demo

- `make demo-up DEMO_MODE=plugin_c_synthetic` (introduced in SLICE_07): now uses real template (not just mock)

---

## §9. Slice-specific adversarial review checklist

1. Template `model_predictor_stub.py` returns valid response (not always 1000 fixed)?
2. Dockerfile reproducible?  Base image pinned?
3. `mtls_setup.md` walkthrough actually produces working setup?
4. Conformance corpus size and coverage?
5. License clear (Apache 2.0 expected; consistent with SpendGuard main)?
6. README quickstart actually quick? Tested by someone unfamiliar.
7. Backtest harness uses real audit data structure (not toy)?
8. Customer instructions for replacing the stub model are explicit?

---

## §10. Out-of-scope deferrals

| 項目 | 推給 |
|---|---|
| Multi-language templates | Post-launch |
| Sample real ML models for diff use cases (chat / code / etc.) | Future enhancement |

---

## §11. Risk / rollback plan

- Risk: template confuses customer (false sense it's production-ready)
- Mitigation: README explicit "STUB MODEL; YOU MUST REPLACE"; backtest harness emphasizes calibration
- Rollback: revert PR; customers stick with mock-only or build from scratch

---

## §12. Review Execution Notes

- Recommended reviewer profile: Backend Architect + `Technical Writer` (template docs)
- Review depth: standard
- Expected rounds: 2

---

## §13. Adoption history (filled during review)

| Round | Reviewer | 採納率 | 主要產出 |
|---|---|---|---|
| (placeholder) | (placeholder) | (placeholder) | (placeholder) |

---

## §14. Merge checklist

- [ ] §8 acceptance green
- [ ] §9 specific clear
- [ ] Docker sandbox build + run in CI green
- [ ] Conformance corpus included
- [ ] PR references `output-predictor-plugin-contract-v1alpha1.md` §10

---

*Slice version: SLICE_14_customer_template_contrib v1alpha1 (draft) | Spec ancestor: output-predictor-plugin-contract-v1alpha1.md §10 | Depends: SLICE_07 | Branch: `slice/SLICE_14_customer_template_contrib`*
